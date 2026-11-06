//! The relay: the one kind of plain HTTP request the proxy serves. An agent whose backend
//! relays sends its model requests to [`Relay::URL`] and holds no key. The relay forwards each
//! request from the host to the backend's endpoint, over TLS for an `https://` one, with the
//! key, tries again while the endpoint refuses it before sending any body, and then streams
//! the answer back unbuffered.

use std::{
    error::Error,
    io,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use chrono::Utc;
use eyre::{Result, WrapErr, ensure, eyre};
use rand::{RngExt, SeedableRng};
use rand_pcg::Pcg64;
use reqwest::{
    Method, Url,
    header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER},
    redirect,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::mpsc,
    time::timeout,
};

use super::{
    Record, Verdict,
    allowlist::Host,
    request::{self, Forward},
    retry::{self, Backoff},
    tunnel::BAD_GATEWAY,
};
use crate::secret::Secret;

/// The host the relay answers to. `.invalid` never resolves, so no real host is shadowed.
const HOST: &str = "relay.invalid";
const PORT: u16 = 80;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the agent may take to send a request body once its head has arrived.
const BODY_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest request body the relay holds for retrying.
const MAX_BODY: usize = 64 << 20;

const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const LENGTH_REQUIRED: &[u8] =
    b"HTTP/1.1 411 Length Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const TOO_LARGE: &[u8] =
    b"HTTP/1.1 413 Content Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const REQUEST_TIMEOUT: &[u8] =
    b"HTTP/1.1 408 Request Timeout\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Fields that describe one connection rather than the message, and so never cross the relay
/// in either direction, besides those a `connection` field names. Each side's length and host
/// are framed anew.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "content-length",
    "expect",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// What the relay did with one try of an agent's request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relayed {
    /// The agent's request, numbered from 1 in the order the attempt's relay received them.
    pub request: u32,
    /// The try of that request, numbered from 1.
    pub r#try: u32,
    pub method: String,
    /// The path and query the agent asked for, which follow the endpoint's own path.
    pub path: String,
    /// The endpoint's status, absent when no response arrived.
    pub status: Option<u16>,
    /// Why no response arrived, or why its body broke off.
    pub error: Option<String>,
    /// When the relay tried again, for a try it did not pass to the agent.
    pub retry_at: Option<chrono::DateTime<Utc>>,
}

/// Forwards one attempt's model requests to its backend's endpoint with the backend's key.
pub struct Relay {
    endpoint: String,
    host: String,
    port: u16,
    authorization: HeaderValue,
    client: reqwest::Client,
    backoff: Backoff,
    requests: AtomicU32,
    jitter: Mutex<Pcg64>,
}

impl Relay {
    /// The base URL an agent sends its model requests to, through its egress proxy.
    pub const URL: &str = "http://relay.invalid";

    /// A relay to `endpoint`, the backend's API base URL, that sends `key` as a bearer token.
    pub fn new(endpoint: &str, key: &Secret<String>) -> Result<Self> {
        let url = Url::parse(endpoint).wrap_err_with(|| format!("relay endpoint {endpoint:?}"))?;
        ensure!(
            matches!(url.scheme(), "https" | "http") && url.query().is_none(),
            "relay endpoint {endpoint:?} is not an HTTP base URL"
        );
        let host = url
            .host_str()
            .ok_or_else(|| eyre!("relay endpoint {endpoint:?} names no host"))?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| eyre!("relay endpoint {endpoint:?} names no port"))?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", key.expose()))
            .map_err(|_| eyre!("the key for {endpoint} is not a valid header value"))?;
        authorization.set_sensitive(true);
        let client = client_builder()
            .build()
            .wrap_err("building the relay's HTTP client")?;
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64)
            ^ u64::from(std::process::id());
        Ok(Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            host,
            port,
            authorization,
            client,
            backoff: Backoff::DEFAULT,
            requests: AtomicU32::new(0),
            jitter: Mutex::new(Pcg64::seed_from_u64(seed)),
        })
    }

    #[cfg(test)]
    pub(super) fn with_backoff(self, backoff: Backoff) -> Self {
        Self { backoff, ..self }
    }

    /// Also trusts `root`, so that a test can reach an endpoint on loopback over TLS.
    #[cfg(test)]
    pub(super) fn trusting(self, root: reqwest::Certificate) -> Self {
        let client = client_builder().add_root_certificate(root).build().unwrap();
        Self { client, ..self }
    }

    /// Whether a request in absolute form is addressed to the relay.
    pub(super) fn serves(&self, host: &Host, port: u16, forward: &Forward) -> bool {
        forward.plain && port == PORT && host.as_str() == HOST
    }

    /// Answers the request whose head the client sent, with `early` the bytes that followed
    /// it. Each try is logged as its own record: every one but the last through `records`, and
    /// the last left in `record` for the caller, so that a connection cut short still logs it.
    pub(super) async fn serve(
        &self,
        client: &mut UnixStream,
        forward: &Forward,
        head: &[u8],
        early: Vec<u8>,
        record: &mut Record,
        records: &mpsc::UnboundedSender<Record>,
    ) -> io::Result<()> {
        let (Some(fields), Ok(method)) = (
            request::fields(head),
            Method::from_bytes(forward.method.as_bytes()),
        ) else {
            return client.write_all(BAD_REQUEST).await;
        };
        let Some(headers) = upstream_headers(&fields, &self.authorization) else {
            return client.write_all(BAD_REQUEST).await;
        };
        let Ok(url) = Url::parse(&format!("{}{}", self.endpoint, forward.path)) else {
            return client.write_all(BAD_REQUEST).await;
        };
        let body = match read_body(client, &fields, early).await? {
            Body::Read(body) => body,
            Body::Refused(refusal) => return client.write_all(refusal).await,
        };
        let number = self.requests.fetch_add(1, Ordering::Relaxed) + 1;
        let (mut reader, mut writer) = client.split();
        let hung_up = hung_up(&mut reader);
        tokio::pin!(hung_up);

        let first = Instant::now();
        let mut tries = 0;
        let answer = loop {
            tries += 1;
            let next = self.record(number, tries, forward, body.len());
            let ended = std::mem::replace(record, next);
            if tries > 1 {
                // The try before ended when its wait began; the writer only stops on an
                // error, which closing the proxy reports.
                let _ = records.send(ended);
            }
            let sent = self
                .client
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .body(body.clone())
                .send();
            let sent = tokio::select! {
                sent = sent => sent,
                () = &mut hung_up => return Ok(()),
            };
            let (answer, retry_after) = match sent {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if let Some(relayed) = &mut record.relay {
                        relayed.status = Some(status);
                    }
                    if !retry::retries(status) {
                        break Some(response);
                    }
                    let retry_after = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| retry::retry_after(value.as_bytes(), Utc::now()));
                    (Some(response), retry_after)
                }
                Err(err) => {
                    if let Some(relayed) = &mut record.relay {
                        relayed.error = Some(chain(&err));
                    }
                    (None, None)
                }
            };
            let jitter = self.jitter();
            let Some(wait) = self
                .backoff
                .wait(tries, retry_after, first.elapsed(), jitter)
            else {
                break answer;
            };
            drop(answer);
            let now = Utc::now();
            record.end = now;
            if let Some(relayed) = &mut record.relay {
                relayed.retry_at = chrono::TimeDelta::from_std(wait)
                    .ok()
                    .and_then(|wait| now.checked_add_signed(wait));
            }
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = &mut hung_up => return Ok(()),
            }
        };
        let Some(mut response) = answer else {
            return writer.write_all(BAD_GATEWAY).await;
        };
        let bodyless = method == Method::HEAD
            || response.status().is_informational()
            || matches!(response.status().as_u16(), 204 | 304);
        writer
            .write_all(&response_head(&response, !bodyless))
            .await?;
        if bodyless {
            return writer.shutdown().await;
        }
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk,
                () = &mut hung_up => return Ok(()),
            };
            match chunk {
                Ok(Some(chunk)) => {
                    let mut frame = format!("{:x}\r\n", chunk.len()).into_bytes();
                    frame.extend_from_slice(&chunk);
                    frame.extend_from_slice(b"\r\n");
                    writer.write_all(&frame).await?;
                    record.bytes_down += chunk.len() as u64;
                }
                Ok(None) => {
                    writer.write_all(b"0\r\n\r\n").await?;
                    return writer.shutdown().await;
                }
                // Ending without the last chunk tells the agent the body broke off.
                Err(err) => {
                    if let Some(relayed) = &mut record.relay {
                        relayed.error = Some(chain(&err));
                    }
                    return Ok(());
                }
            }
        }
    }

    /// The record of try `tries` of request `number`, starting now.
    fn record(&self, number: u32, tries: u32, forward: &Forward, body: usize) -> Record {
        let start = Utc::now();
        Record {
            host: Some(self.host.clone()),
            port: Some(self.port),
            verdict: Verdict::Relayed,
            bytes_up: body as u64,
            bytes_down: 0,
            start,
            end: start,
            relay: Some(Relayed {
                request: number,
                r#try: tries,
                method: forward.method.clone(),
                path: forward.path.clone(),
                status: None,
                error: None,
                retry_at: None,
            }),
        }
    }

    fn jitter(&self) -> f64 {
        let mut rng = self.jitter.lock().unwrap_or_else(PoisonError::into_inner);
        rng.random_range(0.0..1.0)
    }
}

/// How every relay's client connects: never through a proxy, and never following a redirect.
fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(redirect::Policy::none())
        .no_proxy()
}

/// A request body, or the answer that refuses the request instead.
enum Body {
    Read(Vec<u8>),
    Refused(&'static [u8]),
}

/// Reads the body that `fields` frame by its length, after the `early` bytes the head
/// arrived with. A body without a length, such as a chunked one, is refused.
async fn read_body(
    client: &mut UnixStream,
    fields: &[(String, Vec<u8>)],
    mut early: Vec<u8>,
) -> io::Result<Body> {
    if fields.iter().any(|(name, _)| name == "transfer-encoding") {
        return Ok(Body::Refused(LENGTH_REQUIRED));
    }
    let lengths: Vec<&[u8]> = fields
        .iter()
        .filter(|(name, _)| name == "content-length")
        .map(|(_, value)| value.as_slice())
        .collect();
    let length = match lengths[..] {
        [] => 0,
        [value] if !value.is_empty() && value.iter().all(u8::is_ascii_digit) => {
            match std::str::from_utf8(value).ok().and_then(|v| v.parse().ok()) {
                Some(length) => length,
                None => return Ok(Body::Refused(TOO_LARGE)),
            }
        }
        _ => return Ok(Body::Refused(BAD_REQUEST)),
    };
    if length > MAX_BODY {
        return Ok(Body::Refused(TOO_LARGE));
    }
    early.truncate(length);
    let mut rest = vec![0; length - early.len()];
    match timeout(BODY_TIMEOUT, client.read_exact(&mut rest)).await {
        Ok(Ok(_)) => {}
        Ok(Err(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
            return Ok(Body::Refused(BAD_REQUEST));
        }
        Ok(Err(err)) => return Err(err),
        Err(_) => return Ok(Body::Refused(REQUEST_TIMEOUT)),
    }
    early.extend_from_slice(&rest);
    Ok(Body::Read(early))
}

/// The agent's header fields as the relay forwards them: without the connection's own
/// fields and the agent's authorization, with the key's instead. `None` when a field is no
/// valid header.
fn upstream_headers(
    fields: &[(String, Vec<u8>)],
    authorization: &HeaderValue,
) -> Option<HeaderMap> {
    let named = connection_names(
        fields
            .iter()
            .map(|(name, value)| (name.as_str(), &value[..])),
    );
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        if name == "authorization" || is_hop_by_hop(name, &named) {
            continue;
        }
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).ok()?,
            HeaderValue::from_bytes(value).ok()?,
        );
    }
    headers.insert(AUTHORIZATION, authorization.clone());
    Some(headers)
}

/// The status line and fields of `response` as the agent gets them: framed in chunks when it
/// has a body, and closing the connection after it.
fn response_head(response: &reqwest::Response, body: bool) -> Vec<u8> {
    let status = response.status();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_str(),
        status.canonical_reason().unwrap_or_default()
    )
    .into_bytes();
    let fields = response.headers();
    let named = connection_names(
        fields
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_bytes())),
    );
    for (name, value) in fields {
        if is_hop_by_hop(name.as_str(), &named) {
            continue;
        }
        head.extend_from_slice(name.as_str().as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    if body {
        head.extend_from_slice(b"transfer-encoding: chunked\r\n");
    }
    head.extend_from_slice(b"connection: close\r\n\r\n");
    head
}

/// The field names a `connection` field lists, in lowercase.
fn connection_names<'a>(fields: impl Iterator<Item = (&'a str, &'a [u8])>) -> Vec<String> {
    fields
        .filter(|(name, _)| name.eq_ignore_ascii_case("connection"))
        .flat_map(|(_, value)| value.split(|&byte| byte == b','))
        .map(|name| String::from_utf8_lossy(name.trim_ascii()).to_ascii_lowercase())
        .collect()
}

fn is_hop_by_hop(name: &str, named: &[String]) -> bool {
    HOP_BY_HOP.contains(&name) || named.iter().any(|listed| listed == name)
}

/// Returns once the client hangs up. Anything it sends after its request is ignored.
async fn hung_up(reader: &mut (impl AsyncRead + Unpin)) {
    let mut ignored = [0; 1024];
    while matches!(reader.read(&mut ignored).await, Ok(read) if read > 0) {}
}

/// An error and every error beneath it, as one line.
fn chain(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    use tokio::{
        io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, BufReader},
        net::TcpListener,
        sync::Notify,
    };
    use tokio_rustls::{
        TlsAcceptor,
        rustls::{
            ServerConfig,
            pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer},
        },
    };

    use super::*;
    use crate::egress::{Allowlist, Proxy};

    const KEY: &str = "nvapi-relay-test";
    const BODY: &str = r#"{"model":"vendor/nemo","input":"hello","stream":true}"#;
    const REFUSAL: &str = r#"{"status":429,"title":"Too Many Requests"}"#;

    /// Fast enough for a test, with room for a few tries.
    const QUICK: Backoff = Backoff {
        first: Duration::from_millis(20),
        cap: Duration::from_millis(40),
        budget: Duration::from_secs(5),
    };

    /// How the upstream answers one request.
    #[derive(Clone)]
    enum Answer {
        /// A whole response with this status, extra header lines and body.
        Whole(u16, &'static str, &'static str),
        /// A stream of server-sent events: the first part, then the rest once `gate` opens.
        Stream {
            first: &'static str,
            rest: &'static str,
            gate: Arc<Notify>,
        },
        /// The first part of a stream, then the connection closed before its end.
        BrokenOff(&'static str),
    }

    /// One request the upstream received.
    #[derive(Debug)]
    struct Seen {
        line: String,
        fields: Vec<(String, String)>,
        body: String,
    }

    impl Seen {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value.as_str())
        }
    }

    /// An HTTP endpoint on a loopback port that answers the n-th request with `script[n]`,
    /// and the last answer for every request after the script.
    struct Upstream {
        port: u16,
        seen: Arc<Mutex<Vec<Seen>>>,
        /// The certificate the endpoint serves TLS with for `localhost`, when it serves TLS.
        tls: Option<reqwest::Certificate>,
    }

    impl Upstream {
        /// A plain HTTP endpoint, reached by its IPv4 address.
        async fn start(script: Vec<Answer>) -> Self {
            Self::serve(script, None).await
        }

        /// An endpoint that serves TLS for `localhost` with a self-signed certificate.
        async fn start_tls(script: Vec<Answer>) -> Self {
            let certified = rcgen::generate_simple_self_signed(["localhost".to_owned()]).unwrap();
            let key = PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
            let config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![certified.cert.der().clone()],
                    PrivateKeyDer::Pkcs8(key),
                )
                .unwrap();
            let root = reqwest::Certificate::from_der(certified.cert.der()).unwrap();
            let acceptor = TlsAcceptor::from(Arc::new(config));
            Self {
                tls: Some(root),
                ..Self::serve(script, Some(acceptor)).await
            }
        }

        async fn serve(script: Vec<Answer>, acceptor: Option<TlsAcceptor>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let answer = {
                        let count = log.lock().unwrap().len();
                        script[count.min(script.len() - 1)].clone()
                    };
                    let (log, acceptor) = (log.clone(), acceptor.clone());
                    tokio::spawn(async move {
                        match acceptor {
                            Some(acceptor) => {
                                let Ok(stream) = acceptor.accept(stream).await else {
                                    return;
                                };
                                respond(stream, answer, &log).await;
                            }
                            None => respond(stream, answer, &log).await,
                        }
                    });
                }
            });
            Self {
                port,
                seen,
                tls: None,
            }
        }

        fn endpoint(&self) -> String {
            match self.tls {
                Some(_) => format!("https://localhost:{}/v1", self.port),
                None => format!("http://127.0.0.1:{}/v1", self.port),
            }
        }

        /// The relay a proxy holds for this endpoint.
        fn relay(&self, backoff: Backoff) -> Relay {
            let relay = Relay::new(&self.endpoint(), &Secret::new(KEY.to_owned()))
                .unwrap()
                .with_backoff(backoff);
            match &self.tls {
                Some(root) => relay.trusting(root.clone()),
                None => relay,
            }
        }

        fn seen(&self) -> std::sync::MutexGuard<'_, Vec<Seen>> {
            self.seen.lock().unwrap()
        }
    }

    /// Reads one request from `stream`, logs it and writes `answer`.
    async fn respond(
        stream: impl AsyncRead + AsyncWrite + Unpin,
        answer: Answer,
        log: &Mutex<Vec<Seen>>,
    ) {
        let mut reader = BufReader::new(stream);
        let request = read_request(&mut reader).await;
        log.lock().unwrap().push(request);
        answer.write(reader.get_mut()).await;
    }

    async fn read_request(reader: &mut (impl AsyncBufRead + Unpin)) -> Seen {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let mut fields = Vec::new();
        loop {
            let mut field = String::new();
            reader.read_line(&mut field).await.unwrap();
            let field = field.trim_end();
            if field.is_empty() {
                break;
            }
            let (name, value) = field.split_once(':').unwrap();
            fields.push((name.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let length = fields
            .iter()
            .find(|(name, _)| name == "content-length")
            .map_or(0, |(_, value)| value.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await.unwrap();
        Seen {
            line: line.trim_end().to_owned(),
            fields,
            body: String::from_utf8(body).unwrap(),
        }
    }

    fn chunk(data: &str) -> String {
        format!("{:x}\r\n{data}\r\n", data.len())
    }

    const STREAM_HEAD: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                               transfer-encoding: chunked\r\nconnection: close\r\n\r\n";

    impl Answer {
        async fn write(self, stream: &mut (impl AsyncWrite + Unpin)) {
            match self {
                Self::Whole(status, fields, body) => {
                    let head = format!(
                        "HTTP/1.1 {status} Whatever\r\ncontent-type: application/json\r\n\
                         {fields}content-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(head.as_bytes()).await.unwrap();
                    stream.write_all(body.as_bytes()).await.unwrap();
                }
                Self::Stream { first, rest, gate } => {
                    let start = format!("{STREAM_HEAD}{}", chunk(first));
                    stream.write_all(start.as_bytes()).await.unwrap();
                    gate.notified().await;
                    let end = format!("{}0\r\n\r\n", chunk(rest));
                    stream.write_all(end.as_bytes()).await.unwrap();
                }
                Self::BrokenOff(first) => {
                    let start = format!("{STREAM_HEAD}{}", chunk(first));
                    stream.write_all(start.as_bytes()).await.unwrap();
                }
            }
            let _ = stream.shutdown().await;
        }
    }

    /// A proxy whose relay forwards to `endpoint`, and where it logs.
    struct Fixture {
        _dir: tempfile::TempDir,
        socket: PathBuf,
        log: PathBuf,
        proxy: Proxy,
    }

    impl Fixture {
        async fn new(endpoint: &str, backoff: Backoff) -> Self {
            let relay = Relay::new(endpoint, &Secret::new(KEY.to_owned()))
                .unwrap()
                .with_backoff(backoff);
            Self::serving(relay).await
        }

        async fn serving(relay: Relay) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let (socket, log) = (
                dir.path().join("egress.sock"),
                dir.path().join("egress.jsonl"),
            );
            let proxy = Proxy::start(&socket, Allowlist::new(["example.com"]), Some(relay), &log)
                .await
                .unwrap();
            Self {
                _dir: dir,
                socket,
                log,
                proxy,
            }
        }

        /// Sends `request` and returns the response head and the stream to read its body.
        async fn ask(&self, request: &str) -> (Vec<String>, BufReader<UnixStream>) {
            let mut stream = UnixStream::connect(&self.socket).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let line = line.trim_end().to_owned();
                if line.is_empty() {
                    return (head, reader);
                }
                head.push(line);
            }
        }

        /// Closes the proxy and returns every record it logged.
        async fn records(self) -> Vec<Record> {
            self.proxy.close().await.unwrap();
            std::fs::read_to_string(&self.log)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    /// The agent's request: in absolute form to the relay, with a placeholder authorization
    /// and fields that belong to its connection.
    fn post(path: &str) -> String {
        format!(
            "POST http://relay.invalid{path} HTTP/1.1\r\nhost: relay.invalid\r\n\
             authorization: Bearer placeholder\r\nconnection: keep-alive, x-hop\r\nx-hop: 1\r\n\
             content-type: application/json\r\naccept: text/event-stream\r\n\
             content-length: {}\r\n\r\n{BODY}",
            BODY.len()
        )
    }

    /// The next chunk of a chunked body, or `None` at its last chunk or when the stream ends
    /// without one.
    async fn next_chunk(reader: &mut BufReader<UnixStream>) -> Option<String> {
        let mut size = String::new();
        if reader.read_line(&mut size).await.unwrap() == 0 {
            return None;
        }
        let size = usize::from_str_radix(size.trim_end(), 16).unwrap();
        let mut data = vec![0; size + 2];
        reader.read_exact(&mut data).await.unwrap();
        assert_eq!(&data[size..], b"\r\n");
        data.truncate(size);
        (size > 0).then(|| String::from_utf8(data).unwrap())
    }

    fn summary(records: &[Record]) -> Vec<(u32, u32, Option<u16>, bool)> {
        records
            .iter()
            .map(|record| {
                assert_eq!(record.verdict, Verdict::Relayed);
                let relayed = record.relay.as_ref().unwrap();
                (
                    relayed.request,
                    relayed.r#try,
                    relayed.status,
                    relayed.retry_at.is_some(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn retries_refusals_then_streams_the_answer_through_unbuffered() {
        const CREATED: &str = "event: response.created\ndata: {}\n\n";
        const COMPLETED: &str = "event: response.completed\ndata: {}\n\n";
        let gate = Arc::new(Notify::new());
        let refused = Answer::Whole(429, "", REFUSAL);
        let upstream = Upstream::start(vec![
            refused.clone(),
            Answer::Whole(503, "", "busy"),
            refused,
            Answer::Stream {
                first: CREATED,
                rest: COMPLETED,
                gate: gate.clone(),
            },
        ])
        .await;
        let fixture = Fixture::new(&upstream.endpoint(), QUICK).await;

        let (head, mut body) = fixture.ask(&post("/responses")).await;
        assert_eq!(head[0], "HTTP/1.1 200 OK");
        for field in [
            "content-type: text/event-stream",
            "transfer-encoding: chunked",
            "connection: close",
        ] {
            assert!(head.iter().any(|line| line == field), "{field}: {head:?}");
        }
        assert_eq!(head.len(), 4, "{head:?}");
        // The first event arrives while the endpoint still holds back the rest.
        assert_eq!(next_chunk(&mut body).await.as_deref(), Some(CREATED));
        gate.notify_one();
        assert_eq!(next_chunk(&mut body).await.as_deref(), Some(COMPLETED));
        assert_eq!(next_chunk(&mut body).await, None);
        drop(body);

        {
            let seen = upstream.seen();
            assert_eq!(seen.len(), 4);
            for request in seen.iter() {
                assert_eq!(request.line, "POST /v1/responses HTTP/1.1");
                assert_eq!(request.body, BODY);
                assert_eq!(
                    request.field("authorization"),
                    Some("Bearer nvapi-relay-test")
                );
                assert_eq!(request.field("accept"), Some("text/event-stream"));
                assert_eq!(request.field("content-type"), Some("application/json"));
                let host = format!("127.0.0.1:{}", upstream.port);
                assert_eq!(request.field("host"), Some(host.as_str()));
                assert_eq!(request.field("x-hop"), None);
                assert_eq!(request.field("connection"), None);
            }
        }
        let records = fixture.records().await;
        assert_eq!(
            summary(&records),
            [
                (1, 1, Some(429), true),
                (1, 2, Some(503), true),
                (1, 3, Some(429), true),
                (1, 4, Some(200), false),
            ]
        );
        for pair in records.windows(2) {
            let retry_at = pair[0].relay.as_ref().unwrap().retry_at.unwrap();
            assert!(
                pair[0].end <= retry_at && retry_at <= pair[1].start,
                "{pair:?}"
            );
        }
        let last = &records[3];
        assert_eq!(last.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(last.port, Some(upstream.port));
        assert_eq!(last.bytes_up, BODY.len() as u64);
        assert_eq!(last.bytes_down, (CREATED.len() + COMPLETED.len()) as u64);
        assert!(records[..3].iter().all(|record| record.bytes_down == 0));
        let relayed = last.relay.as_ref().unwrap();
        assert_eq!(
            (relayed.method.as_str(), relayed.path.as_str()),
            ("POST", "/responses")
        );
        assert_eq!(relayed.error, None);
    }

    #[tokio::test]
    async fn gives_the_agent_the_last_refusal_once_the_wait_runs_out() {
        let upstream = Upstream::start(vec![Answer::Whole(429, "", REFUSAL)]).await;
        let budget = Duration::from_millis(300);
        let backoff = Backoff { budget, ..QUICK };
        let fixture = Fixture::new(&upstream.endpoint(), backoff).await;

        let started = Instant::now();
        let (head, mut body) = fixture.ask(&post("/responses")).await;
        let held = started.elapsed();
        assert_eq!(head[0], "HTTP/1.1 429 Too Many Requests");
        assert_eq!(next_chunk(&mut body).await.as_deref(), Some(REFUSAL));
        assert_eq!(next_chunk(&mut body).await, None);
        assert!(held <= budget + Duration::from_millis(200), "{held:?}");

        let tries = upstream.seen().len();
        let records = fixture.records().await;
        assert!(tries >= 5, "{tries} tries");
        assert_eq!(records.len(), tries);
        let summary = summary(&records);
        assert!(
            summary[..tries - 1]
                .iter()
                .all(|try_| try_.2 == Some(429) && try_.3)
        );
        assert_eq!(summary[tries - 1], (1, tries as u32, Some(429), false));
        assert!(
            records[tries - 1].end - records[0].start
                <= chrono::TimeDelta::from_std(budget).unwrap()
                    + chrono::TimeDelta::milliseconds(200)
        );
    }

    #[tokio::test]
    async fn relays_to_an_endpoint_on_its_own_port_over_plain_http_or_tls() {
        let script = || {
            vec![
                Answer::Whole(429, "", REFUSAL),
                Answer::Whole(200, "", "{}"),
            ]
        };
        for upstream in [
            Upstream::start(script()).await,
            Upstream::start_tls(script()).await,
        ] {
            let endpoint = upstream.endpoint();
            let fixture = Fixture::serving(upstream.relay(QUICK)).await;

            let (head, mut body) = fixture.ask(&post("/responses")).await;
            assert_eq!(head[0], "HTTP/1.1 200 OK", "{endpoint}");
            assert_eq!(next_chunk(&mut body).await.as_deref(), Some("{}"));
            assert_eq!(next_chunk(&mut body).await, None);
            drop(body);

            let host = match upstream.tls {
                Some(_) => "localhost",
                None => "127.0.0.1",
            };
            {
                let seen = upstream.seen();
                assert_eq!(seen.len(), 2, "{endpoint}");
                for request in seen.iter() {
                    assert_eq!(request.line, "POST /v1/responses HTTP/1.1");
                    assert_eq!(request.body, BODY);
                    assert_eq!(
                        request.field("authorization"),
                        Some("Bearer nvapi-relay-test")
                    );
                    let authority = format!("{host}:{}", upstream.port);
                    assert_eq!(request.field("host"), Some(authority.as_str()));
                }
            }
            let records = fixture.records().await;
            assert_eq!(
                summary(&records),
                [(1, 1, Some(429), true), (1, 2, Some(200), false)],
                "{endpoint}"
            );
            for record in &records {
                assert_eq!(record.host.as_deref(), Some(host));
                assert_eq!(record.port, Some(upstream.port));
            }
        }
    }

    #[tokio::test]
    async fn waits_as_long_as_the_endpoint_says() {
        let upstream = Upstream::start(vec![
            Answer::Whole(503, "retry-after: 1\r\n", "busy"),
            Answer::Whole(200, "", "{}"),
        ])
        .await;
        let fixture = Fixture::new(&upstream.endpoint(), QUICK).await;

        let (head, _) = fixture.ask(&post("/models")).await;
        assert_eq!(head[0], "HTTP/1.1 200 OK");
        let records = fixture.records().await;
        assert_eq!(
            summary(&records),
            [(1, 1, Some(503), true), (1, 2, Some(200), false)]
        );
        let waited = records[1].start - records[0].end;
        assert!(waited >= chrono::TimeDelta::seconds(1), "{waited}");
    }

    #[tokio::test]
    async fn a_body_that_broke_off_reaches_the_agent_broken_and_is_never_retried() {
        let upstream =
            Upstream::start(vec![Answer::BrokenOff("event: response.created\n\n")]).await;
        let fixture = Fixture::new(&upstream.endpoint(), QUICK).await;

        let (head, mut body) = fixture.ask(&post("/responses")).await;
        assert_eq!(head[0], "HTTP/1.1 200 OK");
        assert_eq!(
            next_chunk(&mut body).await.as_deref(),
            Some("event: response.created\n\n")
        );
        let mut rest = Vec::new();
        body.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "the last chunk was sent: {rest:?}");

        assert_eq!(upstream.seen().len(), 1);
        let records = fixture.records().await;
        assert_eq!(summary(&records), [(1, 1, Some(200), false)]);
        assert!(records[0].relay.as_ref().unwrap().error.is_some());
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_retried_then_answered_with_502() {
        let closed = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let backoff = Backoff {
            budget: Duration::from_millis(100),
            ..QUICK
        };
        let fixture = Fixture::new(&format!("http://127.0.0.1:{closed}/v1"), backoff).await;

        let (head, _) = fixture.ask(&post("/responses")).await;
        assert_eq!(head[0], "HTTP/1.1 502 Bad Gateway");
        let records = fixture.records().await;
        assert!(records.len() >= 2, "{records:?}");
        for record in &records {
            let relayed = record.relay.as_ref().unwrap();
            assert_eq!(relayed.status, None);
            assert!(relayed.error.is_some());
        }
    }

    #[tokio::test]
    async fn serves_only_its_own_host_and_bodies_of_a_known_length() {
        let upstream = Upstream::start(vec![Answer::Whole(200, "", "{}")]).await;
        let fixture = Fixture::new(&upstream.endpoint(), QUICK).await;

        for (request, status) in [
            (
                "POST http://example.com/responses HTTP/1.1\r\ncontent-length: 0\r\n\r\n",
                "HTTP/1.1 403 Forbidden",
            ),
            (
                "POST https://relay.invalid/responses HTTP/1.1\r\ncontent-length: 0\r\n\r\n",
                "HTTP/1.1 403 Forbidden",
            ),
            (
                "POST http://relay.invalid:8080/responses HTTP/1.1\r\ncontent-length: 0\r\n\r\n",
                "HTTP/1.1 403 Forbidden",
            ),
            (
                "POST http://relay.invalid/responses HTTP/1.1\r\ntransfer-encoding: chunked\r\n\r\n0\r\n\r\n",
                "HTTP/1.1 411 Length Required",
            ),
            (
                "POST http://relay.invalid/responses HTTP/1.1\r\ncontent-length: 1\r\ncontent-length: 2\r\n\r\n",
                "HTTP/1.1 400 Bad Request",
            ),
            (
                "POST http://relay.invalid/responses HTTP/1.1\r\ncontent-length: 99999999999\r\n\r\n",
                "HTTP/1.1 413 Content Too Large",
            ),
        ] {
            let (head, _) = fixture.ask(request).await;
            assert_eq!(head[0], status, "{request:?}");
        }
        assert!(upstream.seen().is_empty());
        let records = fixture.records().await;
        assert_eq!(records.len(), 6);
        assert!(
            records
                .iter()
                .all(|record| record.verdict == Verdict::Denied && record.relay.is_none())
        );
    }

    #[test]
    fn the_url_names_the_host_the_relay_serves() {
        assert_eq!(Relay::URL, format!("http://{HOST}"));
        let url = Url::parse(Relay::URL).unwrap();
        assert_eq!(url.port_or_known_default(), Some(PORT));
    }
}
