//! The host-side egress proxy. Agents have no network of their own: each attempt gets an HTTP
//! CONNECT proxy on its own Unix socket that tunnels TLS, uninspected, to the backend's
//! allowlisted hosts and logs every connection the agent attempted. For a backend that
//! relays, the proxy also serves the agent's plain model requests to [`Relay::URL`] itself.

mod allowlist;
mod relay;
mod request;
mod retry;
mod tunnel;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    net::UnixListener,
    sync::mpsc,
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub use allowlist::Allowlist;
pub use relay::{Relay, Relayed};

/// Pause after a failed accept (such as running out of descriptors) instead of spinning.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// One line of the egress log: a connection the agent attempted and what became of it, or one
/// try the relay made of an agent's request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The host the client named, absent when its request named none. For a relayed try, the
    /// endpoint's host.
    pub host: Option<String>,
    pub port: Option<u16>,
    pub verdict: Verdict,
    /// Bytes the client sent through the tunnel, after the request head. For a relayed try,
    /// the request body sent to the endpoint.
    pub bytes_up: u64,
    /// Bytes that came back through the tunnel. For a relayed try, the response body the
    /// agent got, which is none for a try the relay retried.
    pub bytes_down: u64,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// What the relay did, for a relayed try.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<Relayed>,
}

impl Record {
    fn opened(start: DateTime<Utc>) -> Self {
        Self {
            host: None,
            port: None,
            verdict: Verdict::Denied,
            bytes_up: 0,
            bytes_down: 0,
            start,
            end: start,
            relay: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// Tunnelled to an allowlisted host.
    Allowed,
    /// Refused: with 403 when it is neither a CONNECT to an allowlisted host nor a request to
    /// the relay, or no request at all, and with another 4xx when the relay cannot take it.
    Denied,
    /// Allowlisted, but the host could not be reached; answered with 502.
    Unreachable,
    /// A try of a request to the relay, which forwarded it to its endpoint.
    Relayed,
}

/// Access to a running proxy, handed to a sandbox as its only route out.
#[derive(Clone, Debug)]
pub struct Grant {
    socket: PathBuf,
}

impl Grant {
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    #[cfg(test)]
    pub(crate) fn at(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }
}

/// A proxy listening on one attempt's socket.
pub struct Proxy {
    socket: PathBuf,
    closing: CancellationToken,
    tasks: TaskTracker,
    log: Option<JoinHandle<Result<()>>>,
}

impl Proxy {
    /// Listens on `socket` and appends one JSON line per connection, and per relayed try, to
    /// `log`. With a `relay`, requests to [`Relay::URL`] go to its endpoint.
    pub async fn start(
        socket: &Path,
        allowlist: Allowlist,
        relay: Option<Relay>,
        log: &Path,
    ) -> Result<Self> {
        let listener = UnixListener::bind(socket)
            .wrap_err_with(|| format!("binding egress socket {}", socket.display()))?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .await
            .wrap_err_with(|| format!("opening egress log {}", log.display()))?;
        let (records, written) = mpsc::unbounded_channel();
        let closing = CancellationToken::new();
        let tasks = TaskTracker::new();
        tasks.spawn(accept(
            listener,
            Arc::new(allowlist),
            relay.map(Arc::new),
            records,
            closing.clone(),
            tasks.clone(),
        ));
        Ok(Self {
            socket: socket.to_owned(),
            closing,
            tasks,
            log: Some(tokio::spawn(write_log(file, written))),
        })
    }

    pub fn grant(&self) -> Grant {
        Grant {
            socket: self.socket.clone(),
        }
    }

    /// Stops accepting, ends open tunnels, and returns once every record is written. Call it
    /// after the sandbox using the proxy has finished.
    pub async fn close(mut self) -> Result<()> {
        self.closing.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        let log = self
            .log
            .take()
            .ok_or_else(|| eyre!("egress log closed twice"))?;
        log.await.wrap_err("egress log writer panicked")?
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.closing.cancel();
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn accept(
    listener: UnixListener,
    allowlist: Arc<Allowlist>,
    relay: Option<Arc<Relay>>,
    records: mpsc::UnboundedSender<Record>,
    closing: CancellationToken,
    tasks: TaskTracker,
) {
    loop {
        let client = tokio::select! {
            () = closing.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((client, _)) => client,
                Err(_) => {
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
        };
        let (allowlist, relay) = (allowlist.clone(), relay.clone());
        let (records, closing) = (records.clone(), closing.clone());
        tasks.spawn(async move {
            let record =
                tunnel::serve(client, &allowlist, relay.as_deref(), &records, &closing).await;
            // The writer only stops early on a write error, which `close` reports.
            let _ = records.send(record);
        });
    }
}

async fn write_log(mut file: File, mut records: mpsc::UnboundedReceiver<Record>) -> Result<()> {
    while let Some(record) = records.recv().await {
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        file.write_all(&line).await?;
        file.flush().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, BufReader},
        net::{TcpListener, UnixStream},
    };

    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        socket: PathBuf,
        log: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self> {
            let dir = tempfile::tempdir()?;
            Ok(Self {
                socket: dir.path().join("egress.sock"),
                log: dir.path().join("egress.jsonl"),
                _dir: dir,
            })
        }

        async fn records(&self) -> Result<Vec<Record>> {
            let log = tokio::fs::read_to_string(&self.log).await?;
            Ok(log
                .lines()
                .map(serde_json::from_str)
                .collect::<Result<_, _>>()?)
        }
    }

    /// Accepts one TCP connection on loopback and echoes it back until end of stream.
    async fn echo_server() -> Result<u16> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let (mut read, mut write) = stream.split();
            tokio::io::copy(&mut read, &mut write).await?;
            write.shutdown().await?;
            Ok::<_, std::io::Error>(())
        });
        Ok(port)
    }

    /// Sends `request` and returns the status line of the response, keeping the stream open.
    async fn ask(socket: &Path, request: &str) -> Result<(String, BufReader<UnixStream>)> {
        let mut stream = UnixStream::connect(socket).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).await?;
        let mut header = String::new();
        while reader.read_line(&mut header).await? > 2 {
            header.clear();
        }
        Ok((status.trim_end().to_owned(), reader))
    }

    #[tokio::test]
    async fn tunnels_an_allowlisted_host_and_logs_the_traffic() -> Result<()> {
        let fixture = Fixture::new()?;
        let port = echo_server().await?;
        let allowlist = Allowlist::new(["127.0.0.1"]).on_port(port);
        let proxy = Proxy::start(&fixture.socket, allowlist, None, &fixture.log).await?;

        let (status, mut tunnel) = ask(
            &proxy.grant().socket,
            &format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nhello"),
        )
        .await?;
        assert_eq!(status, "HTTP/1.1 200 Connection Established");
        tunnel.get_mut().write_all(b", tunnel").await?;
        tunnel.get_mut().shutdown().await?;
        let mut echoed = String::new();
        tunnel.read_to_string(&mut echoed).await?;
        assert_eq!(echoed, "hello, tunnel");
        drop(tunnel);
        proxy.close().await?;

        let [record] = &fixture.records().await?[..] else {
            panic!("expected one record");
        };
        assert_eq!(record.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(record.port, Some(port));
        assert_eq!(record.verdict, Verdict::Allowed);
        assert_eq!((record.bytes_up, record.bytes_down), (13, 13));
        assert!(record.start <= record.end);
        Ok(())
    }

    #[tokio::test]
    async fn refuses_everything_else_with_403() -> Result<()> {
        let fixture = Fixture::new()?;
        let port = echo_server().await?;
        let allowlist = Allowlist::new(["127.0.0.1", "api.z.ai"]).on_port(port);
        let proxy = Proxy::start(&fixture.socket, allowlist, None, &fixture.log).await?;

        for request in [
            "CONNECT example.com:443 HTTP/1.1\r\n\r\n".to_owned(),
            format!("GET http://127.0.0.1:{port}/ HTTP/1.1\r\n\r\n"),
            "CONNECT api.z.ai:22 HTTP/1.1\r\n\r\n".to_owned(),
            "POST http://relay.invalid/responses HTTP/1.1\r\n\r\n".to_owned(),
            "not http\r\n\r\n".to_owned(),
        ] {
            let (status, _) = ask(&fixture.socket, &request).await?;
            assert_eq!(status, "HTTP/1.1 403 Forbidden", "{request:?}");
        }
        proxy.close().await?;

        let targets: Vec<_> = fixture
            .records()
            .await?
            .into_iter()
            .map(|record| {
                assert_eq!(record.verdict, Verdict::Denied);
                assert_eq!((record.bytes_up, record.bytes_down), (0, 0));
                (record.host, record.port)
            })
            .collect();
        assert_eq!(
            targets,
            [
                (Some("example.com".to_owned()), Some(443)),
                (Some("127.0.0.1".to_owned()), Some(port)),
                (Some("api.z.ai".to_owned()), Some(22)),
                (Some("relay.invalid".to_owned()), Some(80)),
                (None, None),
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn an_unreachable_allowlisted_host_gets_502() -> Result<()> {
        let fixture = Fixture::new()?;
        let closed = TcpListener::bind("127.0.0.1:0").await?.local_addr()?.port();
        let allowlist = Allowlist::new(["127.0.0.1"]).on_port(closed);
        let proxy = Proxy::start(&fixture.socket, allowlist, None, &fixture.log).await?;

        let (status, _) = ask(
            &fixture.socket,
            &format!("CONNECT 127.0.0.1:{closed} HTTP/1.1\r\n\r\n"),
        )
        .await?;
        assert_eq!(status, "HTTP/1.1 502 Bad Gateway");
        proxy.close().await?;

        let records = fixture.records().await?;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].verdict, Verdict::Unreachable);
        Ok(())
    }

    #[tokio::test]
    async fn close_ends_open_tunnels_and_still_logs_them() -> Result<()> {
        let fixture = Fixture::new()?;
        let port = echo_server().await?;
        let allowlist = Allowlist::new(["127.0.0.1"]).on_port(port);
        let proxy = Proxy::start(&fixture.socket, allowlist, None, &fixture.log).await?;

        let (_, mut tunnel) = ask(
            &fixture.socket,
            &format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\n\r\n"),
        )
        .await?;
        tunnel.get_mut().write_all(b"ping").await?;
        let mut echoed = [0; 4];
        tunnel.read_exact(&mut echoed).await?;
        tokio::time::timeout(Duration::from_secs(5), proxy.close()).await??;

        let records = fixture.records().await?;
        assert_eq!(records.len(), 1);
        assert_eq!((records[0].bytes_up, records[0].bytes_down), (4, 4));
        assert!(!fixture.socket.exists());
        Ok(())
    }
}
