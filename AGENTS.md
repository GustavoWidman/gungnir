# AGENTS.md

## Project standard

gungnir is a Rust CLI that benchmarks AI coding agents. It drives each vendor's agent CLI headless inside a sandbox, grades the program the agent wrote against hidden tests, times it, and exports tables for statistical analysis.

`docs/architecture.md` is the design contract. Code that disagrees with it is a bug in one of the two, and the fix updates both. Read it before you change a data shape, the results layout, or a phase.

This file is guidance for contributors and coding agents. It describes the repository as it is today. Update it when a change makes a section wrong.

## Repository map

- `Cargo.toml` and `Cargo.lock` hold the package metadata and the locked dependencies.
- `flake.nix` and `flake.lock` define the dev shell, the crane package wrapped with `GUNGNIR_TOOLCHAINS`, the toolchains package, and the clippy, rustfmt and package checks. An overlay adds `rust-nightly`, the newest nightly in the fenix lock with rustc, cargo and rust-std, which the rust language builds with; `nix flake update fenix` moves it.
- `nix/toolchains.nix` builds the sandbox environments (`base/`, `measure/`, `profile/`, `language/<id>/`, `harness/<name>/`, each with `bin/` and a `closure` file) from `gungnir.toml` and `languages/*/language.toml`. A `language.toml` names nixpkgs attributes, plus the packages nixpkgs lacks, which `toolchains.nix` lists under `local`.
- `nix/typescript-bun.nix` builds `typescript-bun`, a `tsc` that reads Bun's type declarations from the store, so the TypeScript sandbox type-checks offline.
- `rust-toolchain.toml` pins the nightly toolchain and its components.
- `.envrc` loads the flake for direnv users.
- `gungnir.toml` is the catalog. It lists the sandbox base packages, the measurement tools (`[measure]`), the profiling tools (`[profile]`), the harness packages, the backends with their credentials, egress hosts, optional quota source and whether their model requests go through the relay, and the models with their efforts, optional output ceilings (`max-output-tokens`) and dated prices.
- `languages/<id>/` is a toolchain profile. The loader requires `language.toml`, `preamble.md` and a `template/` directory. The `version` command in `language.toml` prints the version that `{version}` in the preamble states. The `[profile]` table of `language.toml` says how `gungnir profile` builds and samples the language and what its flame graphs can show.
- `scenarios/<id>/` is a task. The loader requires `scenario.toml`, `spec.md`, `examples/`, `hidden/`, `workloads.sha256` and `reference/<language>/`. `gen/gen.py` writes the cases and, on demand, the gitignored `workloads/`.
- `experiments/<name>.toml` is a matrix to run. `pilot.toml` is the pilot study and `smoke.toml` runs each pilot model once.
- `docs/architecture.md` is the design contract.
- `paper/` holds the manuscript and the research notes behind it. `paper/analysis/` is a Python package, run from `paper/` inside the dev shell, that reads a `gungnir report` and a `gungnir index` of the study experiments and the transcripts under `runs/`, and writes `paper/figures/*.svg` and `paper/data/*.json`, which `main.typ` reads for every number it states.
- `src/` is the `gungnir` crate, a library and a thin binary.

## Crate map

- `src/main.rs` parses the command line and calls `Cli::run`.
- `src/lib.rs` declares the modules in sorted order.
- `src/cli.rs` defines the clap commands. `plan` loads an experiment and prints its trial matrix. `check` grades every reference solution, printing its language's version, and regenerates missing workloads. Every command that renders prompts, and `check`, first probes the version of each language it uses. `run` runs an experiment's pending trials with progress logged to stderr through `tracing`, and fails while any trial is still pending; `run --dry-run` prints what it would withdraw and schedule and changes nothing. `grade` grades the committed runs that lack a current grade, and recounts in place the complexity of each current grade that this version of the counting or the lizard on `PATH` would count otherwise. `bench` times one workspace against a reference outside any experiment, or the reference against itself for an A/A test. `measure` times every selected passing trial that lacks a current measurement, one comparison per `--cores` CPU at a time, `machine` prints this machine's identity as the `[machine]` pin, and `profile` runs one trial under perf. `status` counts each system's trials by stage (pending, ran, passed, failed, and measured among the passing), the stale ones, and those whose current grade has a complexity `grade` would recount. `report` folds the committed records of one or more experiments into CSV tables in `--out`, `runs/report/` by default. `index` prints each language's gungnir index over one or more experiments and, with `--out`, writes it as CSV.
- `src/id.rs` holds the identifiers every module shares. The `slug!` macro makes the validated newtypes `LanguageId`, `ScenarioId`, `ModelId`, `BackendId`, `HarnessName`, `HarnessVersion`, `CaseId` and `WorkloadId`. `Rep`, `AttemptNo` and the rank-ordered `Effort` enum live there too.
- `src/catalog.rs` loads `gungnir.toml` into `Catalog`, `Backend`, `Endpoint`, `Credential`, `QuotaSource`, `Model`, with its optional output ceiling, and dated price tiers. An `Endpoint` is `https://` with a DNS host and no port, except for a backend that relays, whose endpoint may also be `http://`, name an IPv4 address and add a port. `Catalog::system` is the only constructor of `System`, which carries its model's output ceiling, and `System::id` derives the `SystemId`, which never includes it.
- `src/language.rs` loads `languages/` into `Language` profiles. `Language::prompt` renders the agent's prompt from a `Brief` and the `Version` the language's toolchain reported, and the loader refuses a preamble that never says `{version}`.
- `src/scenario.rs` loads `scenarios/` into `Scenario` values, with digests of the brief, the hidden suite, the workloads and each reference.
- `src/experiment.rs` loads an experiment against the repository that holds it, with its harness pins, optional `[machine]` pin and optional `[measure]` timing protocol, which `Experiment::protocol` serves (`Protocol::DEFAULT` without one), validates it, and expands it into `TrialKey` values with `Experiment::trials`. A `[[systems]]` entry may set its own `repetitions`, which `Experiment::repetitions_of` serves.
- `src/trial.rs` defines `Phase`, `TrialKey`, `ResultsDir`, `TrialDir` and `AttemptDir`. Paths under `results/<experiment>/` are named there and nowhere else. `ResultsDir::retract` withdraws a committed directory through the phase's scratch, so a crash never leaves one half deleted.
- `src/store.rs` and `src/store/` hold the file mechanics: `Digest`, atomic writes, `read_json`, the `PartialDir` commit protocol with `rewrite_record`, which replaces a committed record in place, `replace_dir`, which swaps an output directory in for the old one with its `manifest.json` written last, and the `Lock` used for phase locks.
- `src/provenance.rs` records one `Session` per invocation in `sessions/<id>.json`, with the harness versions it probed and the store paths of the toolchains it used.
- `src/quota.rs` reads a backend's `QuotaSource` from the host with the backend's `Key`, through `reqwest` over rustls, into quota `Window`s named by their length (`5h`). The parse of Z.ai's monitor is a pure function over the response bytes, tested on a captured response in `tests/fixtures/quota/`, and every error quotes the body redacted.
- `src/secret.rs` holds `Secret<T>`, whose `Debug` prints nothing, `EnvValue` for sandbox environments, and redaction of secret values from bytes and files.
- `src/vault.rs` reads each backend's credential into a `Key` (the credential and the endpoint, what a driver launches with, which for a backend that relays is the relay's URL and no credential, the attempt's `Relay`, and the exact strings to redact) and stages launch files into an isolated `Home` under `$XDG_RUNTIME_DIR/gungnir/`, writing a refreshed Codex login back to its master under a lock.
- `src/stop.rs` is the two-stage interruption: the first Ctrl-C drains, the second kills in-flight sandboxes.
- `src/toolchains.rs` loads `GUNGNIR_TOOLCHAINS` into `Closure` sets and `bin` directories, refusing any closure that contains a flake source path. `Toolchains::probe` asks a tool what it is in an offline sandbox holding only its environment over the base. `Toolchains::probe_versions` runs each named language's `version` command that way, once per invocation, and `Toolchains::version` serves the result. A language without an environment is an error only when an experiment uses it.
- `src/sandbox/` defines `Spec`, `Network`, `Finished`, `Fault` and the object-safe `Backend` trait, with the bubblewrap backend. It is the only place untrusted code is spawned.
- `src/egress/` is the host-side CONNECT proxy that tunnels only allowlisted hosts on port 443 and logs every connection. `relay.rs` is the relay it also serves for a backend that relays: plain HTTP requests to `Relay::URL`, forwarded to the backend's endpoint with its key, retried while the endpoint refuses them before any body under the pure `Backoff` in `retry.rs`, then streamed back unbuffered, with one log record per try.
- `src/harness/` holds the `Driver` registry (`DRIVERS`), the `Harness` handle, the Claude Code and Codex drivers, the harness-neutral `Event` and `TokenUsage`, and the pure `Fold` into an `Ending`. The Claude Code driver sets `CLAUDE_CODE_MAX_OUTPUT_TOKENS` to the model's output ceiling when the catalog states one. The Codex driver launches with a ChatGPT login for OpenAI's own endpoint, with an API key and a model provider in `CODEX_HOME/config.toml` for any other endpoint that serves the Responses API, and with a provider that names no key at the relay of a backend that relays. Captured transcripts for its tests live in `tests/fixtures/harness/`.
- `src/grade/` judges a workspace: an offline build, every hidden case in its own sandbox compared on the host, lizard complexity, and the `GradeRecord` with its `GradeInputs`. `complexity.rs` runs lizard with its `outside` extension, so each file's top-level code counts as one more function, `*global*`, stores the version of the counting and the version lizard reports with the complexity, and holds `Counting`, which tells whether a stored complexity is what measuring the workspace would give now. `phase.rs` is the grade phase, `grade_runs`, which commits a grade for every committed run without a current one and never commits an environment fault, and rewrites through `rewrite_record` the complexity of every current grade that `Counting` reads as counted otherwise, building and running nothing.
- `src/workload.rs` regenerates a scenario's workloads in a sandbox and verifies them against `workloads.sha256`.
- `src/measure/` times a candidate against the reference in its language. `Program` is a workspace built offline; `compare` checks each workload's output under GNU time (`rusage.rs`, peak memory) and then runs hyperfine (`hyperfine.rs`) in ABBA blocks pinned to one core. `timing.rs` and `stats.rs` hold the samples and the pure statistics: the wall-time ratio with its t interval and the instruction ratio. `record.rs` holds `MeasureRecord` and `MeasureInputs`, whose `Protocol` is part of the inputs. `machine.rs` holds the `Machine` a measurement ran on and its `Identity`, the CPU model and host id that a current measurement shares with this machine, the cpufreq `Clock` range of a core, which `check_clock` compares with the protocol's `clock_mhz`, and `first_threads`, which picks the first isolated thread of each physical core by `thread_siblings` for the default measurement cores. `lanes.rs` holds `Cores`, the CPUs measuring pins its comparisons to, and `on_cores`, which starts jobs in order on the lowest free core, never two on one core, and starts no more once the first Ctrl-C drains. `phase.rs` is the measure phase, `measure_trials`, one comparison per core at a time in those lanes, with each reference built once and shared by the lanes. `profile.rs` runs one trial's candidate and reference under perf and folds the samples into flame graphs.
- `src/run/` is the run phase. `Task` is one trial as an agent sees it, with a `Brief` and never a `Scenario`. `records.rs` holds `RunInputs`, `RunRecord`, `DiscardedAttempt` and the stored form of the agent's outcome, with the sample policy: `Counted::of` commits only a session that completed or was refused, and gives every other ending a `Reason`. A `DiscardedAttempt` is the run phase's own bookkeeping, never data. `workspace.rs` scaffolds the agent's starting workspace and keeps it, without build output or secrets, for the run directory. `attempt.rs` makes one attempt: an isolated home, an egress proxy, the sandbox, the transcript streamed, redacted and folded live, and then the run committed or the attempt discarded with its reason. `schedule.rs` runs pending trials in one lane per backend behind the pure `Gate` in `gate.rs` and, for a backend with a quota source, the pure `Meter` in `meter.rs`, which reads the quota through `quota::Reader` and holds the lane while a window is spent. `review.rs` reviews each committed run before scheduling: `review` reads it, re-deriving the outcome for a newer parser and finding a run that is no sample, and `apply` rewrites the record or withdraws the run with its `grade/` and `measure/`. `run::run` is the phase: pins, session, references, staleness, reviews, then the lanes. `run::preview` is the dry run: pins, staleness and reviews, without a lock and without applying anything. Tests play agents through the `Agent` backend in `fixture.rs`.
- `src/lifecycle.rs` derives a trial's `Stage` and its first stale phase from the committed records, comparing each record's inputs with `Current`, which takes the timing protocol from the experiment. `Lifecycle::current_grade` serves the grade when neither it nor its run is stale.
- `src/report/` is the report of model samples. `report::report` reads every trial of the given experiments through `Lifecycle` and replaces the output directory, writing `manifest.json` last. Attempts that committed nothing never reach it. `tables.rs` holds one row struct per table and `Tables::add`, the pure fold of one trial's records into rows. `csv.rs` holds the `row!` macro, whose fields name a row's columns, and the encoder. `price.rs` prices each usage sample from the dated price table in exact picodollars.
- `src/index/` is the gungnir index, one score per language for how well agents do in it against the average language for the same model and scenario. `mod.rs` holds the definition, `Dimension` and `Weights`. `score.rs` holds `score`, the pure function from `Sample` rows to `Scores`: each dimension's cell value, each language's advantage, the weighted fold into every index, and the seeded two-stage bootstrap behind every interval. `read.rs` reads the samples through `report::read`, keeping the `Generation`s graded completely enough, and `write.rs` writes `scores.csv` and `generations.csv` with the report's `row!` encoder under a manifest through `store::replace_dir`.
- `src/testing.rs` holds the fixture writers the unit tests share. It compiles only for tests.

## How `gungnir plan` loads an experiment

1. `Experiment::load` finds the repository root, which is the nearest directory above the experiment file that has a `gungnir.toml`.
2. `Catalog::load`, `Language::load_all` and `Scenario::load_all` load the catalog and every language and scenario. One invalid entry fails the whole load.
3. The experiment picks its scenarios and languages, turns each `[[systems]]` entry into one `System` per effort, and checks the pins, the concurrency keys and the references.
4. `cli::Plan` prints one row per model with its efforts and any repetitions its `[[systems]]` entry sets for itself, then the scenarios, languages, the experiment's repetitions and the trial count.

## How `gungnir run` runs an experiment

1. `run::run` takes the run lock, sweeps `.scratch/run/`, and opens the `Vault`, which reads every credential and sweeps the isolated homes a crashed run left.
2. It asks each harness CLI for `--version` in an offline sandbox, refuses a version other than the pin, and writes the session.
3. It judges every requested reference into `references/` and refuses unless all pass.
4. `triage` reads each trial's `Lifecycle`, refuses stale committed runs by name, and reviews every committed run without writing anything. `settle` then applies the reviews: it rewrites the records an older parser folded, and withdraws every run that is no sample, a rate limit the old parser missed or an ending such as a crash or a timeout, so its trial is pending again.
5. `schedule` starts the pending trials in one lane per backend behind its `Gate`, and `Rig::attempt` makes each attempt. A lane whose backend has a quota source reads it before it starts anything once its reading is two minutes old, and after a rate limit with no reset time, and holds while a window is at or past `--quota-hold`. Its readings count toward `--quota-ceiling` as attempts' quota reports do. A discarded or failed attempt puts its trial back into the lane as `retry` says, a rate limit at the front and anything else at the back, until the trial reaches `--max-attempts`.

## How `gungnir measure` measures an experiment

1. `measure_trials` refuses any machine but the one the experiment's `[machine]` pins and, when the experiment's protocol holds the clock, any measurement core whose cpufreq range is not that one frequency. Then it takes the measure lock. Unless `--allow-busy`, it also takes the run and grade locks and refuses a one-minute load average of 1.0 or more. It sweeps `.scratch/measure/` and writes the session.
2. It reads the `Lifecycle` of each trial that `--rep` and `--system` select, under the experiment's `Protocol`, and keeps the passing trials whose measurement is missing or stale.
3. It ensures those scenarios' workloads, then measures the trials in one lane per CPU of `--cores`: `on_cores` starts them in order on free cores, the reference of each scenario and language is built once, the first time a lane needs it, and shared, the candidate is built from `run/final/`, `compare` times both pinned to the lane's core, and `measure/` is committed through `PartialDir`. A fault or error is logged and leaves the trial for the next invocation. The first Ctrl-C lets the comparisons in progress finish and starts no more.

## How `gungnir report` writes a report

1. `cli` loads every experiment and starts a `Session` for the report's own provenance. `report::report` refuses an experiment given twice.
2. `read` derives each trial's `Lifecycle` and keeps the trials whose committed run is a sample.
3. `Tables::add` folds each trial into rows, pricing every usage sample with the experiment's catalog at the report's price date.
4. `store::replace_dir` writes the tables into `<out>.partial`, writes `manifest.json` last, and swaps the directory in for `<out>`.

## How `gungnir index` scores the languages

1. `cli` parses `--weight` into `Weights`, loads every experiment and probes its languages' versions. `index::read` refuses an experiment given twice.
2. `read` counts each generation's cells from `Experiment::trials`, reads the trials through `report::read`, keeps those with a current grade as `Sample`s, and drops the samples of every generation with less than `--complete` of its cells graded.
3. `score` takes the fastest reference median of each scenario's workload among the samples, folds the samples into cells, and computes every score, then repeats on 4000 resamples from a fixed seed for the intervals.
4. `cli` prints one row per language in index order. With `--out`, `index::write` replaces the directory with `scores.csv`, `generations.csv` and `manifest.json` through `store::replace_dir`.

## Toolchain sets

An experiment's results are tied to the toolchain set it ran with. A committed run records the store paths of its base, language and harness environments, and its prompt states the version its language's toolchain reported, so against another set its runs read as stale. A toolchain upgrade starts a new experiment, and an experiment that already has runs keeps its own set, kept alive by a GC root.

Each experiment in the repository and the set to run its commands with:

| Experiments | `GUNGNIR_TOOLCHAINS` |
|---|---|
| `pilot`, `claude-ext`, `openai-ext`: `run`, `grade` | `/nix/store/76fa1jkl6f76s2w73mhhlhfcm6dlgw4c-gungnir-toolchains`, with only the python, rust and zig languages |
| `pilot`, `claude-ext`, `openai-ext`: `measure`, `profile`, `status`, `report`, `index` | `/nix/store/nqyqmxzf4d76l96j5s3zyi8fvwj011xf-gungnir-toolchains`, the same base, language and harness environments with GNU time in `measure/` and a `profile/` |
| `study-openai`, `study-glm`, `study-claude`, `study-nvidia`, `study-deepseek`, `study-gemini`, `study-ornith`, `study-astra`, `study-fable`: every command | `/nix/store/fszs07fs8l58n1cp03fzq0p75v0f8jy4-gungnir-toolchains`, all eight languages |

Set it inside the dev shell, which otherwise sets the variable to the current set, for example `GUNGNIR_TOOLCHAINS=/nix/store/fszs07fs8l58n1cp03fzq0p75v0f8jy4-gungnir-toolchains gungnir status experiments/study-glm.toml`. A measurement records the `measure/` environment it timed with, so the pilot's measurements read as stale against the first set, whose `measure/` lacks GNU time.

## Invariants to keep

- Parse external data at the boundary. Each loader deserializes into a private `Raw*` type with `deny_unknown_fields` and converts it into the domain type, so code after the loader trusts its input.
- Name the file, the key and the value in every load error. Wrap the conversion with the file name and put the key and value in the inner message.
- Construct a `System` only through `Catalog::system`. A `System` is always a model its backend serves at an effort the model offers.
- Give the run phase a `Brief`, never a `Scenario`. A `Brief` holds the spec and the visible examples in memory and holds no path, so no hidden case is reachable from it.
- Measure only trials whose grade passed, one comparison per measurement core at a time, with every run of a comparison pinned to its core. A measurement and its stage depend on its `MeasureInputs` and on the machine it ran on, by CPU model and host id, never on the core it was pinned to or the load. An experiment's `[machine]` pins that machine, as `[pins]` pins harness versions: `measure` refuses any other, and a measurement from it is current wherever the records are read. Without a pin, the machine reading them must be the one that measured.
- Take the timing protocol only from the experiment. `Current::of` reads `Experiment::protocol`, so `measure`, `status`, `report` and `index` judge a measurement against one protocol, and `measure` has no flag that could disagree with it.
- Commit a phase directory only through `PartialDir`. It writes `record.json` last and renames `<phase>.partial/` into place, so a phase directory is complete or absent.
- Build results paths only through `ResultsDir` and `TrialDir`.
- Never store a trial's state. Derive it from the committed records.
- Keep a model's output ceiling out of `RunInputs`, so raising it makes no committed run stale. A committed run whose response reached the ceiling holds Claude Code's resume turn in its transcript.
- Keep complexity out of `GradeInputs`. A complexity carries the version of its counting and the version of the lizard that counted it, and `grade` recounts one counted otherwise in place, so a change to the counting never rebuilds or reruns a trial. Raise `VERSION` in `src/grade/complexity.rs` whenever measuring would count the same sources differently.
- State a language's version only through `{version}`. The prompt then names the version the toolchain reports, and the same toolchain renders the same prompt.

## Rust style

- Keep modules narrow, with one owner for each piece of domain knowledge. Split a file when it starts to own two unrelated things.
- Prefer typed values to strings and parallel booleans: a newtype per identifier, an enum per closed set, a map keyed by id.
- Keep public surfaces small. Make fields private and add accessors when the type carries an invariant.
- Return `eyre::Result` and add context with `wrap_err_with`. Do not add `unwrap` or `expect` to runtime paths.
- Write a comment only for a reason the code cannot show. Keep doc comments on public items to intent and invariants.
- Test what a caller observes, using `tempfile` fixture trees from `src/testing.rs`.
- Add a dependency with `cargo add`, and only when code uses it.

## Validate a change

Run every command from the repository root, inside the dev shell.

```sh
nix develop --command cargo fmt --check
nix develop --command cargo clippy --all-targets -- -D warnings
nix develop --command cargo test
nix flake check
```

In a git worktree that builds next to other checkouts, give cargo a local target directory and no compiler wrapper, for example `nix develop --command env CARGO_TARGET_DIR=$PWD/target RUSTC_WRAPPER= cargo test`.

The git provenance test needs `git` on `PATH` and skips itself when `git` is missing.

## Commits

- Make each commit one logical change that builds and passes the three cargo checks on its own.
- Use a Conventional Commit subject with a scope, such as `feat(store): add atomic partial-directory commit`, and a body of one or two sentences.
- Do not push, and do not rewrite history you did not write.
