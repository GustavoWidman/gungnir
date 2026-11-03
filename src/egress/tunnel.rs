//! One client connection: read its request, refuse it or open the tunnel, count the bytes.

use std::{io, time::Duration};

use chrono::Utc;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpStream, UnixStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use super::{
    Record, Verdict,
    allowlist::Allowlist,
    request::{self, MAX_HEAD},
};

const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";
const FORBIDDEN: &[u8] =
    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Serves one connection to its end, or until the proxy closes, and returns its log record.
pub(super) async fn serve(
    mut client: UnixStream,
    allowlist: &Allowlist,
    closing: &CancellationToken,
) -> Record {
    let mut record = Record::opened(Utc::now());
    tokio::select! {
        // An I/O error only ends the connection; the record already holds what happened.
        _ = handle(&mut client, allowlist, &mut record) => {}
        () = closing.cancelled() => {}
    }
    record.end = Utc::now();
    record
}

/// Fills `record` as the connection progresses, so a cancelled connection still logs its
/// target, verdict and the bytes moved so far.
async fn handle(
    client: &mut UnixStream,
    allowlist: &Allowlist,
    record: &mut Record,
) -> io::Result<()> {
    let Ok(Ok(Some((head, early)))) = timeout(HEAD_TIMEOUT, read_head(client)).await else {
        return client.write_all(FORBIDDEN).await;
    };
    let request = request::parse(&head);
    if let Some(request) = &request {
        record.host = Some(request.host.as_str().to_owned());
        record.port = Some(request.port);
    }
    let Some(request) =
        request.filter(|request| request.tunnel && allowlist.allows(&request.host, request.port))
    else {
        return client.write_all(FORBIDDEN).await;
    };
    let target = (request.host.as_str(), request.port);
    let Ok(Ok(upstream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect(target)).await else {
        record.verdict = Verdict::Unreachable;
        return client.write_all(BAD_GATEWAY).await;
    };
    record.verdict = Verdict::Allowed;
    client.write_all(ESTABLISHED).await?;
    relay(client, upstream, &early, record).await
}

/// Reads up to the end of the request head. Returns the head and any bytes the client sent
/// after it, or `None` when the client closed or the head grew past `MAX_HEAD`.
async fn read_head(client: &mut UnixStream) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        if let Some(len) = request::head_len(&buffer) {
            let early = buffer.split_off(len);
            return Ok(Some((buffer, early)));
        }
        if buffer.len() >= MAX_HEAD {
            return Ok(None);
        }
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

async fn relay(
    client: &mut UnixStream,
    mut upstream: TcpStream,
    early: &[u8],
    record: &mut Record,
) -> io::Result<()> {
    upstream.write_all(early).await?;
    record.bytes_up += early.len() as u64;
    let (mut client_read, mut client_write) = client.split();
    let (mut upstream_read, mut upstream_write) = upstream.split();
    tokio::try_join!(
        pump(&mut client_read, &mut upstream_write, &mut record.bytes_up),
        pump(
            &mut upstream_read,
            &mut client_write,
            &mut record.bytes_down
        ),
    )?;
    Ok(())
}

/// Copies one direction until end of stream, then half-closes the other side so the peer sees
/// the end too.
async fn pump(
    from: &mut (impl AsyncRead + Unpin),
    to: &mut (impl AsyncWrite + Unpin),
    moved: &mut u64,
) -> io::Result<()> {
    let mut buffer = vec![0; 16 * 1024];
    loop {
        let read = from.read(&mut buffer).await?;
        if read == 0 {
            return to.shutdown().await;
        }
        to.write_all(&buffer[..read]).await?;
        *moved += read as u64;
    }
}
