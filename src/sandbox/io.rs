//! What flows into and out of a sandboxed program.

use std::{path::PathBuf, process::Stdio};

use eyre::{Result, WrapErr};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::ChildStdin,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

/// How much of stderr is kept in every mode, to explain a sandbox that failed to start.
pub(super) const DIAGNOSTIC: usize = 2048;

pub struct Io {
    pub stdin: Input,
    pub stdout: Output,
    pub stderr: Output,
    pub kill: Kill,
}

pub enum Input {
    Null,
    Bytes(Vec<u8>),
    /// A host file read directly as stdin, never copied into the sandbox.
    File(PathBuf),
}

/// Where one output stream goes. Every mode keeps at most `cap` bytes and drains the rest, so
/// the program never blocks on a full pipe.
pub enum Output {
    Capture {
        cap: u64,
    },
    /// Written to a host file as the bytes arrive.
    File {
        path: PathBuf,
        cap: u64,
    },
    /// Each line, without its newline, sent as it arrives. Lines stop at the first one that
    /// would pass `cap`.
    Lines {
        sink: mpsc::Sender<Vec<u8>>,
        cap: u64,
    },
}

/// What became of one output stream.
#[derive(Debug, Default)]
pub struct Collected {
    /// The kept bytes for `Output::Capture`; empty for the other modes.
    pub bytes: Vec<u8>,
    /// Every byte the program wrote, kept or not.
    pub total: u64,
    pub truncated: bool,
}

/// Ends one running program early. Clones share the switch.
#[derive(Clone, Debug, Default)]
pub struct Kill(CancellationToken);

impl Kill {
    pub fn kill(&self) {
        self.0.cancel();
    }

    pub(super) async fn requested(&self) {
        self.0.cancelled().await;
    }
}

impl Input {
    /// The child's stdin, and the bytes to feed into it once it runs.
    pub(super) async fn open(self) -> Result<(Stdio, Option<Vec<u8>>)> {
        Ok(match self {
            Self::Null => (Stdio::null(), None),
            Self::Bytes(bytes) => (Stdio::piped(), Some(bytes)),
            Self::File(path) => {
                let file = File::open(&path)
                    .await
                    .wrap_err_with(|| format!("opening stdin {}", path.display()))?;
                (file.into_std().await.into(), None)
            }
        })
    }
}

pub(super) async fn feed(stdin: Option<ChildStdin>, bytes: Option<Vec<u8>>) {
    if let (Some(mut stdin), Some(bytes)) = (stdin, bytes) {
        // A program may exit without reading all of its input.
        let _ = stdin.write_all(&bytes).await;
    }
}

/// An opened `Output`, with the budget of bytes it may still keep.
pub(super) struct Sink {
    target: Target,
    cap: u64,
    remaining: u64,
}

enum Target {
    Memory(Vec<u8>),
    File(File),
    Lines {
        sink: mpsc::Sender<Vec<u8>>,
        line: Vec<u8>,
        exhausted: bool,
    },
}

impl Output {
    /// Opens the destination before the program starts, so a bad path is a fault, not a
    /// half-run program.
    pub(super) async fn open(self) -> Result<Sink> {
        let (target, cap) = match self {
            Self::Capture { cap } => (Target::Memory(Vec::new()), cap),
            Self::File { path, cap } => {
                let file = File::create(&path)
                    .await
                    .wrap_err_with(|| format!("creating {}", path.display()))?;
                (Target::File(file), cap)
            }
            Self::Lines { sink, cap } => (
                Target::Lines {
                    sink,
                    line: Vec::new(),
                    exhausted: false,
                },
                cap,
            ),
        };
        Ok(Sink {
            target,
            cap,
            remaining: cap,
        })
    }
}

impl Sink {
    async fn accept(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let take = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        match &mut self.target {
            Target::Memory(kept) => kept.extend_from_slice(&bytes[..take]),
            Target::File(file) => file.write_all(&bytes[..take]).await?,
            Target::Lines {
                sink,
                line,
                exhausted,
            } => {
                for segment in bytes.split_inclusive(|&byte| byte == b'\n') {
                    if *exhausted {
                        break;
                    }
                    if (line.len() + segment.len()) as u64 > self.remaining {
                        *exhausted = true;
                        line.clear();
                        break;
                    }
                    line.extend_from_slice(segment);
                    if line.ends_with(b"\n") {
                        self.remaining -= line.len() as u64;
                        line.pop();
                        // A caller that stopped listening still lets the program run on.
                        let _ = sink.send(std::mem::take(line)).await;
                    }
                }
                return Ok(());
            }
        }
        self.remaining -= take as u64;
        Ok(())
    }

    async fn finish(mut self, total: u64) -> std::io::Result<Collected> {
        let bytes = match &mut self.target {
            Target::Memory(kept) => std::mem::take(kept),
            Target::File(file) => {
                file.flush().await?;
                Vec::new()
            }
            Target::Lines {
                sink,
                line,
                exhausted,
            } => {
                if !*exhausted && !line.is_empty() {
                    self.remaining -= line.len() as u64;
                    let _ = sink.send(std::mem::take(line)).await;
                }
                Vec::new()
            }
        };
        Ok(Collected {
            bytes,
            total,
            truncated: total > self.cap - self.remaining,
        })
    }
}

/// Reads `reader` to its end into `sink`. Also returns the first `DIAGNOSTIC` bytes.
pub(super) async fn drain(
    mut reader: impl AsyncRead + Unpin,
    mut sink: Sink,
) -> std::io::Result<(Collected, Vec<u8>)> {
    let mut head = Vec::new();
    let mut total = 0;
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let bytes = &chunk[..read];
        head.extend_from_slice(&bytes[..read.min(DIAGNOSTIC - head.len())]);
        total += read as u64;
        sink.accept(bytes).await?;
    }
    Ok((sink.finish(total).await?, head))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect(output: Output, input: &[u8]) -> Result<Collected> {
        let (collected, _) = drain(input, output.open().await?).await?;
        Ok(collected)
    }

    #[tokio::test]
    async fn capture_keeps_at_most_its_cap() -> Result<()> {
        let whole = collect(Output::Capture { cap: 16 }, b"hello").await?;
        assert_eq!((whole.bytes.as_slice(), whole.total), (&b"hello"[..], 5));
        assert!(!whole.truncated);

        let cut = collect(Output::Capture { cap: 3 }, b"hello").await?;
        assert_eq!((cut.bytes.as_slice(), cut.total), (&b"hel"[..], 5));
        assert!(cut.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn file_receives_the_capped_stream() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("stdout.log");
        let collected = collect(
            Output::File {
                path: path.clone(),
                cap: 4,
            },
            b"0123456789",
        )
        .await?;
        assert_eq!(tokio::fs::read(&path).await?, b"0123");
        assert_eq!(collected.total, 10);
        assert!(collected.truncated && collected.bytes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn lines_arrive_whole_and_stop_at_the_cap() -> Result<()> {
        let (sink, mut lines) = mpsc::channel(8);
        let collected = collect(Output::Lines { sink, cap: 12 }, b"one\ntwo\nthree\nfour").await?;
        let mut received = Vec::new();
        while let Some(line) = lines.recv().await {
            received.push(String::from_utf8(line)?);
        }
        assert_eq!(received, ["one", "two"]);
        assert_eq!(collected.total, 18);
        assert!(collected.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn a_last_line_without_newline_is_delivered() -> Result<()> {
        let (sink, mut lines) = mpsc::channel(8);
        let collected = collect(Output::Lines { sink, cap: 64 }, b"{\"a\":1}\n{\"b\":2}").await?;
        assert_eq!(lines.recv().await.as_deref(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(lines.recv().await.as_deref(), Some(&b"{\"b\":2}"[..]));
        assert!(!collected.truncated);
        Ok(())
    }
}
