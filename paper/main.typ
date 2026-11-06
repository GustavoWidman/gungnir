#import "@preview/charged-ieee:0.1.4": ieee

#let n = json("data/numbers.json")
#let crashes = json("data/python-crashes.json")

#let langs = ("python", "javascript", "typescript", "go", "rust", "zig", "c", "cpp")
#let lname = (python: "Python", javascript: "JavaScript", typescript: "TypeScript", go: "Go",
  rust: "Rust", zig: "Zig", c: "C", cpp: "C++")
#let mname = (luna: "GPT-6 Luna", terra: "GPT-6.1 Terra", haiku: "Claude Haiku 5.5")

#let fix(x, d) = {
  let s = str(calc.round(x, digits: d))
  let parts = s.split(".")
  if parts.len() == 1 { s + "." + "0" * d } else { s + "0" * (d - parts.at(1).len()) }
}
#let r0(x) = str(int(calc.round(x)))
#let r1(x) = fix(x, 1)
#let r2(x) = fix(x, 2)
#let num(k) = {
  let digits = str(int(k)).clusters().rev()
  let out = ()
  for (i, c) in digits.enumerate() {
    if i > 0 and calc.rem(i, 3) == 0 { out.push(",") }
    out.push(c)
  }
  out.rev().join()
}
#let pct(x) = r0(x) + "%"
#let times(x) = r1(x) + "×"
#let pv(p) = if p < 0.001 [_p_ < 0.001] else [_p_ = #r2(p)]
#let words = ("no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
  "eleven", "twelve")
#let count(k) = if k < words.len() { words.at(k) } else { str(k) }
#let Count(k) = { let w = count(k); upper(w.first()) + w.slice(1) }

#let pass(m, l) = n.pass.at(m).at(l)
#let pr(m, l) = pct(pass(m, l).rate)
#let prci(m, l) = [#pr(m, l) (#r0(pass(m, l).lo)–#r0(pass(m, l).hi))]
#let lowest(m) = calc.min(..langs.map(l => pass(m, l).rate))
#let highest(m) = calc.max(..langs.map(l => pass(m, l).rate))
#let outcome(m, l, k) = n.outcomes.at(m).at(l).at(k, default: 0)
#let paired(m, a, b) = n.paired.at(m).at(a + "-" + b)
#let cost(m, l, k) = n.cost.at(m).at(l).at(k).ratio
#let sess(m, l, k) = n.session.at(m).at(l).at(k)
#let gens(m) = calc.min(..n.data.models.at(m).generations.values())
#let gens-text(m) = {
  let g = n.data.models.at(m).generations.values()
  if calc.min(..g) == calc.max(..g) [#count(g.first())] else [#count(calc.min(..g)) or more]
}
#let crash-kind(k) = crashes.luna.kinds.at(k, default: 0)
#let fewest-wrong = langs.slice(1).all(l => outcome("luna", "python", "wrong-answer") < outcome("luna", l, "wrong-answer"))
#let efforts(m) = n.data.models.at(m).efforts.join(", ", last: " and ")
#let speed(m, l) = n.speed.at(m).at(l).ratio
#let byspeed(m) = langs.filter(l => l in n.speed.at(m)).sorted(key: l => speed(m, l))
#let absolute(k, l) = n.speed_absolute.at(k).at(l)
#let zig = n.diagnostic_kinds.zig
#let rust = n.diagnostic_kinds.rust
#let ts = n.diagnostic_kinds.typescript
#let sessions-total = n.data.models.values().map(m => m.trials).sum()
#let zig-tokens = ("luna", "terra", "haiku").map(m => cost(m, "zig", "tokens"))
#let mainstream = ("luna", "terra", "haiku").map(m => ("python", "javascript", "typescript", "go", "rust", "cpp").map(l => cost(m, l, "tokens"))).flatten().sorted()
#let static-others = calc.max(..("python", "javascript", "typescript", "go", "rust", "c", "cpp").map(
  l => calc.max(..("luna", "terra", "haiku").map(m => sess(m, l, "failed_builds")))))

#set page(footer: context align(center, text(size: 8pt, fill: rgb("555555"))[#counter(page).display()]))

#show: ieee.with(
  title: [Does the Language Matter to a Coding Agent? Correctness, Cost and Speed in Eight Languages],
  authors: (
    (name: "Gustavo Wagon Widman", organization: [Instituto de Tecnologia e Liderança (Inteli)],
      location: [São Paulo, Brazil]),
  ),
  abstract: [
    Developers now hand whole programs to coding agents, and they still choose the language the agent writes. Two arguments compete over that choice: a strict compiler such as Rust's should catch an agent's mistakes before they ship, while Python and JavaScript dominate the code the models learned from. We tested both. Three models, GPT-6 Luna, GPT-6.1 Terra and Claude Haiku 5.5, driven through their vendors' command-line agents across reasoning-effort settings, wrote #num(n.data.trials) programs for eight specification-only tasks in eight languages. We graded each program against hidden tests, timed it against a reference implementation on isolated cores, and read every session's transcript for the compiler errors the agent saw. The two stronger models passed every hidden test in at least #pct(calc.min(lowest("terra"), lowest("haiku"))) of trials in every language. For the weakest, the language mattered mostly through the task: a big-integer type in the standard library moved pass rates more than any type system, and JavaScript and TypeScript, which differ only by the type checker, passed equally often. Python, the language the models have seen most, trailed JavaScript, TypeScript and Go, because #pct(outcome("luna", "python", "crash")) of its programs crashed, mostly at its runtime's limits on recursion depth and on printing large integers. Agents rarely tripped the Rust compiler. The compiler earned its keep in Zig, where every model wrote an older version of the language and spent #times(calc.min(..zig-tokens))–#times(calc.max(..zig-tokens)) the tokens of an average language to recover. Among the mainstream languages, Rust cost no more than Python. The language decides what a correct program costs and how fast it runs, far more than whether the agent gets it right.
  ],
  index-terms: ([coding agents], [programming languages], [type systems], [benchmarking], [large language models]),
  bibliography: bibliography("refs.bib", style: "ieee"),
)

#set text(font: "TeX Gyre Termes", lang: "en")
#show raw: set text(font: "TeX Gyre Cursor")
#show table.cell: set text(size: 8pt)
#set table(inset: (x: 4pt, y: 2pt))
#show figure.where(kind: table): set figure.caption(position: top)

= Introduction

A coding agent is a language model in a loop with a shell. It writes a program, builds it, runs it on examples, reads what went wrong and tries again. The developer who hands it a task still decides the language it writes in, and that decision is now argued on new grounds.

One argument says to pick the strictest compiler available. Rust's type system and borrow checker reject whole classes of programs before they run, and an agent that reads compiler errors gets a reviewer that never tires. If the compiler works as a crutch, an agent should be more often right in Rust than in Python, where the same mistakes surface only when a test happens to execute them. The opposite argument says to pick the language the model knows best. Python and JavaScript dominate public code, and function-level benchmarks have found model performance correlated with a language's popularity @cassano2023multiple and tilted toward Python @multilcb2026. On this view, a model that has seen far more Python than Rust will write better Python, whatever the compiler offers.

The evidence for either view in the agent setting is thin. Function-level benchmarks sample one completion, with no loop in which a compiler could help. The agent studies of 2026 found accuracy nearly flat across languages @wu2026tokenmaxxing @endoh2026langbench, but each fixed the model's reasoning effort, covered a few languages or a single model, and neither timed the programs the agents produced. We therefore ask two questions. Does the language an agent writes in change whether its program is correct, what the program costs to produce, and how fast it runs? Where it does, is the difference explained by the strictness of the language's compiler, or by how much of the language the model has seen?

To answer them we gave three models, GPT-6 Luna and GPT-6.1 Terra through the Codex CLI and Claude Haiku 5.5 through Claude Code, eight tasks specified only in prose: a big-integer calculator, an exact-cover solver, JSON path queries, an event-sourced ledger, a line diff, a small regular-expression engine, a fuel-constrained route planner and a stack virtual machine. Each task was posed in Python, JavaScript, TypeScript, Go, Rust, Zig, C and C++, at every reasoning-effort setting the models offer, and repeated over independent generations because none of the models can be seeded. Every program was graded against 73 to 120 hidden test cases per task and timed against a reference implementation in the same language on isolated, clock-pinned cores. We also parsed every session's transcript to count the compiler and type errors the agent met on the way. Three pairs of languages act as natural experiments. JavaScript and TypeScript share a runtime and a standard library and differ only by the type checker; Python and JavaScript are both dynamically typed and both among the most common languages in public code, but run on different runtimes; and Python and Rust sit at opposite ends of both arguments.

The answer to the first question is no for correctness and yes for everything else. GPT-6.1 Terra passed every hidden test in #pct(lowest("terra"))–#pct(highest("terra")) of its trials in each language, and Claude Haiku 5.5 in #pct(lowest("haiku"))–#pct(highest("haiku")). GPT-6 Luna, the weakest model, passed #pr("luna", "all") overall and separated the languages, but mostly through the task: where the standard library offers arbitrary-precision integers, Luna's big-integer calculator passed in #pct(n.scenario_grid.at("bigint-calc").go) of Go trials, against #pct(n.scenario_grid.at("bigint-calc").rust) in Rust. JavaScript and TypeScript passed equally often, as did Python and Rust. Python, the most familiar language of all, trailed JavaScript, TypeScript and Go, and not for want of types: #pct(outcome("luna", "python", "crash")) of its programs crashed on a hidden test, against #pct(outcome("luna", "javascript", "crash")) in JavaScript, mostly at limits of Python's runtime on recursion depth and on printing large integers. The second question has a sharper answer than either argument expected. The agents seldom needed the Rust compiler, which stopped a build #r1(sess("luna", "rust", "failed_builds")) times per session for Luna and only #rust.borrow times in #rust.sessions sessions for an ownership or borrowing error. The compiler mattered in Zig, the least common language in the study and the one whose standard library changed most recently. Every model wrote Zig as it was a few releases ago and relied on the compiler to find out, at #times(calc.min(..zig-tokens)) to #times(calc.max(..zig-tokens)) the tokens of an average language for the same task. Training data showed up as price only at that far end: among the mainstream languages, Rust cost no more than Python or JavaScript.

= Study design <design>

== Tasks

Each task is a specification written in English, a handful of visible examples, a hidden test suite and a set of timing workloads (@tab:tasks). A program reads its input from standard input and writes its answer to standard output, and the specification fixes the output byte for byte, including the order of results and the wording of every error. Standard libraries differ in what they offer these tasks, and we left the differences in place, since a developer who chooses a language chooses its library too. The largest is in the big-integer calculator: Python, JavaScript (`BigInt`), Go (`math/big`) and Zig (`std.math.big`) provide arbitrary-precision integers, and Rust, C and C++ do not. Elsewhere the specifications define their own semantics closely enough that a library rarely fits as is; JSON path queries, for instance, fix strings as sequences of UTF-16 code units and numbers as 64-bit integers, which no standard JSON parser implements. The hidden suites target the edges the specifications define: empty inputs, the largest values the input admits, deep nesting and pathological backtracking. No hidden case is visible to the agent.

#figure(
  table(
    columns: (auto, 1fr, auto, auto),
    align: (left, left, right, right),
    stroke: none,
    table.hline(stroke: 0.6pt),
    [*Task*], [*What the program does*], [*Tests*], [*Loads*],
    table.hline(stroke: 0.4pt),
    [bigint-calc], [Evaluates integer expressions exactly], [73], [3],
    [exact-cover], [Counts tilings of a board by given pieces], [98], [2],
    [json-path], [Parses JSON and answers path queries], [116], [2],
    [ledger], [Runs an event-sourced bank ledger], [120], [2],
    [line-diff], [Prints a shortest line edit script], [87], [3],
    [regex-lite], [Counts matches of a small regex language], [118], [3],
    [route-fuel], [Finds cheapest routes under fuel limits], [97], [2],
    [tiny-vm], [Assembles and runs a stack machine], [120], [3],
    table.hline(stroke: 0.6pt),
  ),
  caption: [The eight tasks, with the number of hidden test cases and timing workloads of each.],
) <tab:tasks>

== Languages

The eight languages span the two arguments. Python and JavaScript are dynamically typed and the most common in public code. TypeScript adds a static type checker to JavaScript, and we build it with `tsc --noEmit` under `strict`, so a type error fails the build exactly as a compile error does in Go, Rust, Zig, C or C++. C and C++ are compiled with warnings on, Rust as a release build, and Zig in `ReleaseFast` mode. We used the newest release of every toolchain: Python 3.15, JavaScript on Bun 1.4, TypeScript 7.0 on the same Bun, Go 1.27, Rust 1.101 nightly (edition 2024), Zig 0.17, and Clang 23 for C23 and C++23. The prompt states the version, so an agent that writes for an older release does so knowing which one it is building with. Only the standard library is available, there is no network, and the prompt forbids other languages, which closes the route of generating the target program from a Python script.

== Agents

A system under test is a harness, a model and a reasoning-effort setting. GPT-6 Luna and GPT-6.1 Terra ran through the Codex CLI 0.166.0 at efforts #efforts("luna"), and Claude Haiku 5.5 ran through Claude Code 2.1.316 at #efforts("haiku"). None of the three models accepts a seed, so we treat each session as a draw and repeat every trial over independent generations: #gens-text("luna") for Luna, #gens-text("terra") for Terra and #gens-text("haiku") for Haiku. A trial is one task, one language, one system and one generation. The agent starts in a workspace that already builds an empty program and receives the specification, the visible examples, and the build and run commands. Each session may run for four hours and may sit idle for thirty minutes. A session counts once the agent declares itself finished; one cut short by the provider or the harness is run again, so every sample is a session the agent completed.

Every session runs in a bubblewrap sandbox with a private file system, no network except the model provider's endpoint through an allowlisting proxy, and limits on memory, processes and CPU. The hidden tests, the references and the grader never enter it.

== Grading and timing

A program passes when it builds offline with the language's build command and produces the exact expected output for every hidden case, each case run in its own sandbox with ten seconds and one gibibyte. We also record which cases fail and how: a wrong answer, a crash, a timeout, or exhausted memory.

Every passing program is timed against a reference implementation of the same task in the same language. Each reference passes every hidden test and was written for speed with the standard library alone. Timing runs on an Intel Core i9-13900K whose four measurement cores are isolated from the scheduler, timer ticks, RCU callbacks and interrupts and held at a fixed 3.0 GHz. Each comparison first checks the candidate's output on every workload, then runs hyperfine in six ABBA blocks of three runs each, alternating candidate and reference on one pinned core. We report the ratio of median wall times with a _t_ interval over blocks. Timing the reference against itself on the same cores puts the noise floor at a median 95% half-width of 0.7%, far below the differences we report.

== Analysis

The tasks are few and the generations of one task are correlated, so every interval we report is a 95% two-stage bootstrap interval: we resample the eight tasks with replacement, then the trials within each. Comparisons between two languages are paired: each trial in one language is matched with the trial of the same task, system and generation in the other, and an exact McNemar test asks whether one language passes where the other fails more often than the reverse. For GPT-6 Luna, whose pass rates leave room to move, logistic regressions of pass or fail on task, effort and language test the language's main effect and its interactions by likelihood ratio (Appendix A). Costs are compared within a task: each session's tokens and time are divided by the geometric mean of the same task, system and generation across the eight languages, so 1× is an average language. Finally, we parse every transcript and count the commands whose output carried a compiler, type-checker or syntax diagnostic, the distinct diagnostics, and the runs in which the agent's own test crashed.

= Results <results>

The analysis covers #num(n.data.trials) trials: #num(n.data.models.luna.trials) from GPT-6 Luna, #num(n.data.models.terra.trials) from GPT-6.1 Terra and #num(n.data.models.haiku.trials) from Claude Haiku 5.5. They consumed #r1(n.data.tokens / 1e9) billion tokens over #r0(n.data.agent_hours) hours of agent time.

== Strong models pass in every language

GPT-6.1 Terra and Claude Haiku 5.5 wrote a correct program in nearly every trial, in every language (@fig:pass). Terra's pass rate ranged from #pct(lowest("terra")) to #pct(highest("terra")) across the eight languages and Haiku's from #pct(lowest("haiku")) to #pct(highest("haiku")). At this difficulty a capable agent's language does not decide whether it gets the program right.

#figure(
  image("figures/pass-by-language.svg", width: 100%),
  caption: [Share of trials passing every hidden test, over all tasks, efforts and generations, with 95% two-stage bootstrap intervals. Languages are ordered by GPT-6 Luna's pass rate.],
) <fig:pass>

GPT-6 Luna did separate the languages. It passed #pr("luna", "all") of all its trials, from #prci("luna", "go") in Go to #prci("luna", "zig") in Zig. The intervals are wide because they include the variation between tasks, which is where most of Luna's differences come from.

== The standard library separates the languages; the type checker does not

The differences between languages depend on the task far more than on the language itself (@fig:scenarios). Adding language to a logistic model of Luna's outcomes that already accounts for task and effort improves it (χ² = #r1(n.logistic.language.chi2), #n.logistic.language.df df, #pv(n.logistic.language.p)), but letting the language effect differ by task improves it much more (χ² = #r1(n.logistic.scenario_x_language.chi2), #n.logistic.scenario_x_language.df df, #pv(n.logistic.scenario_x_language.p)). The interaction explains #r1((n.logistic.r2.scenario_x_language - n.logistic.r2.language) / (n.logistic.r2.language - n.logistic.r2.base)) times as much deviance as the main effect.

#figure(
  image("figures/luna-scenarios.svg", width: 100%),
  caption: [GPT-6 Luna's pass rate (%) by task and language, over all efforts and generations. Darker cells pass more often.],
) <fig:scenarios>

The big-integer calculator shows why. In Go, Python and JavaScript, whose standard libraries carry arbitrary-precision integers, Luna passed #pct(n.scenario_grid.at("bigint-calc").go), #pct(n.scenario_grid.at("bigint-calc").python) and #pct(n.scenario_grid.at("bigint-calc").javascript) of its trials. In Rust, C and C++, where the agent must write its own multiplication and division over limbs, it passed #pct(n.scenario_grid.at("bigint-calc").rust), #pct(n.scenario_grid.at("bigint-calc").c) and #pct(n.scenario_grid.at("bigint-calc").cpp). The stack machine reverses the ranking: Go and Rust passed #pct(n.scenario_grid.at("tiny-vm").go) and #pct(n.scenario_grid.at("tiny-vm").rust), Python #pct(n.scenario_grid.at("tiny-vm").python), because the task's 64-bit overflow rules come free with fixed-width integers and must be emulated over Python's unbounded ones.

JavaScript and TypeScript isolate the type checker: same runtime, same library, same tasks. Of #paired("luna", "javascript", "typescript").pairs matched trials, #paired("luna", "javascript", "typescript").only_a passed only in JavaScript and #paired("luna", "javascript", "typescript").only_b only in TypeScript (#pv(paired("luna", "javascript", "typescript").p)). Python and Rust, the two poles of the debate, came out even: #paired("luna", "python", "rust").only_a trials passed only in Python and #paired("luna", "python", "rust").only_b only in Rust. C and C++, which differ less in their checking than in their standard library, #if paired("luna", "c", "cpp").p < 0.05 [favoured] else [leaned toward] C++: #paired("luna", "c", "cpp").only_b trials passed only in C++ and #paired("luna", "c", "cpp").only_a only in C (#pv(paired("luna", "c", "cpp").p)).

== Python's runtime, not its typing, costs it trials

#figure(
  table(
    columns: (auto, auto, auto, auto, auto, auto, auto),
    align: (left, right, right, right, right, right, right),
    stroke: none,
    table.hline(stroke: 0.6pt),
    [], [*Pass*], [*Wrong*], [*Time*], [*Crash*], [*Build*], [*Mem.*],
    table.hline(stroke: 0.4pt),
    ..langs.map(l => (
      [#lname.at(l)],
      [#r0(outcome("luna", l, "pass"))],
      [#r0(outcome("luna", l, "wrong-answer"))],
      [#r0(outcome("luna", l, "timeout"))],
      [#r0(outcome("luna", l, "crash"))],
      [#r0(outcome("luna", l, "build-error"))],
      [#r0(outcome("luna", l, "out-of-memory"))],
    )).flatten(),
    table.hline(stroke: 0.6pt),
  ),
  caption: [How GPT-6 Luna's programs ended (% of trials): passing every hidden test, or failing first by a wrong answer, timeout, crash, failed build or exhausted memory.],
) <tab:outcomes>

Python, the language with the most public code, fell behind JavaScript, TypeScript and Go. Of #paired("luna", "python", "javascript").pairs matched trials, #paired("luna", "python", "javascript").only_b passed only in JavaScript and #paired("luna", "python", "javascript").only_a only in Python (#pv(paired("luna", "python", "javascript").p)), and TypeScript and Go beat it by similar margins, #paired("luna", "python", "typescript").only_b to #paired("luna", "python", "typescript").only_a (#pv(paired("luna", "python", "typescript").p)) and #paired("luna", "python", "go").only_b to #paired("luna", "python", "go").only_a (#pv(paired("luna", "python", "go").p)). JavaScript shares Python's dynamic typing and its place in public code, so neither explains the gap. How the programs failed does (@tab:outcomes). Luna's Python programs crashed on a hidden test in #pct(outcome("luna", "python", "crash")) of trials, against #pct(outcome("luna", "javascript", "crash")) in JavaScript, #pct(outcome("luna", "typescript", "crash")) in TypeScript, #pct(outcome("luna", "go", "crash")) in Go and #pct(outcome("luna", "rust", "crash")) in Rust, and gave a wrong answer in #pct(outcome("luna", "python", "wrong-answer"))#if fewest-wrong [, the fewest of any language].

The crashes are not the errors a type checker catches. We replayed one crashing case of each of the #crashes.luna.trials Python trials that crashed and read the exception. Of these, #count(crash-kind("RecursionError")) hit Python's default limit of 1,000 nested calls on the regular-expression engine's deeply nested patterns, and #count(crash-kind("IntDigitLimit")) hit the interpreter's default guard against converting integers of more than 4,300 digits to text, which the big-integer calculator's answers exceed. Only #count(crash-kind("TypeError")) raised a `TypeError`. Both common causes are runtime defaults that a program can raise in one line, and no type system would have flagged either. JavaScript, whose runtime has neither limit at these sizes, crashed in #pct(outcome("luna", "javascript", "crash")) of its trials. Type checking itself changed little: TypeScript's programs passed and failed much as JavaScript's did.

== Agents rarely need the Rust compiler

If the Rust compiler were a crutch, the agents would lean on it. They hardly did (@fig:builds). A Rust session met a failing build #r1(sess("luna", "rust", "failed_builds")) times on average for Luna, #r1(sess("terra", "rust", "failed_builds")) for Terra and #r1(sess("haiku", "rust", "failed_builds")) for Haiku. Across all #rust.sessions Rust sessions the compiler reported #rust.total distinct errors, and #rust.borrow of them concerned ownership or borrowing; the rest were type mismatches and calls to methods that do not exist on the receiver's type. The models nearly always write Rust that satisfies the borrow checker at the first attempt, and what remains wrong in their programs is logic the compiler cannot see.

#figure(
  image("figures/failed-builds.svg", width: 100%),
  caption: [Commands per session whose output carried a compiler, type-checker or syntax error: mean with 95% two-stage bootstrap interval. Python and JavaScript surface such errors only at run time.],
) <fig:builds>

TypeScript's checker spoke up more often, #r1(sess("luna", "typescript", "failed_builds")) failing builds per Luna session, and #ts.null of its #ts.total errors were strict null checks: an array read or map lookup that might be `undefined`. The agent answers them with a guard, the build passes, and, as the paired comparison shows, the program is no more often correct for it.

== Zig's compiler corrects what the models remember wrongly

Zig is where the compiler did the work its advocates claim. Zig sessions met #r1(sess("luna", "zig", "failed_builds")), #r1(sess("terra", "zig", "failed_builds")) and #r1(sess("haiku", "zig", "failed_builds")) failing builds per session for Luna, Terra and Haiku, against at most #r1(static-others) in any other language. Of the #zig.total errors, #zig.api named a declaration the standard library no longer has. The models reach for `std.io.getStdOut`, `std.fs.File` and the `writer` method of a managed `ArrayList`, all renamed or removed in the releases before 0.17, although the prompt names the version. A further #zig.rules errors enforced Zig's own discipline: a variable declared `var` but never mutated, an unused local, whitespace on one side of a binary operator. These catch no bug; they are a toll the language charges every writer, and the agents paid it repeatedly.

The compiler turns this stale knowledge into working code. Terra and Haiku passed #pr("terra", "zig") and #pr("haiku", "zig") of their Zig trials. Luna, with less to recover with, passed #pr("luna", "zig"), and #pct(outcome("luna", "zig", "build-error")) of its Zig programs never built at all, the most of any language.

== More reasoning closes the gap between languages

Luna's spread between languages shrank as its reasoning effort rose (@fig:effort). Without reasoning its languages passed between #pct(calc.min(..langs.map(l => n.effort.at(l).none))) and #pct(calc.max(..langs.map(l => n.effort.at(l).none))) of trials; at the highest setting, between #pct(calc.min(..langs.map(l => n.effort.at(l).max))) and #pct(calc.max(..langs.map(l => n.effort.at(l).max))). Zig was the slowest to come up, at #pct(n.effort.zig.medium) at medium effort against #pct(n.effort.all.medium) for the average language, and caught up only at the two highest settings. Effort, like model strength, buys the agent out of most of what the language costs it.

#figure(
  image("figures/luna-effort.svg", width: 100%),
  caption: [GPT-6 Luna's pass rate by reasoning effort, one panel per language. Black: the language; dashed: the mean over languages; band: 95% two-stage bootstrap interval.],
) <fig:effort>

== The least familiar language costs two to three times as much

Correctness comes at a different price in each language (@fig:tokens). Zig sessions used #times(cost("luna", "zig", "tokens")), #times(cost("terra", "zig", "tokens")) and #times(cost("haiku", "zig", "tokens")) the tokens of an average language for Luna, Terra and Haiku, and took #times(calc.min(..("luna", "terra", "haiku").map(m => cost(m, "zig", "wall_s")))) to #times(calc.max(..("luna", "terra", "haiku").map(m => cost(m, "zig", "wall_s")))) the time. C was the next most expensive, at #times(calc.min(..("luna", "terra", "haiku").map(m => cost(m, "c", "tokens")))) to #times(calc.max(..("luna", "terra", "haiku").map(m => cost(m, "c", "tokens")))) the tokens. The other six languages sat between #times(mainstream.first()) and #times(mainstream.last()). Python and JavaScript were the cheapest for Luna, at #times(cost("luna", "python", "tokens")) and #times(cost("luna", "javascript", "tokens"))\; for the stronger models the six overlap within their intervals, and Rust cost no more than Python. The training-data argument holds at the far end, where the least familiar language costs two to three times as much, and not among the languages a developer is likely to weigh against each other.

#figure(
  image("figures/tokens-by-language.svg", width: 100%),
  caption: [Tokens per session relative to the geometric mean of the same task, system and generation across the eight languages (1×, dashed): geometric mean with 95% two-stage bootstrap interval.],
) <fig:tokens>

== Agents leave the most speed behind in the fastest languages

Measured against a reference in their own language, the agents' programs came closest in Python and fell furthest behind in the compiled languages (@fig:speed). Terra's passing Python programs ran at #times(speed("terra", "python")) the reference's time, against #times(speed("terra", "go")) in Go, #times(speed("terra", "c")) in C and #times(speed("terra", "zig")) in Zig. Luna's ranged from #times(speed("luna", byspeed("luna").first())) in #lname.at(byspeed("luna").first()) to #times(speed("luna", byspeed("luna").last())) in #lname.at(byspeed("luna").last()). In an interpreted language much of the runtime is the interpreter's, so a weaker algorithm costs less in relative terms; a compiled language exposes every extra allocation and copy.

#figure(
  image("figures/speed.svg", width: 100%),
  caption: [Runtime of each passing program relative to the same-language reference, on a log scale. Grey: one program, geometric mean over the task's workloads; bar: geometric mean over programs; dashed: parity.],
) <fig:speed>

The choice of language still decides the order of magnitude (@fig:absolute). Against the fastest reference for each workload, Terra's Rust programs took #times(absolute("terra", "rust")) its time and its C programs #times(absolute("terra", "c")), while its Python programs took #times(absolute("terra", "python")) and the Python references themselves #times(absolute("reference", "python")). An agent writing Rust or C leaves more on the table than one writing Python, and its program is still an order of magnitude faster. Reasoning effort bought speed as well: Luna's programs went from #times(n.speed.luna.by_effort.none) the reference's time at no reasoning to #times(n.speed.luna.by_effort.max) at the highest setting.

#figure(
  image("figures/speed-absolute.svg", width: 100%),
  caption: [Runtime relative to the fastest reference in any language for the same workload, geometric mean over workloads. Bars mark the references; open markers the agents' passing programs.],
) <fig:absolute>

= Discussion <discussion>

The compiler-as-crutch argument assumes that an agent's mistakes are the kind a compiler catches. For these models and tasks they mostly are not. The agents write well-typed, borrow-checked code from the start; what goes wrong is an algorithm that times out, an edge case the specification spells out and the program gets wrong, or a limit of the language's runtime. Adding a type checker to JavaScript changed none of these counts. Where the compiler did help, in Zig, it helped by correcting stale knowledge of the language rather than errors of reasoning, and between those corrections and its own style rules it charged two to three times the tokens.

The training-data argument assumes that familiarity buys correctness. The most familiar language, Python, trailed JavaScript, TypeScript and Go for the weakest model, and for reasons unrelated to how much Python it had seen: a recursion limit and a cap on printing large integers, each a default the program could have raised. For capable models familiarity bought, at most, a cheaper session, and only at the extreme: the least familiar language, Zig, cost two to three times as much, while Rust cost no more than Python or JavaScript. Familiarity in the sense that matters is also a matter of date. Zig's standard library has moved under the models' feet, and they write the version they learned.

For a developer choosing a language for an agent, the results point away from both slogans. A capable agent will get the program right in any of these languages. The standard library and the runtime's defaults decide how much the agent must build or work around, and so how often a weaker agent gets it right; the language's popularity decides how much the session costs; and the language's performance ceiling decides how fast the result runs, even after the agent leaves part of that ceiling unused.

= Related work <related>

Multilingual benchmarks of code generation translate function-level problems into many languages and score single completions. MultiPL-E @cassano2023multiple translated two Python benchmarks into eighteen languages and found performance correlated with a language's popularity, with some niche languages doing as well as popular ones, and static type checking neither helping nor hurting. Multi-LCB @multilcb2026 extended a contest benchmark to twelve languages and found evidence of overfitting to Python. These studies fix the setting our questions turn on: a single completion has no compiler to consult. That MultiPL-E's verdict on type checking survives the move to agents, which do consult one, is one of our results.

Agent studies of 2026 moved to the loop. Wu, Anderson and Guha @wu2026tokenmaxxing ran five models in Python, Java, Rust and OCaml and found accuracy almost independent of language while OCaml cost 1.28 to 1.69 times Python's tokens. Acher and Jézéquel @acher2026chess had Claude Code and Codex write chess engines in seventeen languages, one engine per language, and measured their strength. Endoh's benchmark @endoh2026langbench repeated one task twenty times in thirteen languages with one model, reported 597 passes in 600 runs, and found that adding a type checker made sessions 1.6 to 3.2 times slower. Our study adds a sweep over reasoning effort, the measured speed of every passing program, Zig, and a reading of the sessions themselves, which is what lets us say why the languages differ where they do.

The question of what static types buy predates agents. Gao, Bird and Barr @gao2017type found that TypeScript and Flow would have caught 15% of the public bugs they studied in JavaScript projects. Ray and colleagues @ray2014large associated language with defect rates across GitHub projects, and Berger and colleagues @berger2019impact showed in a reproduction that most of those associations do not survive careful analysis. Our JavaScript and TypeScript comparison asks the same question of agents and finds no difference in correctness. On measurement, we follow the advice of Mytkowicz and colleagues @mytkowicz2009wrong to control the environment that a timing silently depends on, and we time every program against a reference on isolated, clock-pinned cores.

The harness itself matters: the same model scores differently in different agents @merrill2026terminalbench, and rankings of complete systems are far more stable than rankings of bare models @hardy2026reliability. We therefore compare languages within a system and report each model through its own vendor's harness.

= Conclusion

We asked whether the language a coding agent writes in changes whether its program is correct, what it costs and how fast it runs, and whether a strict compiler or a large body of training data explains the differences. For the two stronger models in our study, correctness did not depend on the language: both passed every hidden test in nearly all trials in all eight languages. For the weakest, the differences followed the standard library and the runtime, not the type system: pairs of languages that differ only in their checking passed equally often, and Python, the most familiar language, trailed JavaScript because its runtime's limits crashed many of its programs.

The strict compiler was a crutch only where the model's knowledge was out of date. The agents seldom tripped Rust's compiler, and in Zig they leaned on it to unlearn a standard library that had changed, at two to three times the cost. The training data showed up as price only at that far end; among the mainstream languages, Rust was as cheap to get right as Python. The language an agent writes in decides what a correct program costs and how fast it runs; whether the agent gets it right depends on the agent.

#heading(numbering: none)[Appendix A: Logistic models]

#let clear = langs.slice(1).filter(l => {
  let o = n.logistic.odds_vs_python.at(l)
  o.hi < 1 or o.lo > 1
})

The models are fitted by maximum likelihood to Luna's #num(n.data.models.luna.trials) trials. The base model has a term for each task and each effort; adding language gives χ² = #r1(n.logistic.language.chi2) on #n.logistic.language.df degrees of freedom (#pv(n.logistic.language.p)), adding the interaction of language with task gives χ² = #r1(n.logistic.scenario_x_language.chi2) on #n.logistic.scenario_x_language.df (#pv(n.logistic.scenario_x_language.p)), and adding the interaction of language with effort instead gives χ² = #r1(n.logistic.effort_x_language.chi2) on #n.logistic.effort_x_language.df (#pv(n.logistic.effort_x_language.p)). McFadden's pseudo-R² is #r2(n.logistic.r2.base) for the base model, #r2(n.logistic.r2.language) with language and #r2(n.logistic.r2.scenario_x_language) with its interaction with task. @tab:odds gives the odds ratios of the model with language. Once the variation between tasks is allowed for, #if clear.len() == 0 [no language's interval stays clear of one.] else [only the interval of #clear.map(l => lname.at(l)).join(" and ") stays clear of one.]

#figure(
  table(
    columns: (auto, auto, auto, auto),
    align: (left, right, right, right),
    stroke: none,
    table.hline(stroke: 0.6pt),
    [*Language*], [*Odds ratio*], [*95% interval*], [*_p_*],
    table.hline(stroke: 0.4pt),
    [Python], [1], [reference], [],
    ..langs.slice(1).map(l => {
      let o = n.logistic.odds_vs_python.at(l)
      ([#lname.at(l)], [#r2(o.or)], [#r2(o.lo)–#r2(o.hi)], [#r2(o.p)])
    }).flatten(),
    table.hline(stroke: 0.6pt),
  ),
  caption: [Odds of passing relative to Python for GPT-6 Luna, with task and effort held fixed and standard errors clustered by task and language.],
) <tab:odds>
