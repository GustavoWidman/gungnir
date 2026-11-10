//! The host a comparison ran on: what it is, and how busy it was.
use crate::{provenance::cpu_model, store::Digest};
use eyre::{Result, WrapErr, bail, eyre};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, fs, num::NonZeroU32, sync::OnceLock};

/// Which machine timed a comparison: its CPU model and its host id. Timings compare only within
/// one machine: an experiment's `[machine]` pins the one its measurements must come from, and
/// without a pin it is the machine looking at them. The core, the governor, the kernel and the
/// load vary between measurements on one machine and are kept as provenance only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub cpu: Option<String>,
    /// The [`host_id`] of `/etc/machine-id`, when the host has one.
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
                .filter(|id| !id.is_empty())
                .map(|id| host_id(&id)),
        };
        Ok(HERE.get_or_init(|| here))
    }
}

/// The host id gungnir records and pins for a machine: 128 bits of a SHA-256 digest of its
/// machine id under a prefix of gungnir's own, in hex. systemd asks applications to keep the
/// machine id itself confidential, so it never reaches a record or an experiment file.
pub fn host_id(machine_id: &str) -> String {
    let digest = Digest::of_bytes(format!("gungnir host id\n{machine_id}").as_bytes());
    digest.to_string()[..32].to_owned()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    /// The CPU model, which some architectures do not report.
    pub cpu: Option<String>,
    /// The [`host_id`] of `/etc/machine-id`, which a record may lack.
    #[serde(default)]
    pub host: Option<String>,
    /// How many CPUs gungnir could run on, as `nproc` counts them.
    pub cpus: usize,
    /// The CPU every run was pinned to.
    pub core: u32,
    /// Whether the kernel kept the scheduler off that CPU (`isolcpus`).
    #[serde(default)]
    pub isolated: bool,
    /// The frequency governor of that CPU, when the kernel exposes one.
    pub governor: Option<String>,
    /// The range cpufreq let that CPU's clock move in when timing ended, when the kernel
    /// exposes one.
    #[serde(default)]
    pub clock: Option<Clock>,
    /// Whether the CPU could boost above its base clock, when the kernel says.
    #[serde(default)]
    pub turbo: Option<bool>,
    pub kernel: String,
    /// `kernel.perf_event_paranoid`, when readable. Above 2 the kernel refuses hyperfine's
    /// user-space counters, so no run has an instruction count.
    pub perf_event_paranoid: Option<i32>,
    /// The 1, 5 and 15 minute load averages when timing started and when it ended.
    pub load: Load,
}

/// The range cpufreq lets one CPU's clock move in: its `scaling_min_freq` and
/// `scaling_max_freq`, in kHz. A machine holds a CPU at one frequency by setting both to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    pub min_khz: u32,
    pub max_khz: u32,
}

impl Clock {
    /// `cpu`'s range now, when the kernel exposes one.
    pub fn of(cpu: u32) -> Option<Self> {
        let read = |name: &str| -> Option<u32> {
            let path = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/{name}");
            fs::read_to_string(path).ok()?.trim().parse().ok()
        };
        Some(Self {
            min_khz: read("scaling_min_freq")?,
            max_khz: read("scaling_max_freq")?,
        })
    }
}

impl fmt::Display for Clock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mhz = |khz: u32| f64::from(khz) / 1000.0;
        write!(f, "{} to {} MHz", mhz(self.min_khz), mhz(self.max_khz))
    }
}

/// Refuses unless every one of `cores` runs at `clock_mhz`, with cpufreq's `scaling_min_freq`
/// and `scaling_max_freq` both at that frequency, when a protocol holds the clock there.
pub fn check_clock(
    clock_mhz: Option<NonZeroU32>,
    cores: impl IntoIterator<Item = u32>,
) -> Result<()> {
    let Some(mhz) = clock_mhz else {
        return Ok(());
    };
    for core in cores {
        if let Some(why) = off_clock(mhz, core, Clock::of(core)) {
            bail!("{why}; set both to {} kHz", u64::from(mhz.get()) * 1000);
        }
    }
    Ok(())
}

/// Why `core`, whose cpufreq range is `clock`, does not run at `mhz`, when it does not.
fn off_clock(mhz: NonZeroU32, core: u32, clock: Option<Clock>) -> Option<String> {
    let khz = u64::from(mhz.get()) * 1000;
    let Some(clock) = clock else {
        return Some(format!(
            "measuring holds the clock at {mhz} MHz, and CPU {core} reports no cpufreq \
             scaling_min_freq and scaling_max_freq"
        ));
    };
    let held = u64::from(clock.min_khz) == khz && u64::from(clock.max_khz) == khz;
    (!held).then(|| {
        format!(
            "measuring holds the clock at {mhz} MHz, and CPU {core} reports scaling_min_freq {} \
             kHz and scaling_max_freq {} kHz",
            clock.min_khz, clock.max_khz
        )
    })
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
            isolated: isolated_cpus().contains(&core),
            governor: fs::read_to_string(governor)
                .ok()
                .map(|governor| governor.trim().to_owned()),
            clock: Clock::of(core),
            turbo: turbo(),
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
    /// matches on its CPU model alone, and one that names a raw machine id matches its
    /// [`host_id`].
    pub fn is(&self, identity: &Identity) -> bool {
        let host = match (&self.host, &identity.host) {
            (None, _) => true,
            (Some(recorded), Some(host)) => recorded == host || host_id(recorded) == *host,
            (Some(_), None) => false,
        };
        self.cpu == identity.cpu && host
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

/// Whether CPUs may boost above their base clock: intel_pstate's `no_turbo`, or else the
/// generic `cpufreq/boost`, and `None` when neither is readable.
fn turbo() -> Option<bool> {
    let read = |path: &str| {
        fs::read_to_string(path)
            .ok()
            .map(|value| value.trim() == "1")
    };
    read("/sys/devices/system/cpu/intel_pstate/no_turbo")
        .map(|no_turbo| !no_turbo)
        .or_else(|| read("/sys/devices/system/cpu/cpufreq/boost"))
}

/// The CPUs the kernel keeps the scheduler off (`isolcpus`), in order, or none when it reports
/// none.
pub fn isolated_cpus() -> Vec<u32> {
    fs::read_to_string("/sys/devices/system/cpu/isolated")
        .ok()
        .and_then(|list| parse_cpu_list(&list))
        .unwrap_or_default()
}

/// The CPUs that share `cpu`'s physical core, `cpu` among them, or only `cpu` when the kernel
/// does not say.
pub fn thread_siblings(cpu: u32) -> Vec<u32> {
    let path = format!("/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list");
    fs::read_to_string(path)
        .ok()
        .and_then(|list| parse_cpu_list(&list))
        .filter(|siblings| siblings.contains(&cpu))
        .unwrap_or_else(|| vec![cpu])
}

/// One CPU on each physical core that holds a CPU of `isolated` that gungnir may run on
/// (`allowed`): the first such CPU, in order. `siblings` names the CPUs that share a CPU's core.
/// The other threads of each core get no lane and stay idle, so no two lanes share a core.
pub fn first_threads(
    isolated: &[u32],
    allowed: &[u32],
    siblings: impl Fn(u32) -> Vec<u32>,
) -> Vec<u32> {
    let mut taken = BTreeSet::new();
    let mut first = Vec::new();
    for &cpu in isolated {
        if allowed.contains(&cpu) && !taken.contains(&cpu) {
            taken.extend(siblings(cpu));
            taken.insert(cpu);
            first.push(cpu);
        }
    }
    first
}

/// A kernel CPU list such as `14-15` or `0,2-4`, or `None` when it is malformed.
pub fn parse_cpu_list(text: &str) -> Option<Vec<u32>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(Vec::new());
    }
    let mut cpus = Vec::new();
    for part in text.split(',') {
        match part.split_once('-') {
            Some((first, last)) => {
                let (first, last) = (first.parse::<u32>().ok()?, last.parse::<u32>().ok()?);
                if first > last {
                    return None;
                }
                cpus.extend(first..=last);
            }
            None => cpus.push(part.parse().ok()?),
        }
    }
    Some(cpus)
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
            isolated: false,
            governor: Some("performance".into()),
            clock: None,
            turbo: Some(true),
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
        busier.isolated = true;
        busier.governor = None;
        busier.turbo = Some(false);
        busier.kernel = "7.4.0".into();
        busier.load.end = [9.0, 9.0, 9.0];
        assert!(busier.is(&vm));

        assert!(!machine(Some("AMD EPYC 9655"), Some("4a7c1e90")).is(&vm));
        assert!(!machine(Some("AMD Ryzen 7 5800U"), Some("b2d9f6e3")).is(&vm));
        assert!(!machine(None, Some("4a7c1e90")).is(&vm));
        let bench = Identity {
            cpu: Some("AMD EPYC 9655".into()),
            host: Some("b2d9f6e3".into()),
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
    fn a_host_id_is_a_digest_of_the_machine_id() {
        let id = host_id("6f3e0c1b9a8d4e2f7c5b1a0d9e8f7a6b");
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(id, host_id("6f3e0c1b9a8d4e2f7c5b1a0d9e8f7a6b"));
        assert_ne!(id, host_id("6f3e0c1b9a8d4e2f7c5b1a0d9e8f7a6c"));
        assert_ne!(id, "6f3e0c1b9a8d4e2f7c5b1a0d9e8f7a6b");
    }

    #[test]
    fn a_record_naming_the_raw_machine_id_matches_its_host_id() {
        let raw = "6f3e0c1b9a8d4e2f7c5b1a0d9e8f7a6b";
        let here = Identity {
            cpu: Some("AMD Ryzen 7 5800U".into()),
            host: Some(host_id(raw)),
        };
        assert!(machine(Some("AMD Ryzen 7 5800U"), Some(raw)).is(&here));
        assert!(!machine(Some("AMD Ryzen 7 5800U"), Some("4a7c1e90")).is(&here));
    }

    #[test]
    fn here_is_probed_once() {
        let here = Identity::here().unwrap();
        assert!(std::ptr::eq(here, Identity::here().unwrap()));
    }

    #[test]
    fn a_cpu_list_names_single_cpus_and_ranges() {
        assert_eq!(parse_cpu_list("14-15\n"), Some(vec![14, 15]));
        assert_eq!(parse_cpu_list("0,2-4,7"), Some(vec![0, 2, 3, 4, 7]));
        assert_eq!(parse_cpu_list("\n"), Some(vec![]));
        assert_eq!(parse_cpu_list("4-2"), None);
        assert_eq!(parse_cpu_list("a-b"), None);
    }

    #[test]
    fn each_isolated_physical_core_gets_its_first_thread() {
        let all: Vec<u32> = (0..32).collect();
        let pairs = |cpu: u32| {
            if cpu < 16 {
                vec![cpu & !1, cpu | 1]
            } else {
                vec![cpu]
            }
        };
        let isolated: Vec<u32> = (8..16).collect();
        assert_eq!(first_threads(&isolated, &all, pairs), [8, 10, 12, 14]);

        let alone = |cpu: u32| vec![cpu];
        assert_eq!(first_threads(&[14, 15], &all, alone), [14, 15]);
        assert_eq!(first_threads(&[9, 10, 11], &all, pairs), [9, 10]);
        let without_8: Vec<u32> = (9..32).collect();
        assert_eq!(first_threads(&isolated, &without_8, pairs), [9, 10, 12, 14]);
        assert_eq!(first_threads(&isolated, &[0, 1, 2], pairs), [] as [u32; 0]);
        assert_eq!(first_threads(&[], &all, pairs), [] as [u32; 0]);
    }

    #[test]
    fn every_cpu_shares_its_core_with_itself() {
        let cpus = u32::try_from(std::thread::available_parallelism().unwrap().get()).unwrap();
        for cpu in 0..cpus {
            assert!(thread_siblings(cpu).contains(&cpu), "CPU {cpu}");
        }
        assert_eq!(thread_siblings(1 << 20), [1 << 20]);
    }

    #[test]
    fn a_held_clock_is_a_range_of_one_frequency() {
        let mhz = NonZeroU32::new(3000).unwrap();
        let held = Clock {
            min_khz: 3_000_000,
            max_khz: 3_000_000,
        };
        assert_eq!(off_clock(mhz, 8, Some(held)), None);

        let free = Clock {
            min_khz: 800_000,
            max_khz: 5_800_000,
        };
        let why = off_clock(mhz, 8, Some(free)).unwrap();
        for needle in ["CPU 8", "3000 MHz", "800000 kHz", "5800000 kHz"] {
            assert!(why.contains(needle), "{needle:?} missing from: {why}");
        }
        let higher = Clock {
            min_khz: 3_200_000,
            max_khz: 3_200_000,
        };
        assert!(off_clock(mhz, 10, Some(higher)).unwrap().contains("CPU 10"));
        let why = off_clock(mhz, 12, None).unwrap();
        assert!(why.contains("CPU 12 reports no cpufreq"), "{why}");

        assert!(check_clock(None, [0]).is_ok());
        assert_eq!(free.to_string(), "800 to 5800 MHz");
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
