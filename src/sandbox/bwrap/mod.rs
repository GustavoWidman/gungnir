//! The bubblewrap backend: unprivileged namespaces inside a transient systemd scope that
//! enforces the memory and task limits.

mod argv;
mod scope;

use std::{
    ffi::OsString,
    fs,
    os::unix::process::ExitStatusExt,
    process::{ExitStatus, Stdio},
    time::{Duration, Instant},
};

use command_fds::{CommandFdExt, FdMapping};
use eyre::{Result, WrapErr, ensure, eyre};
use nix::{
    errno::Errno,
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::unix::pipe,
    process::{Child, Command},
};

use self::{
    argv::{ARGS_FD, Invocation, Layout, READY_FD},
    scope::Scope,
};
use super::{Backend, BoxFuture, Exit, Fault, Finished, Io, Spec, io};
use crate::{
    secret::Secret,
    stop::{Stage, Stop},
    toolchains::Toolchain,
};

pub struct Bubblewrap {
    base: Toolchain,
    etc: TempDir,
}

impl Bubblewrap {
    /// A backend whose sandboxes run on `base`, the sandbox base toolchain, which provides the
    /// shell, the egress bridge and the CA bundle of every sandbox.
    pub fn new(base: Toolchain) -> Result<Self> {
        for file in argv::BASE_FILES {
            let path = base.root().join(file);
            ensure!(
                path.exists(),
                "the sandbox base lacks {file} ({}); add its package to [sandbox].packages",
                path.display()
            );
        }
        let etc = tempfile::Builder::new()
            .prefix("gungnir-etc-")
            .tempdir()
            .wrap_err("creating the sandbox /etc")?;
        for (name, contents) in argv::etc_files() {
            fs::write(etc.path().join(name), contents)
                .wrap_err_with(|| format!("writing the sandbox /etc/{name}"))?;
        }
        Ok(Self { base, etc })
    }

    async fn run(
        &self,
        spec: &Spec,
        command: &[OsString],
        io: Io,
        stop: &Stop,
    ) -> Result<Finished, Fault> {
        if command.is_empty() {
            return Err(eyre!("no command to run").into());
        }
        let scope = Scope::next();
        let layout = Layout {
            unit: scope.unit(),
            base: &self.base,
            etc: self.etc.path(),
        };
        let Invocation { argv, secret_args } = argv::invocation(&layout, spec, command);
        let (stdin, input) = io.stdin.open().await?;
        let stdout_sink = io.stdout.open().await?;
        let stderr_sink = io.stderr.open().await?;

        let Launched {
            mut child,
            ready,
            args,
        } = launch(&argv, stdin, secret_args.is_some())?;
        let child_stdin = child.stdin.take();
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| eyre!("stdout is not piped"))?;
        let child_stderr = child
            .stderr
            .take()
            .ok_or_else(|| eyre!("stderr is not piped"))?;

        let (ended, stdout, stderr, started, (), ()) = tokio::join!(
            supervise(&mut child, spec.limits.wall, &io.kill, stop),
            io::drain(child_stdout, stdout_sink),
            io::drain(child_stderr, stderr_sink),
            started(ready),
            io::feed(child_stdin, input),
            send_args(args, secret_args),
        );
        let out_of_memory = scope.settle().await?;
        let Ended { status, wall, cut } = ended?;
        let (stdout, _) = stdout.wrap_err("reading stdout")?;
        let (stderr, diagnostic) = stderr.wrap_err("reading stderr")?;

        let exit = conclude(cut, started?, out_of_memory, status).ok_or_else(|| {
            eyre!(
                "the sandbox did not start ({status}): {}",
                String::from_utf8_lossy(&diagnostic).trim()
            )
        })?;
        Ok(Finished {
            exit,
            wall,
            stdout,
            stderr,
        })
    }
}

impl Backend for Bubblewrap {
    fn exec<'a>(
        &'a self,
        spec: &'a Spec,
        argv: &'a [OsString],
        io: Io,
        stop: &'a Stop,
    ) -> BoxFuture<'a, Result<Finished, Fault>> {
        Box::pin(self.run(spec, argv, io, stop))
    }
}

struct Launched {
    child: Child,
    /// Read end of the launcher's `READY_FD`.
    ready: pipe::Receiver,
    /// Write end of bwrap's `--args` fd, when there are secrets to send.
    args: Option<pipe::Sender>,
}

/// Spawns the invocation in its own process group with the ready and argument pipes on their
/// fixed descriptors.
fn launch(argv: &[OsString], stdin: Stdio, secrets: bool) -> Result<Launched> {
    let (ready_writer, ready) = pipe::pipe().wrap_err("creating the ready pipe")?;
    let mut mappings = vec![FdMapping {
        parent_fd: ready_writer.into_blocking_fd()?,
        child_fd: READY_FD,
    }];
    let args = if secrets {
        let (args, args_reader) = pipe::pipe().wrap_err("creating the argument pipe")?;
        mappings.push(FdMapping {
            parent_fd: args_reader.into_blocking_fd()?,
            child_fd: ARGS_FD,
        });
        Some(args)
    } else {
        None
    };
    let (program, rest) = argv
        .split_first()
        .ok_or_else(|| eyre!("empty invocation"))?;
    let mut command = Command::new(program);
    command
        .args(rest)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .fd_mappings(mappings)?;
    let child = command
        .spawn()
        .wrap_err_with(|| format!("starting {}", program.display()))?;
    // The command holds the child's ends of both pipes; dropping it lets them close.
    drop(command);
    Ok(Launched { child, ready, args })
}

async fn send_args(args: Option<pipe::Sender>, payload: Option<Secret<Vec<u8>>>) {
    if let (Some(mut args), Some(payload)) = (args, payload) {
        // A failed write means bwrap is gone, which the silent ready pipe reports as a fault.
        let _ = args.write_all(payload.expose()).await;
    }
}

/// Whether the launcher got as far as exec'ing the command.
async fn started(mut ready: pipe::Receiver) -> Result<bool> {
    let mut signal = Vec::new();
    ready
        .read_to_end(&mut signal)
        .await
        .wrap_err("reading the ready pipe")?;
    Ok(!signal.is_empty())
}

/// Why gungnir ended the program itself.
#[derive(Clone, Copy, Debug)]
enum Cut {
    TimedOut,
    Killed,
}

struct Ended {
    status: ExitStatus,
    wall: Duration,
    cut: Option<Cut>,
}

async fn supervise(
    child: &mut Child,
    wall: Duration,
    kill: &io::Kill,
    stop: &Stop,
) -> Result<Ended> {
    let launched = Instant::now();
    let cut = tokio::select! {
        biased;
        status = child.wait() => {
            return Ok(Ended { status: status?, wall: launched.elapsed(), cut: None });
        }
        () = tokio::time::sleep(wall) => Cut::TimedOut,
        () = kill.requested() => Cut::Killed,
        () = stop.reached(Stage::Killing) => Cut::Killed,
    };
    kill_group(child)?;
    let status = child.wait().await?;
    Ok(Ended {
        status,
        wall: launched.elapsed(),
        cut: Some(cut),
    })
}

/// Kills the sandbox's process group. bwrap leads it; its death takes the namespace with it,
/// because `--die-with-parent` and pid-namespace teardown kill everything inside.
fn kill_group(child: &Child) -> Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    match killpg(Pid::from_raw(i32::try_from(pid)?), Signal::SIGKILL) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(error).wrap_err("killing the sandbox"),
    }
}

/// How the program ended, or `None` when it never started, which is the sandbox's fault. A kill
/// by gungnir explains the end better than the status it caused, and a timeout better than an
/// OOM kill along the way. An OOM kill explains the end only of a program that a signal ended:
/// a program that exited on its own outlived the process the kernel killed.
fn conclude(
    cut: Option<Cut>,
    started: bool,
    out_of_memory: bool,
    status: ExitStatus,
) -> Option<Exit> {
    Some(match cut {
        Some(Cut::Killed) => Exit::Killed,
        _ if !started => return None,
        Some(Cut::TimedOut) => Exit::TimedOut,
        None => match exit(status) {
            Exit::Signal(_) if out_of_memory => Exit::OutOfMemory,
            exit => exit,
        },
    })
}

/// bwrap exits with the command's code, or 128 plus the signal that killed it, as a shell does.
fn exit(status: ExitStatus) -> Exit {
    match (status.code(), status.signal()) {
        (Some(code @ 129..=192), _) => Exit::Signal(code - 128),
        (Some(code), _) => Exit::Code(code),
        (None, Some(signal)) => Exit::Signal(signal),
        (None, None) => Exit::Code(-1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_shell_convention() {
        let exited = |code: i32| ExitStatus::from_raw(code << 8);
        assert_eq!(exit(exited(0)), Exit::Code(0));
        assert_eq!(exit(exited(3)), Exit::Code(3));
        assert_eq!(exit(exited(128 + 11)), Exit::Signal(11));
        assert_eq!(exit(ExitStatus::from_raw(9)), Exit::Signal(9));
    }

    #[test]
    fn our_own_kill_explains_the_end_first() {
        let killed = ExitStatus::from_raw(9);
        let done = |cut, started, oom| conclude(cut, started, oom, killed);
        assert_eq!(done(Some(Cut::Killed), false, true), Some(Exit::Killed));
        assert_eq!(done(Some(Cut::TimedOut), false, false), None);
        assert_eq!(done(Some(Cut::TimedOut), true, true), Some(Exit::TimedOut));
        assert_eq!(done(None, true, true), Some(Exit::OutOfMemory));
        assert_eq!(done(None, false, false), None);
        assert_eq!(
            conclude(None, true, false, ExitStatus::from_raw(2 << 8)),
            Some(Exit::Code(2))
        );
    }

    #[test]
    fn a_program_that_exits_on_its_own_outlived_an_oom_kill() {
        assert_eq!(
            conclude(None, true, true, ExitStatus::from_raw(0)),
            Some(Exit::Code(0))
        );
        assert_eq!(
            conclude(None, true, true, ExitStatus::from_raw((128 + 9) << 8)),
            Some(Exit::OutOfMemory)
        );
    }
}
