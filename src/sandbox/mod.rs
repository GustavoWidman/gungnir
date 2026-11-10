//! The only place gungnir runs untrusted code. A `Spec` describes what a sandbox exposes, and a
//! `Backend` runs one command under it.

mod bwrap;
mod io;

use std::{ffi::OsString, fmt, future::Future, path::PathBuf, pin::Pin, time::Duration};

pub use bwrap::Bubblewrap;
pub use io::{Collected, Input, Io, Kill, Output};

use crate::{egress::Grant, secret::EnvValue, stop::Stop, toolchains::Closure};

/// Where the workspace is mounted in every sandbox. Guest paths are constants, so nothing the
/// program sees names the trial, model or effort.
pub const WORK: &str = "/work";
/// The home directory in every sandbox: a private tmpfs unless a mount replaces it.
pub const HOME: &str = "/home/agent";

/// What one sandbox exposes to the program it runs.
#[derive(Clone, Debug)]
pub struct Spec {
    pub network: Network,
    /// Store paths bound read-only, beside the sandbox base's closure. Nothing else of
    /// `/nix/store` is visible.
    pub closure: Closure,
    /// Host paths bound into the guest, in order, after everything else.
    pub mounts: Vec<Mount>,
    /// PATH inside: the language, the sandbox base, and the harness for agents.
    pub path: Vec<PathBuf>,
    pub env: Vec<(String, EnvValue)>,
    pub cwd: PathBuf,
    pub limits: Limits,
}

#[derive(Clone, Debug)]
pub enum Network {
    Offline,
    /// Reachable only through the host-side egress proxy behind this grant.
    Egress(Grant),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    pub writable: bool,
}

impl Mount {
    pub fn read_only(host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        Self {
            host: host.into(),
            guest: guest.into(),
            writable: false,
        }
    }

    pub fn writable(host: impl Into<PathBuf>, guest: impl Into<PathBuf>) -> Self {
        Self {
            writable: true,
            ..Self::read_only(host, guest)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub wall: Duration,
    /// Memory in bytes, swap excluded.
    pub memory: u64,
    pub tasks: u32,
    /// The one CPU to pin the sandbox to, for measurement.
    pub cpu: Option<u32>,
    pub oom: OomKill,
}

/// What the kernel's kill at the memory limit ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OomKill {
    /// The whole sandbox: a program under test that runs out of memory has failed.
    Sandbox,
    /// Only the process the kernel picks, as on an ordinary machine, so an agent's session
    /// outlives a program of its own that runs out of memory and sees that command killed.
    Process,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Runs commands under a `Spec`. Object safe, so backends can be chosen at runtime.
pub trait Backend: Send + Sync {
    /// Runs `argv` to its end. The program is killed when the wall-time limit passes, when
    /// `io.kill` fires, or when `stop` reaches `Stage::Killing`.
    fn exec<'a>(
        &'a self,
        spec: &'a Spec,
        argv: &'a [OsString],
        io: Io,
        stop: &'a Stop,
    ) -> BoxFuture<'a, Result<Finished, Fault>>;
}

/// How a program that started ended, whatever the reason.
#[derive(Debug)]
pub struct Finished {
    pub exit: Exit,
    /// From launch to exit, sandbox setup included.
    pub wall: Duration,
    pub stdout: Collected,
    pub stderr: Collected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    /// Ended by a signal it did not get from gungnir.
    Signal(i32),
    /// Killed at the wall-time limit.
    TimedOut,
    /// The kernel killed the program at the memory limit.
    OutOfMemory,
    /// Killed through `Io::kill` or the second Ctrl-C.
    Killed,
}

impl Exit {
    pub fn success(self) -> bool {
        self == Self::Code(0)
    }
}

/// The sandbox itself could not run the command. Never the program's fault: a grader records
/// it as an environment error.
#[derive(Debug)]
pub struct Fault(eyre::Report);

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sandbox fault: {:#}", self.0)
    }
}

impl std::error::Error for Fault {}

impl From<eyre::Report> for Fault {
    fn from(report: eyre::Report) -> Self {
        Self(report)
    }
}
