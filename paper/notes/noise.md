# Measurement noise floor

A/A tests time a reference against itself with `gungnir bench`: 6 ABBA blocks of 3 runs each, pinned to one core. The 95% interval of the ratio is the noise floor a real comparison has to clear.

## Desktop, 2026-11-04

Machine: i9-13900K, 64 GB. The run was pinned to CPU 14 (P-core 7), with CPUs 14 and 15 under `isolcpus`, `nohz_full` and `rcu_nocbs`. The machine was otherwise idle (load average about 1). Raw log: `data/desktop-aa-2026-11-04.log`.

Bench mode ran with the performance governor and turbo off. IRQs were not moved off the measurement core during this run.

| Scenario, language | Workload median | Ratio, 95% interval |
| --- | --- | --- |
| tiny-vm, c | 33–45 ms | 0.977–1.044 (widest), 0.990–1.003 (narrowest) |
| json-path, rust | 168–256 ms | 0.997–1.001 |
| word-freq, python | 1.07–1.48 s | 0.996–1.004 |
| route-fuel, go | 247–268 ms | 0.998–1.002 |
| regex-lite, javascript | 229–298 ms | 0.985–1.008 (widest) |

Instruction-count ratios were 1.000, with one at 0.999 (JavaScript's JIT).

With bench mode off (powersave governor, turbo on), tiny-vm in C ran about 1.8 times faster, at the turbo clock. Its intervals widened to 0.935–1.032.

On the shared VM the noise floor was 5–15%. On the desktop in bench mode it is under 0.5% for workloads above about 150 ms, and a few percent for workloads near 35 ms.

## Desktop, four lanes, 2026-11-05

CPUs 8-15 (P-cores 4 to 7) are isolated with `isolcpus`, `nohz_full` and `rcu_nocbs`. Four A/A comparisons run at once, one on the first thread of each isolated P-core (CPUs 8, 10, 12 and 14), with the sibling threads idle. Eight references were timed in two rounds of four per configuration, at the default protocol (6 ABBA blocks of 3 runs) unless noted. Raw output: `data/desktop-aa-lanes-2026-11-05/`, including a clock and package-temperature log sampled every 5 seconds.

| Configuration | Median 95% half-width | 90th percentile | Largest | Reference run time |
|---|---|---|---|---|
| Base clock (3.0 GHz, turbo off) | 0.70% | 2.35% | 6.40% | 1× |
| 5.5 GHz pinned (minimum = maximum, turbo on) | 0.95% | 4.15% | 5.85% | 0.56× (1.62× to 1.91× faster) |
| 5.5 GHz pinned, 4 blocks of 2 runs | 1.45% | 4.20% | 15.55% | 0.56× |

At 5.5 GHz every sample of every measurement core read 5,500,000 kHz, and the package peaked at 67 °C. Every A/A ratio lies within 3% of 1.00. Running four lanes raises the noise mostly on memory-heavy workloads (json-path records, ledger, word-freq with many distinct words), which share the last-level cache and memory bandwidth across lanes; compute-bound workloads stay between 0.1% and 0.6%. The shorter protocol widens the intervals further and produces one 15% outlier.

Held at 5.5 GHz on all four measurement cores, with candidate builds running on the other cores, the machine reset itself after about 90 minutes of continuous measuring: the journal stops without a shutdown sequence, no kernel panic or machine-check error was recorded (`kernel.panic` is 0, so a panic would have halted the machine rather than restarted it), and it booted again 37 seconds later. The measurements therefore use four lanes at the P-cores' base clock of 3.0 GHz, pinned with minimum and maximum frequency equal, and the default protocol, the configuration of the first row, whose median half-width is 0.70%.
