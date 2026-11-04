# gungnir architecture

gungnir runs a benchmark of AI coding agents. It drives each vendor's own CLI headless inside a sandbox, grades the program the agent wrote against hidden tests, times it, measures its complexity, and exports tidy tables for statistical analysis. This document is the design contract. Code that disagrees with it is a bug in one of the two, and the fix updates both.

## Vocabulary

- **Harness.** A vendor CLI agent driven headless: Claude Code, Codex, later Gemini CLI.
- **Backend.** One account behind one endpoint, used through one harness. `anthropic` is the Claude subscription through Claude Code, `openai` is the ChatGPT subscription through Codex, `zai` is the GLM coding plan through Claude Code. Concurrency, rate limits, credentials and the egress allowlist all belong to a backend, never to a harness.
- **System.** Harness (with its pinned version), backend, model and reasoning effort. Every claim in the paper is about a system.
- **Scenario.** A language-neutral task: a spec, visible examples, hidden cases, performance workloads, and one reference solution per supported language.
- **Language.** A toolchain profile: packages, project template, build steps, run command, the command that prints its version, source globs and the lizard language name.
- **Trial.** One system, scenario, language and repetition. Its identity is derived from those four, so every phase can find and resume it.
- **Attempt.** One launch of the agent for a trial. A trial commits exactly one attempt, and only a sample: an agent session that ended on its own, which the CLI reported complete or the model declined. Every other attempt is discarded with its reason. Discarded and interrupted attempts stay on disk only as the run phase's bookkeeping, which keeps retries and attempt numbering crash-safe, and are never data.

## Repository layout

```text
gungnir.toml                  catalog: sandbox base packages, harness packages, backends, models, prices
languages/<id>/
  language.toml               packages, build, run, version, sources, ignore, lizard, env
  preamble.md                 per-language prompt preamble; {build}, {run} and {version} placeholders
  template/                   files scaffolded into every workspace
scenarios/<id>/
  scenario.toml               title, compare mode, case limits
  spec.md                     the language-neutral task
  examples/NAME.in NAME.out   visible to the agent
  hidden/NAME.in NAME.out     never mounted into a sandbox that runs agent code
  workloads.sha256            the SHA-256 of every workload, in sha256sum format
  workloads/NAME.in           inputs for timing, written on demand by gen/ and never committed
  gen/                        the seeded generator for hidden cases and workloads
  reference/<language>/       a complete reference solution per supported language
experiments/<name>.toml       the matrix to run
nix/toolchains.nix            sandbox environments derived from gungnir.toml and languages/
nix/typescript-bun.nix        tsc with Bun's type declarations, a package nixpkgs lacks
src/                          the gungnir crate
docs/architecture.md          this file
AGENTS.md                     repository map for contributors and coding agents
paper/                        the manuscript
```

Each catalog is one list in one place.

- Languages are the directory listing of `languages/`. Rust loads them at runtime and `nix/toolchains.nix` reads the same `language.toml` files, so nothing else names a language.
- Scenarios are the directory listing of `scenarios/`. The languages a scenario supports are the directories under its `reference/`.
- Models, backends, harness packages and prices live in `gungnir.toml`.
- Harness drivers are the `DRIVERS` registry in `src/harness/mod.rs`. `gungnir.toml` names a nix package for each harness, because nix cannot read Rust. Loading fails if the two key sets differ.

## The toolchain closure

`nix/toolchains.nix` builds one directory, exported to gungnir as `GUNGNIR_TOOLCHAINS` by the dev shell and by the wrapped package:

```text
$GUNGNIR_TOOLCHAINS/
  base/bin, base/closure                 sandbox base packages from gungnir.toml [sandbox]
  measure/bin, measure/closure           measurement tools from gungnir.toml [measure]
  language/<id>/bin, language/<id>/closure
  harness/<name>/bin, harness/<name>/closure
```

Each `closure` file is the `closureInfo` store-path list of that environment. A sandbox bind-mounts exactly the union of the closures it needs, read-only, and never the whole `/nix/store`. This matters because the flake's own source copy in the store contains `scenarios/*/hidden/`. A test asserts that no mounted closure contains a flake source path. gungnir never runs `nix` at runtime. Adding a language means re-entering the dev shell.

A language's packages are nixpkgs attributes, or packages nixpkgs lacks that `toolchains.nix` defines under `local`. Anything a toolchain reads at build time must reach the sandbox through that closure, never through the workspace. TypeScript is the example: `typescript-bun` is `tsc` wrapped to pass `--typeRoots` into a store path holding Bun's type declarations, pinned to the Bun version nixpkgs ships, so a TypeScript workspace holds no `node_modules` and its type check runs offline.

Each preamble states its language's version through `{version}`, and the loader refuses a preamble without it. The text comes from the toolchain. Every command that renders a prompt (`run`, `grade`, `status`, `report`) and `check` run the language's `version` command once per invocation, in an offline sandbox holding the language's environment over the base with the language's variables, and take the one line it prints. A prompt is a function of the preamble, the brief and the toolchain, so it never names a version other than the one the agent's compiler reports, and one toolchain always renders the same bytes. A command that fails or prints anything but one line is an error naming the language.

A toolchain directory needs only the environments of the languages an experiment uses. gungnir looks a language up, and probes its version, only when an experiment uses it, so a set built before the checkout gained a language still serves the experiments that ran on it.

An experiment's results are tied to the toolchain set it ran with. A committed run records the store paths of its environments, and its prompt digest covers the version its toolchain reported, so against another set the run is stale. A toolchain upgrade starts a new experiment, and an experiment with runs keeps its set alive with a GC root. The pilot, `claude-ext` and `openai-ext` ran with `/nix/store/76fa1jkl6f76s2w73mhhlhfcm6dlgw4c-gungnir-toolchains`, so every command on them sets `GUNGNIR_TOOLCHAINS` to that path.

## Data shapes

Identifiers are slugs (`[a-z0-9][a-z0-9.-]*`) behind one macro, one newtype per kind: `LanguageId`, `ScenarioId`, `ModelId`, `BackendId`, `CaseId`, `WorkloadId`. `Rep` and `AttemptNo` are positive integers. A `SystemId` is derived from a `System` and never parsed back.

`Effort` is a closed enum: `None`, `Minimal`, `Low`, `Medium`, `High`, `Xhigh`, `Max`, ordered by rank. The catalog lists the levels each model accepts. Each driver declares which levels its CLI flag can carry. A `System` is constructed only by `Catalog::systems`, which rejects an effort outside the intersection, so a `System` value is always runnable.

`TokenUsage` keeps disjoint buckets: `input`, `cache_read`, `cache_write` (the default cache), `cache_write_long` (Anthropic's one-hour cache), `output`, and `reasoning: Option<u64>` as a subset of `output`. Each driver normalizes its vendor's semantics at the parse boundary (Anthropic's `input_tokens` excludes cache reads, OpenAI's includes them). Usage is stored per sample at the finest grain the CLI reports, each sample tagged with the request and model when the CLI names them (Codex names neither). Totals are derived, never stored.

`Brief` is the agent-visible part of a scenario: the spec and the visible examples. The run phase receives a `Brief`, never a `Scenario`, so it cannot name a hidden path.

`Secret<T>` wraps every credential. Its `Debug` prints nothing. Secret values are redacted from transcripts, stderr logs and workspace snapshots before anything is committed, because an agent can print its own environment.

## Results directory

```text
results/<experiment>/
  experiment.json                         name and first-run provenance
  sessions/<id>.json                      one per invocation: git rev and dirty flag, gungnir version,
                                          kernel, CPU, probed harness versions, toolchain store paths
  .locks/{run,grade,measure}.lock         flock per phase; measure takes all three
  .scratch/                               per-phase scratch, swept at phase start
  references/<scenario>/<language>/       the reference's own grade record, the runtime baseline
  trials/<system>/<scenario>/<language>/r<k>/
    attempts/<n>/transcript.jsonl         raw CLI stdout, streamed as it arrives, secrets redacted
    attempts/<n>/stderr.log
    attempts/<n>/egress.jsonl             every connection the agent attempted
    attempts/<n>/discarded.json           present only for an attempt that is no sample, with why
    run/      record.json  prompt.md  final/      the committed attempt and its workspace
    grade/    record.json  build.log  cases/<case>.out
    measure/  record.json  hyperfine/<workload>.json
```

A report spans experiments, so it lives outside any results directory, in `runs/report/` unless told otherwise.

**Commit protocol.** A phase builds `<phase>.partial/` and publishes it with one directory rename, writing `record.json` last before the rename. A phase directory is therefore complete or absent. A crash leaves `<phase>.partial/`, which the next invocation of that phase sweeps under its lock. A phase withdraws a committed directory (a run that turned out to be no sample, with the grade and measurement built on it, or a stale reference or grade) by renaming it into its own `.scratch/` and deleting it there, so a crash leaves it committed or in scratch, which the next invocation sweeps. The one record rewritten in place is a reparsed `run/record.json`, atomically. An attempt directory that has neither a `discarded.json` nor a committed `run/record.json` naming it is an interrupted attempt. Its transcript is kept because it may have spent tokens.

**Lifecycle.** A trial's state is never stored. `Lifecycle::read` (`src/lifecycle.rs`) reads the committed records in phase order, stopping at the first phase that is not committed, and compares each record's inputs with `Current`, what each phase would use for that trial now. It returns:

```rust
enum Stage {
    Pending,
    Ran { run: RunRecord },
    Graded { run: RunRecord, grade: GradeRecord },
    Measured { run: RunRecord, grade: GradeRecord, measure: MeasureRecord },
}
struct Lifecycle { stage: Stage, stale: Option<Phase> }
```

The records are boxed in the code. A grade whose run is not committed, left behind when a run is withdrawn, does not count: the trial is pending.

Every record embeds the inputs it was made from (`RunInputs`, `GradeInputs`, `MeasureInputs`): digests of the prompt, initial workspace, hidden suite, workloads and reference, the closure store paths, the build argv, and the grader version. `RunInputs` holds the system id, the API model, the endpoint, the digests of the prompt and of the workspace the agent starts in, and the store paths of the base, language and harness environments. It also records the wall and idle budgets the attempt ran under, which `RunInputs::is_stale` leaves out: a budget only bounds how long gungnir waits for a session to end on its own, the only kind of session that is a sample, so raising one changes no sample already taken. `stale` names the first phase whose stored inputs differ from what it would use now. Editing a hidden case regrades exactly the affected trials. A stale run is never redone automatically: `run` refuses and names the trials, because rerunning spends quota and changes the sample. The parser version is not an input: a run record stores the version that folded its outcome, and an older one is reparsed rather than refused. A record also names the session that produced it.

**Writers.** Each phase writes only its own directories, so `grade` can run while `run` is still going. The one exception is a run that `run` withdraws as no sample: it withdraws the `grade/` and `measure/` built on it first, so that nothing derived from it outlives it. A phase lock stops two copies of the same phase. `measure` takes all three locks because it needs a quiet machine.

## Harness adapters

`src/harness/` owns everything vendor-specific. Its public surface is a `Harness` handle and the types the runner consumes. The `Driver` trait and every wire type are private to the module.

```rust
trait Driver: Sync {
    fn name(&self) -> &'static str;
    fn parser_version(&self) -> u32;
    fn granularity(&self) -> Granularity;          // usage per request or per turn
    fn accepts(&self, effort: Effort) -> bool;     // what the CLI flag can carry
    fn parse_version(&self, stdout: &str) -> Result<String>;
    fn launch(&self, req: &LaunchRequest<'_>) -> Result<Launch>;   // pure
    fn decode(&self, line: &str) -> Vec<Event>;                    // pure, one stdout line
    fn diagnose_exit(&self, stderr_tail: &str) -> Option<Event> { None }
    fn artifacts(&self) -> &'static [&'static str] { &[] }         // globs under the isolated home
    fn decode_artifact(&self, line: &str) -> Vec<Event> { vec![] } // pure, one artifact line
}

static DRIVERS: &[&dyn Driver] = &[&ClaudeCode, &Codex];

#[derive(Clone, Copy)]
pub struct Harness(&'static dyn Driver);           // obtainable only from DRIVERS
```

`Launch` is argv, environment (plain and secret values), files to place in the isolated home, and the prompt delivered on stdin. Stdin is always closed after the prompt, because Codex otherwise waits for more input.

`Event` is the harness-neutral vocabulary: `Init { model, tools }`, `Usage(sample)`, `SessionUsage { model, tokens }`, `Quota { window, utilization, resets_at, allowed }`, `RateLimited { resets_at }`, `Refused`, `Finished { turns, api_time, error }`, `Noise`, and `Unrecognised(kind)`. Unrecognised kinds are counted into the run record so format drift is visible. Claude Code reports each response's usage as of its first token, so its per-request samples undercount output; its final `result` line carries per-model session totals, and the fold adds one sample without a request id that tops the per-request samples up to those totals.

A pure `Fold` turns events into an `Ending`:

```rust
enum Ending {
    Finished(AgentOutcome),   // completed, refused, errored, crashed or killed
    RateLimited { resets_at: Option<DateTime<Utc>>, usage: Vec<UsageSample> },
}
```

Whether an ending is a sample is the run phase's policy, not the driver's. `Counted::of` (`src/run/records.rs`) makes a sample only of an outcome that is `Completed` or `Refused`, and gives every other ending a `Reason`: `RateLimited { resets_at }`, `Errored { message }`, `Crashed` (the CLI ended without its final report, an out-of-memory kill included), `TimedOut { budget }` (the wall or the idle budget), `Stopped` (the second Ctrl-C), or `Fault { message }` (the sandbox could not run the agent to its end).

The runner feeds stdout lines to `decode` and `Fold` live, so it can kill the process on a rate limit or after an idle timeout. The runner decides an attempt's ending by replaying the transcript it stored, redacted as it was written, so a live attempt and a later replay of it always agree. When the fold ends without the CLI's final report (`Crashed` or `Killed`), the runner hands the last 16 KiB of `stderr.log` to `diagnose_exit`, whose event, a rate limit for both drivers, joins the fold. Some CLIs report more in files they leave in the isolated home than on stdout. After the process exits, the runner copies every file matching one of the driver's `artifacts` patterns into the attempt directory, redacts it, and feeds its lines to `decode_artifact` and the same `Fold` before `finish`. Codex runs without `--ephemeral` for this reason: its session rollout under `CODEX_HOME/sessions/` carries the account's rate limits, which `exec --json` never prints, and each window becomes a `Quota` event named by its length (`5h`, `7d`), since which window Codex calls primary depends on the plan. The same decoders and fold replay any stored transcript with its artifacts. A run record stores the `parser_version` that produced its outcome. Before it schedules anything, `run` reviews every committed run (`src/run/review.rs`). When a driver's version has increased, it re-derives the outcome from the transcript without calling a model. Whenever the outcome, re-derived or stored, is no sample (a rate limit the old parser missed, or an ending the sample policy rejects, such as a run committed before the policy), the run is withdrawn: its attempt is discarded with the matching reason, the `measure/` and `grade/` built on it are withdrawn, then `run/` itself, and the trial is pending again. Reviewing only reads, and a separate step applies the reviews, so `run --dry-run` shows what `run` would withdraw without doing it.

Adding a harness is one file implementing `Driver`, one line in `DRIVERS`, and one `[harnesses.<name>]` table in `gungnir.toml`. Adding a backend or model for an existing harness (DeepSeek through Claude Code) is catalog data only.

## Sandbox backends

`src/sandbox/` is the only place gungnir spawns untrusted code. A `Spec` describes what a sandbox exposes, and a backend runs a command under it.

```rust
pub struct Spec {
    pub network: Network,
    pub closure: Closure,          // store paths to bind read-only
    pub mounts: Vec<Mount>,        // host path, guest path, writable
    pub path: Vec<PathBuf>,        // PATH inside: language, base, and the harness for agents
    pub env: Vec<(String, EnvValue)>,
    pub cwd: PathBuf,
    pub limits: Limits,            // wall time, memory, tasks, optional CPU pin
}

pub enum Network {
    Offline,
    Egress(EgressGrant),           // reachable only through the host-side egress proxy
}

pub trait Backend: Send + Sync {
    fn exec<'a>(&'a self, spec: &'a Spec, argv: &'a [OsString], io: Io, stop: &'a Stop)
        -> BoxFuture<'a, Result<Finished, Fault>>;
}
```

The first backend is bubblewrap. Its argv builder is a pure function with unit tests. It uses `--unshare-all`, `--die-with-parent`, `--new-session` and `--clearenv`, binds the closure read-only, provides a private `/tmp` and home, and wraps the process in `systemd-run --user --scope` for `MemoryMax`, `TasksMax` and OOM detection. `taskset` pins measurement runs. Guest paths are constants (`/work`, `/home/agent`), so nothing the agent sees reveals the trial, model or effort. A microVM backend implements the same trait later.

`Finished` covers every way the program can end, including timeouts and OOM kills. `Fault` means the sandbox itself could not run the command. The grader records a fault as an environment error, never as the model's failure.

**Egress.** Agents get no network of their own. `src/egress/` runs an HTTP CONNECT proxy on the host, one Unix socket per attempt, that allows only the backend's allowlisted hosts on port 443 and logs every connection (host, port, verdict, bytes each way, timestamps) to the attempt's `egress.jsonl`. Inside the sandbox, `socat` from the sandbox base bridges `127.0.0.1:<port>` to the bound socket, and the harness reads `HTTPS_PROXY`. The proxy tunnels TLS without inspecting it. Server-side tools (vendor web search) never touch the proxy, so each driver also disables them with CLI flags.

## Credentials

`src/vault.rs` owns credentials. A backend's `credential` in `gungnir.toml` points at a file under `~/.config/gungnir/`, never inside the repository.

- `env-token`: a file whose contents become one environment variable. Claude Code uses `CLAUDE_CODE_OAUTH_TOKEN` from `claude setup-token`.
- `api-key`: the same, plus the backend's endpoint. GLM uses `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL`.
- `codex-home`: a dedicated `CODEX_HOME` whose `auth.json` is copied into each attempt's isolated home. After the attempt, if the copy carries a newer `last_refresh`, the vault writes it back to the master atomically under `gungnir.lock` beside it, so parallel attempts, even from two gungnir processes, never fight over one file and the newest login wins. A copy that names another account or a `last_refresh` in the future is refused, because the agent can write to its home.

Every credential is read again as each attempt starts, so an attempt always gets the latest login. The vault also names the exact strings to redact: the token for the first two kinds, and each token inside `auth.json` (never the whole file) for the third, including tokens the CLI refreshed during the attempt.

Isolated homes live under `$XDG_RUNTIME_DIR/gungnir/<results digest>/`, never under the results directory. The directory is private to the run that holds the results directory's run lock, so that run sweeps whatever a crashed run left there. Each home and everything in it is readable by the user alone, sits beside the attempt's egress socket, and is deleted after the attempt, also when it fails.

## Phases

All phases are safe to rerun and safe to interrupt. The first Ctrl-C drains: nothing new starts and in-flight work finishes. The second kills in-flight sandboxes, and their trials stay where they were.

- **check.** Builds and grades every reference solution through the same `grade::judge` path that grades agents, and prints the version each language's toolchain reports beside its references. A scenario whose reference fails cannot be run.
- **run.** Expands the experiment into trials, verifies the harness versions against the experiment's pins (each CLI prints `--version` in an offline sandbox holding only its own environment), records them in the session, judges every requested reference through `grade::judge` into `references/` and refuses unless all pass (a current committed reference grade is reused), refuses and names every trial whose committed run is stale, reviews every committed run (re-deriving outcomes for older parser versions and withdrawing every run that is no sample), and schedules pending trials in lanes. Each attempt's workspace lives in `.scratch/run/` while the agent works. A lane per backend has its own concurrency limit and a `Gate` (`Open`, `Paused { until }`, `Probing`) whose transition is a pure function. A rate limit requeues the trial at the front and pauses only that lane: until the reset time the vendor named, plus a minute, or else for a backoff of 5 minutes that each rate-limited probe doubles up to 60. When a pause ends, one probe attempt runs alone, and the lane reopens when it ends without a rate limit. An attempt whose ending is a sample commits the trial's run. Any other attempt is discarded, `attempts/<n>/discarded.json` records its `Reason`, and its trial goes back into its lane until an attempt is a sample: a rate-limited trial at the front, since the account ended it, and any other behind the lane's other trials, so that they run between its attempts. An attempt that failed in gungnir itself goes behind the lane the same way, without a record. A stopped attempt leaves its trial pending, since the invocation is ending.
- **Dry run.** `run --dry-run` probes the pins and refuses stale runs as `run` does, reviews every committed run without applying a review, and prints the runs it would withdraw with their reasons and the trials each backend would schedule. It changes nothing, so it takes no lock and can run beside a live `run`. It writes no session and judges no reference, since judging one commits its grade, so a reference that does not pass is not caught.
- **Guard.** `run` can bound how much of each account one invocation spends: `--quota-ceiling BACKEND[:WINDOW]=FRACTION[@TIME]` stops a backend once the named quota window (any window when none is named) its latest attempt reported is at least that fraction used, counting only attempts that started at or after `TIME` when one is given (for a ceiling meant for the window after a reset), `--stop-at BACKEND=TIME` stops it at a time, and `--max-errors N` (default 3) stops it after that many attempts in a row errored, crashed or failed, so an expired login or an outage cannot turn every remaining trial into an error. A stopped backend starts nothing more; its running attempts finish and its other trials stay pending for the next invocation. `--max-attempts N` (default 5) leaves one trial pending once N of its attempts in the invocation were discarded or failed, rate limits aside, so a trial its agent can never finish does not run forever.
- **grade.** In a fresh offline sandbox, copies the committed workspace, builds it, feeds each hidden case on stdin from the host, compares stdout on the host, and runs lizard over the sources. Expected outputs never enter a sandbox that runs agent code. Failures are classified as build error, wrong answer, timeout, crash, out of memory, or environment fault. The phase grades every trial whose run is committed and whose grade is missing or stale, a few at a time, withdrawing a stale grade through scratch first. A judgement the sandbox could not make (an environment fault) says nothing about the agent, so it is reported and never committed, and the trial waits for the next invocation.
- **measure.** Serial, for passing trials only. Runs hyperfine inside an offline sandbox pinned to one core, alternating reference and candidate in ABBA blocks per workload, and stores hyperfine's JSON export.
- **report.** Reads the committed records of one or more experiments through `Lifecycle` and folds them into CSV tables of model samples only: `trials.csv`, one row per trial whose committed run is a sample (a committed run that is no sample, which `run` has not withdrawn yet, is left out); `quota.csv`, the first and last utilization of each quota window a run saw; `usage.csv`, one row per usage sample of a committed run; `cases.csv`; `functions.csv`, each function lizard found; `workloads.csv`, one row per measured workload; and `timings.csv`, one row per timed run. Each table's columns are the fields of one row struct, and every row names its trial by experiment, system, scenario, language and repetition. A cell is empty where a value is missing, and units are in column names, never in cells. Cost is computed at report time, in exact picodollars, from the dated price table at one date for the whole report: today unless `--prices` names another. A sample that is one request is priced at the tier its prompt reaches. A per-turn sample, as Codex reports, or the remainder of a session total that no request accounts for, as Claude Code reports, does not tell each request's prompt size, so it is priced at the base tier and flagged in `cost_base_tier_assumed` when a higher tier could apply. A token in a bucket the vendor lists no rate for, or served by a model the catalog does not list, leaves `cost_usd` empty and is counted in `unpriced_tokens`, because leaving it out would understate the cost by an unknown amount. Attempts that committed nothing are internal bookkeeping of the run phase and never appear in a report: no table, column or count says how many attempts a trial took. The report replaces its output directory as a whole, building it beside the old one and swapping it in, refuses a directory that is neither empty nor a report, and writes `manifest.json` last: the experiments, the sessions that wrote the records read, the report's own session with the gungnir version and git revision, the price date, and each table's row count.

## Decisions

- **One crate.** No part has a second consumer. Module privacy already hides the wire types.
- **Files, not SQLite.** Transcripts, workspaces and logs are files anyway. A database would be a second truth that can disagree with them after a crash, and it cannot commit a directory atomically.
- **Harness drivers are a trait behind one registry.** One file per harness. The trait holds only pure functions, so each driver is tested against captured transcripts with no process. Spawning, sandboxing, budgets, redaction and commits are written once in the runner.
- **Languages and scenarios are data.** Their templates and references are files, and nix must read the language list.
- **Backends, not harnesses, are the unit of concurrency and rate limiting.** Claude Code serves two accounts in the pilot.
- **Records carry their inputs.** Provenance and staleness come from the same field.

## Tradeoffs accepted

- We discard every attempt that is not a sample entirely, tokens included. A rate limit, an error, a crash, a timeout or a sandbox fault says nothing about the model, so every sample is one session that ended on its own. What discarded and interrupted attempts spent is not reported.
- We accept about a dozen small files per trial in exchange for single-writer phases, inspection with `ls` and `jq`, and no schema migrations.
- A CLI upgrade is a new system. Pins in the experiment make that deliberate.
- Lizard runs in grade rather than measure. Complexity is a static property that needs no quiet machine.
- CSV only. Python and R read it natively.

## Alternatives considered

The design keys lanes by backend, stores inputs in records, re-parses stored transcripts, and mounts only toolchain closures. It uses a driver registry, live line decoding, the partial-directory commit protocol, the `Brief` type and session provenance. Rejected shapes:

- An enum of harnesses with a `match` in every method. Adding a harness touched many arms.
- Mounting all of `/nix/store`. It exposes the flake source copy that holds hidden tests.
- Running `nix build` at the start of every command. Toolchains are built once with the dev shell instead.
- SQLite, an append-only event journal, a Cargo workspace, and a long-lived pipeline daemon.
