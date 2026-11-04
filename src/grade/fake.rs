//! Test doubles for sandboxed work: a backend whose script decides how each command ends, and
//! toolchains that name store paths no sandbox ever binds.
use crate::{
    sandbox::{Backend, BoxFuture, Collected, Exit, Fault, Finished, Io, Spec},
    stop::Stop,
    testing::write,
    toolchains::Toolchains,
};
use std::{ffi::OsString, os::unix::fs::symlink, path::Path, sync::Mutex, time::Duration};

type Script = dyn Fn(&Spec, &[String], Io) -> Result<Finished, Fault> + Send + Sync;

/// Runs nothing. Every call is kept for the test to inspect.
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

/// Writes a `$GUNGNIR_TOOLCHAINS` layout under `dir` with the base, the measurement and
/// profiling tools and one environment per language, and loads it as if each language had
/// reported version 1.0.
pub(crate) fn toolchains(dir: &Path, languages: &[&str]) -> Toolchains {
    let base = ("base".to_owned(), "aaaa-gungnir-base".to_owned());
    let measure = ("measure".to_owned(), "cccc-gungnir-measure".to_owned());
    let profile = ("profile".to_owned(), "dddd-gungnir-profile".to_owned());
    let environments = languages.iter().map(|language| {
        let env = format!("bbbb-gungnir-language-{language}");
        (format!("language/{language}"), env)
    });
    for (layout, env) in [base, measure, profile].into_iter().chain(environments) {
        write(
            dir,
            &format!("{layout}/closure"),
            format!("/nix/store/{env}\n"),
        );
        symlink(
            format!("/nix/store/{env}/bin"),
            dir.join(layout).join("bin"),
        )
        .unwrap();
    }
    let mut toolchains = Toolchains::load(dir).unwrap();
    for language in languages {
        toolchains.assume_version(language, "1.0");
    }
    toolchains
}
