//! The host a comparison ran on: what it is, and how busy it was.
use crate::provenance::cpu_model;
use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    /// The CPU model, which some architectures do not report.
    pub cpu: Option<String>,
    /// How many CPUs gungnir could run on, as `nproc` counts them.
    pub cpus: usize,
    /// The CPU every run was pinned to.
    pub core: u32,
    /// The frequency governor of that CPU, when the kernel exposes one.
    pub governor: Option<String>,
    pub kernel: String,
    /// The 1, 5 and 15 minute load averages when timing started and when it ended.
    pub load: Load,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Load {
    pub start: [f64; 3],
    pub end: [f64; 3],
}

impl Machine {
    /// Probes the host after timing pinned to `core`, which started at load average `start`.
    pub(super) fn probe(core: u32, start: [f64; 3]) -> Result<Self> {
        let read =
            |path: &str| fs::read_to_string(path).wrap_err_with(|| format!("reading {path}"));
        let governor = format!("/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_governor");
        Ok(Self {
            cpu: cpu_model(&read("/proc/cpuinfo")?),
            cpus: std::thread::available_parallelism()
                .wrap_err("counting the CPUs")?
                .get(),
            core,
            governor: fs::read_to_string(governor)
                .ok()
                .map(|governor| governor.trim().to_owned()),
            kernel: read("/proc/sys/kernel/osrelease")?.trim().to_owned(),
            load: Load {
                start,
                end: load_average()?,
            },
        })
    }
}

/// The 1, 5 and 15 minute load averages now.
pub(super) fn load_average() -> Result<[f64; 3]> {
    let text = fs::read_to_string("/proc/loadavg").wrap_err("reading /proc/loadavg")?;
    parse_load(&text).ok_or_else(|| eyre!("/proc/loadavg reads {text:?}"))
}

fn parse_load(text: &str) -> Option<[f64; 3]> {
    let mut fields = text.split_whitespace().map(str::parse::<f64>);
    let mut next = || fields.next()?.ok();
    Some([next()?, next()?, next()?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_load_average_is_the_first_three_fields() {
        assert_eq!(
            parse_load("6.65 4.26 3.28 2/1234 5678\n"),
            Some([6.65, 4.26, 3.28])
        );
        assert_eq!(parse_load("6.65 4.26\n"), None);
        assert_eq!(parse_load("high 4.26 3.28 1/2 3\n"), None);
    }
}
