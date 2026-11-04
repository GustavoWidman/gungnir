# AGENTS.md

## Project standard

gungnir is a Rust CLI that benchmarks AI coding agents. It drives each vendor's agent CLI headless inside a sandbox, grades the program the agent wrote against hidden tests, times it, and exports tables for statistical analysis.

`docs/architecture.md` is the design contract. Code that disagrees with it is a bug in one of the two, and the fix updates both. Read it before you change a data shape, the results layout, or a phase.

This file is guidance for contributors and coding agents. It describes the repository as it is today. Update it when a change makes a section wrong.

## Repository map

- `Cargo.toml` and `Cargo.lock` hold the package metadata and the locked dependencies.
- `flake.nix` and `flake.lock` define the dev shell, the crane package wrapped with `GUNGNIR_TOOLCHAINS`, the toolchains package, and the clippy, rustfmt and package checks.
- `nix/toolchains.nix` builds the sandbox environments (`base/`, `language/<id>/`, `harness/<name>/`, each with `bin/` and a `closure` file) from `gungnir.toml` and `languages/*/language.toml`.
- `rust-toolchain.toml` pins the nightly toolchain and its components.
- `.envrc` loads the flake for direnv users.
- `gungnir.toml` is the catalog. It lists the sandbox base packages, the harness packages, the backends with their credentials and egress hosts, and the models with their efforts and dated prices.
- `languages/<id>/` is a toolchain profile. The loader requires `language.toml`, `preamble.md` and a `template/` directory.
- `scenarios/<id>/` is a task. The loader requires `scenario.toml`, `spec.md`, `examples/`, `hidden/`, `workloads.sha256` and `reference/<language>/`. `gen/gen.py` writes the cases and, on demand, the gitignored `workloads/`.
- `experiments/<name>.toml` is a matrix to run. `pilot.toml` is the pilot study and `smoke.toml` runs each pilot model once.
- `docs/architecture.md` is the design contract.
- `paper/` holds the manuscript and the research notes behind it.
- `src/` is the `gungnir` crate, a library and a thin binary.

## Crate map

- `src/main.rs` parses the command line and calls `Cli::run`.
- `src/lib.rs` declares the modules in sorted order.
- `src/cli.rs` defines the clap commands. `plan` loads an experiment and prints its trial matrix. `check` grades every reference solution and regenerates missing workloads. `run` runs an experiment's pending trials with progress logged to stderr through `tracing`, and fails while any trial is still pending; `run --dry-run` prints what it would withdraw and schedule and changes nothing. `grade` grades the committed runs that lack a current grade. `status` counts each system's trials by stage (pending, ran, passed, failed) and the stale ones. `report` folds the committed records of one or more experiments into CSV tables in `--out`, `runs/report/` by default.
- `src/id.rs` holds the identifiers every module shares. The `slug!` macro makes the validated newtypes `LanguageId`, `ScenarioId`, `ModelId`, `BackendId`, `HarnessName`, `HarnessVersion`, `CaseId` and `WorkloadId`. `Rep`, `AttemptNo` and the rank-ordered `Effort` enum live there too.
- `src/catalog.rs` loads `gungnir.toml` into `Catalog`, `Backend`, `Credential`, `Model` and dated price tiers. `Catalog::system` is the only constructor of `System`, and `System::id` derives the `SystemId`.
- `src/language.rs` loads `languages/` into `Language` profiles. `Language::prompt` renders the agent's prompt from a `Brief`.
- `src/scenario.rs` loads `scenarios/` into `Scenario` values, with digests of the brief, the hidden suite, the workloads and each reference.
- `src/experiment.rs` loads an experiment against the repository that holds it, validates it, and expands it into `TrialKey` values with `Experiment::trials`.
- `src/trial.rs` defines `Phase`, `TrialKey`, `ResultsDir`, `TrialDir` and `AttemptDir`. Paths under `results/<experiment>/` are named there and nowhere else. `ResultsDir::retract` withdraws a committed directory through the phase's scratch, so a crash never leaves one half deleted.
- `src/store.rs` and `src/store/` hold the file mechanics: `Digest`, atomic writes, `read_json`, the `PartialDir` commit protocol and the `Lock` used for phase locks.
- `src/provenance.rs` records one `Session` per invocation in `sessions/<id>.json`, with the harness versions it probed and the store paths of the toolchains it used.
- `src/secret.rs` holds `Secret<T>`, whose `Debug` prints nothing, `EnvValue` for sandbox environments, and redaction of secret values from bytes and files.
- `src/vault.rs` reads each backend's credential into a `Key` (the material a driver launches with, the endpoint, and the exact strings to redact) and stages launch files into an isolated `Home` under `$XDG_RUNTIME_DIR/gungnir/`, writing a refreshed Codex login back to its master under a lock.
- `src/stop.rs` is the two-stage interruption: the first Ctrl-C drains, the second kills in-flight sandboxes.
- `src/toolchains.rs` loads `GUNGNIR_TOOLCHAINS` into `Closure` sets and `bin` directories, refusing any closure that contains a flake source path.
- `src/sandbox/` defines `Spec`, `Network`, `Finished`, `Fault` and the object-safe `Backend` trait, with the bubblewrap backend. It is the only place untrusted code is spawned.
- `src/egress/` is the host-side CONNECT proxy that tunnels only allowlisted hosts on port 443 and logs every connection.
- `src/harness/` holds the `Driver` registry (`DRIVERS`), the `Harness` handle, the Claude Code and Codex drivers, the harness-neutral `Event` and `TokenUsage`, and the pure `Fold` into an `Ending`. Captured transcripts for its tests live in `tests/fixtures/harness/`.
- `src/grade/` judges a workspace: an offline build, every hidden case in its own sandbox compared on the host, lizard complexity, and the `GradeRecord` with its `GradeInputs`. `phase.rs` is the grade phase, `grade_runs`, which commits a grade for every committed run without a current one and never commits an environment fault.
- `src/workload.rs` regenerates a scenario's workloads in a sandbox and verifies them against `workloads.sha256`.
- `src/run/` is the run phase. `Task` is one trial as an agent sees it, with a `Brief` and never a `Scenario`. `records.rs` holds `RunInputs`, `RunRecord`, `DiscardedAttempt` and the stored form of the agent's outcome, with the sample policy: `Counted::of` commits only a session that completed or was refused, and gives every other ending a `Reason`. A `DiscardedAttempt` is the run phase's own bookkeeping, never data. `workspace.rs` scaffolds the agent's starting workspace and keeps it, without build output or secrets, for the run directory. `attempt.rs` makes one attempt: an isolated home, an egress proxy, the sandbox, the transcript streamed, redacted and folded live, and then the run committed or the attempt discarded with its reason. `schedule.rs` runs pending trials in one lane per backend behind the pure `Gate` in `gate.rs`. `review.rs` reviews each committed run before scheduling: `review` reads it, re-deriving the outcome for a newer parser and finding a run that is no sample, and `apply` rewrites the record or withdraws the run with its `grade/` and `measure/`. `run::run` is the phase: pins, session, references, staleness, reviews, then the lanes. `run::preview` is the dry run: pins, staleness and reviews, without a lock and without applying anything. Tests play agents through the `Agent` backend in `fixture.rs`.
- `src/lifecycle.rs` derives a trial's `Stage` and its first stale phase from the committed records, comparing each record's inputs with `Current`.
- `src/report/` is the report of model samples. `report::report` reads every trial of the given experiments through `Lifecycle` and replaces the output directory, writing `manifest.json` last. Attempts that committed nothing never reach it. `tables.rs` holds one row struct per table and `Tables::add`, the pure fold of one trial's records into rows. `csv.rs` holds the `row!` macro, whose fields name a row's columns, and the encoder. `price.rs` prices each usage sample from the dated price table in exact picodollars.
- `src/testing.rs` holds the fixture writers the unit tests share. It compiles only for tests.

The contract also specifies a part that is not in the crate yet: the measure phase over trials. Give it its own module and update this map when it lands.

## How `gungnir plan` loads an experiment

1. `Experiment::load` finds the repository root, which is the nearest directory above the experiment file that has a `gungnir.toml`.
2. `Catalog::load`, `Language::load_all` and `Scenario::load_all` load the catalog and every language and scenario. One invalid entry fails the whole load.
3. The experiment picks its scenarios and languages, turns each `[[systems]]` entry into one `System` per effort, and checks the pins, the concurrency keys and the references.
4. `cli::Plan` prints one row per model with its efforts, then the scenarios, languages, repetitions and trial count.

## How `gungnir run` runs an experiment

1. `run::run` takes the run lock, sweeps `.scratch/run/`, and opens the `Vault`, which reads every credential and sweeps the isolated homes a crashed run left.
2. It asks each harness CLI for `--version` in an offline sandbox, refuses a version other than the pin, and writes the session.
3. It judges every requested reference into `references/` and refuses unless all pass.
4. `triage` reads each trial's `Lifecycle`, refuses stale committed runs by name, and reviews every committed run without writing anything. `settle` then applies the reviews: it rewrites the records an older parser folded, and withdraws every run that is no sample, a rate limit the old parser missed or an ending such as a crash or a timeout, so its trial is pending again.
5. `schedule` starts the pending trials in one lane per backend behind its `Gate`, and `Rig::attempt` makes each attempt. A discarded or failed attempt puts its trial back into the lane as `retry` says, a rate limit at the front and anything else at the back, until the trial reaches `--max-attempts`.

## How `gungnir report` writes a report

1. `cli` loads every experiment and starts a `Session` for the report's own provenance. `report::report` refuses an experiment given twice.
2. `read` derives each trial's `Lifecycle` and keeps the trials whose committed run is a sample.
3. `Tables::add` folds each trial into rows, pricing every usage sample with the experiment's catalog at the report's price date.
4. `publish` writes the tables into `<out>.partial`, writes `manifest.json` last, and swaps the directory in for `<out>`.

## Invariants to keep

- Parse external data at the boundary. Each loader deserializes into a private `Raw*` type with `deny_unknown_fields` and converts it into the domain type, so code after the loader trusts its input.
- Name the file, the key and the value in every load error. Wrap the conversion with the file name and put the key and value in the inner message.
- Construct a `System` only through `Catalog::system`. A `System` is always a model its backend serves at an effort the model offers.
- Give the run phase a `Brief`, never a `Scenario`. A `Brief` holds the spec and the visible examples in memory and holds no path, so no hidden case is reachable from it.
- Commit a phase directory only through `PartialDir`. It writes `record.json` last and renames `<phase>.partial/` into place, so a phase directory is complete or absent.
- Build results paths only through `ResultsDir` and `TrialDir`.
- Never store a trial's state. Derive it from the committed records.

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
