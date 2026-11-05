//! hyperfine, the timer: the command line that times one program, and its JSON export.
use super::{Protocol, Sample, WORKLOAD};
use eyre::{Result, WrapErr, ensure};
use serde::Deserialize;
use std::{ffi::OsString, num::NonZeroU32};

/// The export schema this parser reads, which hyperfine 2 writes.
const SCHEMA: u32 = 2;
/// Where a timed run's stdout goes. A pipe, like the one the output check reads, so a
/// program cannot tell a timed run from a checked one by looking at its stdout, as it could
/// with `/dev/null`.
const OUTPUT: &str = "--output=pipe";
/// Every metric hyperfine can collect, the user-space instruction count among them. hyperfine
/// leaves out a counter the kernel refuses (`perf_event_paranoid` above 2) instead of failing.
const METRICS: &str = "--metrics=all";

/// Times `run` without a shell, with the workload on stdin, `protocol.warmup` untimed runs and
/// `protocol.runs` timed ones. hyperfine prints only its export, on stdout.
pub(super) fn argv(run: &[String], protocol: &Protocol) -> Vec<OsString> {
    let warmup = protocol.warmup.to_string();
    let runs = protocol.runs.to_string();
    let command = quote(run);
    [
        "hyperfine",
        "-N",
        "--style=none",
        METRICS,
        "--input",
        WORKLOAD,
        OUTPUT,
        "--warmup",
        &warmup,
        "--runs",
        &runs,
        "--export-json",
        "/dev/stdout",
        "--",
        &command,
    ]
    .map(OsString::from)
    .into()
}

/// hyperfine splits a command without a shell into words as a POSIX shell would, so each
/// argument is single-quoted.
fn quote(args: &[String]) -> String {
    let words: Vec<String> = args
        .iter()
        .map(|arg| format!("'{}'", arg.replace('\'', r"'\''")))
        .collect();
    words.join(" ")
}

/// The timed runs in hyperfine's export of one command, which must hold exactly `runs`
/// runs that exited 0.
pub(super) fn samples(export: &[u8], runs: NonZeroU32) -> Result<Vec<Sample>> {
    let raw: RawExport = serde_json::from_slice(export).wrap_err("parsing hyperfine's export")?;
    ensure!(
        raw.schema_version == SCHEMA,
        "hyperfine's export has schema version {}, and gungnir reads version {SCHEMA}",
        raw.schema_version
    );
    let [result] = <[RawResult; 1]>::try_from(raw.results)
        .map_err(|results| eyre::eyre!("hyperfine exported {} commands, not 1", results.len()))?;
    ensure!(
        result.measurements.len() == runs.get() as usize,
        "hyperfine exported {} runs, not {runs}",
        result.measurements.len()
    );
    result
        .measurements
        .into_iter()
        .enumerate()
        .map(|(index, run)| {
            run.sample()
                .wrap_err_with(|| format!("hyperfine's run {index}"))
        })
        .collect()
}

#[derive(Deserialize)]
struct RawExport {
    schema_version: u32,
    results: Vec<RawResult>,
}

#[derive(Deserialize)]
struct RawResult {
    measurements: Vec<RawRun>,
}

#[derive(Deserialize)]
struct RawRun {
    time_wall_clock: RawQuantity,
    time_user: RawQuantity,
    time_system: RawQuantity,
    memory_peak_resident: RawQuantity,
    instructions: Option<RawQuantity>,
    exit_code: i32,
}

#[derive(Deserialize)]
struct RawQuantity {
    value: f64,
    unit: Option<String>,
}

impl RawRun {
    fn sample(self) -> Result<Sample> {
        ensure!(self.exit_code == 0, "exited {}", self.exit_code);
        Ok(Sample {
            wall: self
                .time_wall_clock
                .amount("time_wall_clock", Some("second"))?,
            user: self.time_user.amount("time_user", Some("second"))?,
            system: self.time_system.amount("time_system", Some("second"))?,
            peak_memory: self
                .memory_peak_resident
                .amount("memory_peak_resident", Some("byte"))? as u64,
            instructions: self
                .instructions
                .map(|count| count.amount("instructions", None))
                .transpose()?
                .map(|count| count as u64),
        })
    }
}

impl RawQuantity {
    /// The value, which must be finite, non-negative and in `unit`.
    fn amount(self, name: &str, unit: Option<&str>) -> Result<f64> {
        ensure!(
            self.unit.as_deref() == unit,
            "{name} is in {:?}, not {unit:?}",
            self.unit
        );
        ensure!(
            self.value.is_finite() && self.value >= 0.0,
            "{name} is {}",
            self.value
        );
        Ok(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two runs of `cat`, as hyperfine 2.0.0 exported them, cut to the fields gungnir reads
    /// plus a few it ignores.
    const EXPORT: &str = r#"{
      "schema_version": 2,
      "metadata": {"hyperfine_version": "2.0.0", "start_time": "2026-11-04T00:58:37Z"},
      "primary_metric": "time_wall_clock",
      "results": [{
        "command": "cat",
        "summary": {"time_wall_clock": {"unit": "second", "count": 2, "mean": 0.00487803}},
        "measurements": [
          {"time_wall_clock": {"value": 0.0072980200000000005, "unit": "second"},
           "time_cpu": {"value": 0.007083, "unit": "second"},
           "time_user": {"value": 0.0017649999999999999, "unit": "second"},
           "time_system": {"value": 0.005318, "unit": "second"},
           "memory_peak_resident": {"value": 6742016.0, "unit": "byte"},
           "cpu_cycles": {"value": 1107028},
           "instructions": {"value": 1124783},
           "exit_code": 0},
          {"time_wall_clock": {"value": 0.00245804, "unit": "second"},
           "time_cpu": {"value": 0.002336, "unit": "second"},
           "time_user": {"value": 0.00117, "unit": "second"},
           "time_system": {"value": 0.001166, "unit": "second"},
           "memory_peak_resident": {"value": 6742016.0, "unit": "byte"},
           "exit_code": 0}
        ]
      }]
    }"#;

    fn runs(count: u32) -> NonZeroU32 {
        NonZeroU32::new(count).unwrap()
    }

    #[test]
    fn each_run_becomes_a_sample() {
        let samples = samples(EXPORT.as_bytes(), runs(2)).unwrap();
        assert_eq!(
            samples,
            [
                Sample {
                    wall: 0.0072980200000000005,
                    user: 0.0017649999999999999,
                    system: 0.005318,
                    peak_memory: 6742016,
                    instructions: Some(1124783),
                },
                Sample {
                    wall: 0.00245804,
                    user: 0.00117,
                    system: 0.001166,
                    peak_memory: 6742016,
                    instructions: None,
                },
            ]
        );
    }

    #[test]
    fn an_export_that_is_not_what_was_asked_for_is_an_error() {
        let wrong = [
            (
                EXPORT.replace("\"schema_version\": 2", "\"schema_version\": 1"),
                "version 1",
            ),
            (
                EXPORT.replace(
                    "\"exit_code\": 0}\n        ]",
                    "\"exit_code\": 3}\n        ]",
                ),
                "exited 3",
            ),
            (
                EXPORT.replace(
                    "0.00245804, \"unit\": \"second\"",
                    "0.00245804, \"unit\": \"millisecond\"",
                ),
                "millisecond",
            ),
            (
                EXPORT.replace("\"value\": 0.00117", "\"value\": -0.00117"),
                "-0.00117",
            ),
            (EXPORT.replacen("\"time_user\"", "\"user\"", 1), "time_user"),
        ];
        for (export, needle) in wrong {
            let message = format!("{:#}", samples(export.as_bytes(), runs(2)).unwrap_err());
            assert!(message.contains(needle), "{needle} missing from: {message}");
        }
        let message = format!("{:#}", samples(EXPORT.as_bytes(), runs(3)).unwrap_err());
        assert!(message.contains("2 runs, not 3"), "{message}");
    }

    #[test]
    fn the_command_line_times_one_quoted_command_on_the_workload() {
        let protocol = Protocol {
            blocks: runs(4),
            runs: runs(7),
            warmup: 2,
            clock_mhz: None,
        };
        let run = ["./my solution".to_owned(), "it's".to_owned()];
        let argv: Vec<String> = argv(&run, &protocol)
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(
            argv,
            [
                "hyperfine",
                "-N",
                "--style=none",
                "--metrics=all",
                "--input",
                "/workload",
                "--output=pipe",
                "--warmup",
                "2",
                "--runs",
                "7",
                "--export-json",
                "/dev/stdout",
                "--",
                r"'./my solution' 'it'\''s'",
            ]
        );
    }
}
