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
