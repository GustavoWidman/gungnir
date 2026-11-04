//! GNU time, which reads a program's peak resident memory from the kernel (`ru_maxrss`) as the
//! program exits. hyperfine reports the same figure for each timed run, but the kernel counts
//! the spawning process's own peak into the child's, so hyperfine's figure never falls below
//! its footprint: 3.3 MiB for a Zig program whose peak GNU time reads as 0.9 MiB. Above that
//! floor the two agree to the KiB.
use super::RUSAGE;
use eyre::{Result, WrapErr};
use std::{ffi::OsString, fs, io::ErrorKind, path::Path};

const REPORT: &str = "peak";

/// Runs `run` under GNU time, which writes the program's peak resident set size in KiB to
/// [`REPORT`] under [`RUSAGE`] and exits as the program did, 128 plus the signal for a program
/// a signal ended.
pub(super) fn argv(run: &[String]) -> Vec<OsString> {
    let output = format!("--output={RUSAGE}/{REPORT}");
    ["time", "--quiet", "--format=%M", &output, "--"]
        .into_iter()
        .chain(run.iter().map(String::as_str))
        .map(OsString::from)
        .collect()
}

/// The peak resident memory in bytes that GNU time reported in `dir`. `None` when it reported
/// nothing readable, as when the sandbox killed it with the program.
pub(super) fn peak(dir: &Path) -> Result<Option<u64>> {
    let path = dir.join(REPORT);
    let report = match fs::read_to_string(&path) {
        Ok(report) => report,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    };
    Ok(parse(&report))
}

fn parse(report: &str) -> Option<u64> {
    let kib: u64 = report.lines().last()?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write;
    use tempfile::TempDir;

    #[test]
    fn the_program_runs_under_gnu_time_reporting_into_the_mount() {
        let argv: Vec<String> = argv(&["./solution".into(), "--fast".into()])
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(
            argv,
            [
                "time",
                "--quiet",
                "--format=%M",
                "--output=/rusage/peak",
                "--",
                "./solution",
                "--fast"
            ]
        );
    }

    #[test]
    fn the_report_is_kibibytes_and_a_missing_or_garbled_one_is_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(peak(dir.path()).unwrap(), None);
        write(dir.path(), REPORT, "49156\n");
        assert_eq!(peak(dir.path()).unwrap(), Some(49156 * 1024));
        for garbled in ["", "lots\n", "-3\n"] {
            write(dir.path(), REPORT, garbled);
            assert_eq!(peak(dir.path()).unwrap(), None, "{garbled:?}");
        }
    }
}
