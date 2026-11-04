# gungnir architecture

gungnir runs a benchmark of AI coding agents. It drives each vendor's own CLI headless inside a sandbox, grades the program the agent wrote against hidden tests, times it, measures its complexity, and exports tidy tables for statistical analysis. This document is the design contract. Code that disagrees with it is a bug in one of the two, and the fix updates both.

## Vocabulary

- **Harness.** A vendor CLI agent driven headless: Claude Code, Codex, later Gemini CLI.
- **Backend.** One account behind one endpoint, used through one harness. `anthropic` is the Claude subscription through Claude Code, `openai` is the ChatGPT subscription through Codex, `zai` is the GLM coding plan through Claude Code. Concurrency, rate limits, credentials and the egress allowlist all belong to a backend, never to a harness.
- **System.** Harness (with its pinned version), backend, model and reasoning effort. Every claim in the paper is about a system.
- **Scenario.** A language-neutral task: a spec, visible examples, hidden cases, performance workloads, and one reference solution per supported language.
- **Language.** A toolchain profile: packages, project template, build steps, run command, source globs and the lizard language name.
- **Trial.** One system, scenario, language and repetition. Its identity is derived from those four, so every phase can find and resume it.
- **Attempt.** One launch of the agent for a trial. A trial commits exactly one attempt. Rate-limited and interrupted attempts are kept as overhead, never as samples.

## Repository layout

```text
gungnir.toml                  catalog: sandbox base packages, harness packages, backends, models, prices
languages/<id>/
  language.toml               packages, build, run, sources, ignore, lizard, env
  preamble.md                 per-language prompt preamble; {build} and {run} placeholders
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
  language/<id>/bin, language/<id>/closure
  harness/<name>/bin, harness/<name>/closure
```

Each `closure` file is the `closureInfo` store-path list of that environment. A sandbox bind-mounts exactly the union of the closures it needs, read-only, and never the whole `/nix/store`. This matters because the flake's own source copy in the store contains `scenarios/*/hidden/`. A test asserts that no mounted closure contains a flake source path. gungnir never runs `nix` at runtime. Adding a language means re-entering the dev shell.

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
    attempts/<n>/discarded.json           present only for a rate-limited attempt
    run/      record.json  prompt.md  final/      the committed attempt and its workspace
    grade/    record.json  build.log  cases/<case>.out
    measure/  record.json  hyperfine/<workload>.json
  report/                                 replaced as a whole on each report
```

**Commit protocol.** A phase builds `<phase>.partial/` and publishes it with one directory rename, writing `record.json` last before the rename. A phase directory is therefore complete or absent. A crash leaves `<phase>.partial/`, which the next invocation of that phase sweeps under its lock. An attempt directory that has neither a `discarded.json` nor a committed `run/record.json` naming it is an interrupted attempt. Its transcript is kept because it may have spent tokens.

**Lifecycle.** A trial's state is never stored. `Trial::lifecycle` reads the committed records in phase order and returns:

```rust
enum Stage {
    Pending,
    Ran { run: RunRecord },
    Graded { run: RunRecord, grade: GradeRecord },
    Measured { run: RunRecord, grade: GradeRecord, measure: MeasureRecord },
}
struct Lifecycle { stage: Stage, stale: Option<Phase> }
```

Every record embeds the inputs it was made from (`RunInputs`, `GradeInputs`, `MeasureInputs`): digests of the prompt, initial workspace, hidden suite, workloads and reference, the closure store paths, the build argv, and the grader and parser versions. `stale` names the first phase whose stored inputs differ from what it would use now. Editing a hidden case regrades exactly the affected trials. A stale run is never redone automatically: `run` refuses and names the trials, because rerunning spends quota and changes the sample. A record also names the session that produced it.

**Writers.** Each phase writes only its own directories, so `grade` can run while `run` is still going. A phase lock stops two copies of the same phase. `measure` takes all three locks because it needs a quiet machine.

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
    Finished(AgentOutcome),   // a sample, whatever the agent achieved
    RateLimited { resets_at: Option<DateTime<Utc>>, usage: Vec<UsageSample> },
}
```

The runner feeds stdout lines to `decode` and `Fold` live, so it can kill the process on a rate limit or after an idle timeout. Some CLIs report more in files they leave in the isolated home than on stdout. After the process exits, the runner copies every file matching one of the driver's `artifacts` patterns into the attempt directory, redacts it, and feeds its lines to `decode_artifact` and the same `Fold` before `finish`. Codex runs without `--ephemeral` for this reason: its session rollout under `CODEX_HOME/sessions/` carries the account's rate limits, which `exec --json` never prints, and each window becomes a `Quota` event named by its length (`5h`, `7d`), since which window Codex calls primary depends on the plan. The same decoders and fold replay any stored transcript with its artifacts. A run record stores the `parser_version` that produced its outcome. When a driver's version increases, `run` re-derives every committed outcome from its transcript without calling a model. If the new parser finds a rate limit the old one missed, the attempt is demoted to discarded and the trial is pending again.

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

- **check.** Builds and grades every reference solution through the same `grade::judge` path that grades agents. A scenario whose reference fails cannot be run.
- **run.** Expands the experiment into trials, verifies the harness versions against the experiment's pins, re-derives outcomes for older parser versions, and schedules pending trials in lanes. A lane per backend has its own concurrency limit and a `Gate` (`Open`, `Paused { until }`, `Probing`) whose transition is a pure function. A rate limit requeues the trial at the front and pauses only that lane. When a pause ends, one probe attempt runs before the lane reopens.
- **grade.** In a fresh offline sandbox, copies the committed workspace, builds it, feeds each hidden case on stdin from the host, compares stdout on the host, and runs lizard over the sources. Expected outputs never enter a sandbox that runs agent code. Failures are classified as build error, wrong answer, timeout, crash, out of memory, or environment fault.
- **measure.** Serial, for passing trials only. Runs hyperfine inside an offline sandbox pinned to one core, alternating reference and candidate in ABBA blocks per workload, and stores hyperfine's JSON export.
- **report.** Folds current records into CSV tables: trials, attempts, usage samples, cases, workloads, samples and functions. Cost is computed at report time from the dated price table. A `manifest.json` written last names the sessions, price date and gungnir revision.

## Decisions

- **One crate.** No part has a second consumer. Module privacy already hides the wire types.
- **Files, not SQLite.** Transcripts, workspaces and logs are files anyway. A database would be a second truth that can disagree with them after a crash, and it cannot commit a directory atomically.
- **Harness drivers are a trait behind one registry.** One file per harness. The trait holds only pure functions, so each driver is tested against captured transcripts with no process. Spawning, sandboxing, budgets, redaction and commits are written once in the runner.
- **Languages and scenarios are data.** Their templates and references are files, and nix must read the language list.
- **Backends, not harnesses, are the unit of concurrency and rate limiting.** Claude Code serves two accounts in the pilot.
- **Records carry their inputs.** Provenance and staleness come from the same field.

## Tradeoffs accepted

- We discard a rate-limited attempt entirely, tokens included, so every sample is one uninterrupted session. Discarded and interrupted attempts are reported as overhead.
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
