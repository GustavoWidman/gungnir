//! A backend that runs nothing: a script decides how each command ends, and every call is
//! kept for the test to inspect.
use crate::{
    sandbox::{Backend, BoxFuture, Collected, Exit, Fault, Finished, Io, Spec},
    stop::Stop,
};
use std::{ffi::OsString, sync::Mutex, time::Duration};

type Script = dyn Fn(&Spec, &[String], Io) -> Result<Finished, Fault> + Send + Sync;

pub(crate) struct Scripted {
    script: Box<Script>,
    calls: Mutex<Vec<(Spec, Vec<String>)>>,
}

impl Scripted {
    pub fn new(
        script: impl Fn(&Spec, &[String], Io) -> Result<Finished, Fault> + Send + Sync + 'static,
    ) -> Self {
        Self {
            script: Box::new(script),
            calls: Mutex::default(),
        }
    }

    /// Every command run so far, with the spec it ran under.
    pub fn calls(&self) -> Vec<(Spec, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Backend for Scripted {
    fn exec<'a>(
        &'a self,
        spec: &'a Spec,
        argv: &'a [OsString],
        io: Io,
        _stop: &'a Stop,
    ) -> BoxFuture<'a, Result<Finished, Fault>> {
        let argv: Vec<String> = argv
            .iter()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        self.calls
            .lock()
            .unwrap()
            .push((spec.clone(), argv.clone()));
        let result = (self.script)(spec, &argv, io);
        Box::pin(async move { result })
    }
}

/// A program that ended with `exit` after writing `stdout`, all of it kept.
pub(crate) fn finished(exit: Exit, stdout: &[u8]) -> Finished {
    Finished {
        exit,
        wall: Duration::from_millis(5),
        stdout: Collected {
            bytes: stdout.to_vec(),
            total: stdout.len() as u64,
            truncated: false,
        },
        stderr: Collected::default(),
    }
}
