//! The host a comparison ran on: what it is, and how busy it was.
use crate::provenance::cpu_model;
use eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use std::{fmt, fs, sync::OnceLock};

/// Which machine timed a comparison: its CPU model and its host id. Timings compare only within
/// one machine: an experiment's `[machine]` pins the one its measurements must come from, and
/// without a pin it is the machine looking at them. The core, the governor, the kernel and the
/// load vary between measurements on one machine and are kept as provenance only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub cpu: Option<String>,
    /// `/etc/machine-id`, when the host has one.
    pub host: Option<String>,
}

impl Identity {
    /// This machine's, probed once per process.
    pub fn here() -> Result<&'static Self> {
        static HERE: OnceLock<Identity> = OnceLock::new();
        if let Some(here) = HERE.get() {
            return Ok(here);
        }
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").wrap_err("reading /proc/cpuinfo")?;
        let here = Self {
            cpu: cpu_model(&cpuinfo),
            host: fs::read_to_string("/etc/machine-id")
                .ok()
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty()),
        };
        Ok(HERE.get_or_init(|| here))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    /// The CPU model, which some architectures do not report.
    pub cpu: Option<String>,
    /// The host id, `/etc/machine-id`. A record made before gungnir kept it names none.
    #[serde(default)]
    pub host: Option<String>,
    /// How many CPUs gungnir could run on, as `nproc` counts them.
    pub cpus: usize,
    /// The CPU every run was pinned to.
    pub core: u32,
    /// The frequency governor of that CPU, when the kernel exposes one.
    pub governor: Option<String>,
    pub kernel: String,
    /// `kernel.perf_event_paranoid`, when readable. Above 2 the kernel refuses hyperfine's
    /// user-space counters, so no run has an instruction count.
    pub perf_event_paranoid: Option<i32>,
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
        let Identity { cpu, host } = Identity::here()?.clone();
        Ok(Self {
            cpu,
            host,
            cpus: std::thread::available_parallelism()
                .wrap_err("counting the CPUs")?
                .get(),
            core,
            governor: fs::read_to_string(governor)
                .ok()
                .map(|governor| governor.trim().to_owned()),
            kernel: read("/proc/sys/kernel/osrelease")?.trim().to_owned(),
            perf_event_paranoid: perf_event_paranoid(),
            load: Load {
                start,
                end: load_average()?,
            },
        })
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cpu = self.cpu.as_deref().unwrap_or("an unreported CPU");
        let host = self.host.as_deref().unwrap_or("no host id");
        write!(f, "{cpu} ({host})")
    }
}

impl Machine {
    /// Whether the comparison ran on the machine `identity` names. A record that names no host
    /// matches on its CPU model alone.
    pub fn is(&self, identity: &Identity) -> bool {
        self.cpu == identity.cpu && (self.host.is_none() || self.host == identity.host)
    }
}

/// `kernel.perf_event_paranoid` now, when readable.
pub fn perf_event_paranoid() -> Option<i32> {
    fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")
        .ok()?
        .trim()
        .parse()
        .ok()
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

    fn machine(cpu: Option<&str>, host: Option<&str>) -> Machine {
        Machine {
            cpu: cpu.map(str::to_owned),
            host: host.map(str::to_owned),
            cpus: 16,
            core: 15,
            governor: Some("performance".into()),
            kernel: "7.3.0".into(),
            perf_event_paranoid: Some(1),
            load: Load {
                start: [0.5, 0.5, 0.5],
                end: [0.5, 0.5, 0.5],
            },
        }
    }

    #[test]
    fn a_machine_is_its_cpu_model_and_host_id() {
        let vm = Identity {
            cpu: Some("AMD Ryzen 7 5800U".into()),
            host: Some("4a7c1e90".into()),
        };
        assert!(machine(Some("AMD Ryzen 7 5800U"), Some("4a7c1e90")).is(&vm));
        let mut busier = machine(Some("AMD Ryzen 7 5800U"), Some("4a7c1e90"));
        busier.core = 3;
        busier.governor = None;
        busier.kernel = "7.4.0".into();
        busier.load.end = [9.0, 9.0, 9.0];
        assert!(busier.is(&vm));

        assert!(!machine(Some("AMD EPYC 9655"), Some("4a7c1e90")).is(&vm));
        assert!(!machine(Some("AMD Ryzen 7 5800U"), Some("0b5e7a11")).is(&vm));
        assert!(!machine(None, Some("4a7c1e90")).is(&vm));
        let bench = Identity {
            cpu: Some("AMD EPYC 9655".into()),
            host: Some("0b5e7a11".into()),
        };
        assert!(!machine(Some("AMD Ryzen 7 5800U"), Some("4a7c1e90")).is(&bench));
    }

    #[test]
    fn a_record_without_a_host_id_matches_on_its_cpu_model() {
        let here = Identity {
            cpu: Some("AMD Ryzen 7 5800U".into()),
            host: Some("4a7c1e90".into()),
        };
        assert!(machine(Some("AMD Ryzen 7 5800U"), None).is(&here));
        assert!(!machine(Some("AMD EPYC 9655"), None).is(&here));
    }

    #[test]
    fn here_is_probed_once() {
        let here = Identity::here().unwrap();
        assert!(std::ptr::eq(here, Identity::here().unwrap()));
    }

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
