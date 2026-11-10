#import "@preview/charged-ieee:0.1.4": ieee
#import "@preview/equate:0.3.2": equate

#let n = json("data/numbers.json")
#let crashes = json("data/python-crashes.json")

#let langs = ("python", "javascript", "typescript", "go", "rust", "zig", "c", "cpp")
#let lname = (python: "Python", javascript: "JavaScript", typescript: "TypeScript", go: "Go",
  rust: "Rust", zig: "Zig", c: "C", cpp: "C++")
#let models = ("luna", "terra", "sol", "astra", "nemotron", "deepseek", "haiku", "sonnet", "opus")
#let mname = (luna: "GPT-6 Luna", terra: "GPT-6.1 Terra", sol: "GPT-6.1 Sol", astra: "GPT-6 Astra",
  nemotron: "Nemotron 3.5 Super VL", deepseek: "DeepSeek V4.1 Flash", haiku: "Claude Haiku 5.5",
  sonnet: "Claude Sonnet 5.5", opus: "Claude Opus 5.5")
#let short = (luna: "Luna", terra: "Terra", sol: "Sol", astra: "Astra", nemotron: "Nemotron",
  deepseek: "DeepSeek", haiku: "Haiku", sonnet: "Sonnet", opus: "Opus")
#let vendor = (luna: "OpenAI", terra: "OpenAI", sol: "OpenAI", astra: "OpenAI", nemotron: "NVIDIA",
  deepseek: "DeepSeek", haiku: "Anthropic", sonnet: "Anthropic", opus: "Anthropic")
#let agent = (luna: "Codex", terra: "Codex", sol: "Codex", astra: "Codex", nemotron: "Codex",
  deepseek: "Codex", haiku: "Claude Code", sonnet: "Claude Code", opus: "Claude Code")

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
#let pv(p) = if p < 0.001 [_p_ < 0.001] else if p < 0.01 [_p_ = #fix(p, 3)] else [_p_ = #r2(p)]
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
#let crash-kind(m, k) = crashes.at(m, default: (kinds: (:))).kinds.at(k, default: 0)
#let effort-names = ("none", "low", "medium", "high", "xhigh", "max")
#let effort-span(m) = {
  let e = n.data.models.at(m).efforts
  let at = e.map(x => effort-names.position(y => y == x))
  if e.len() > 2 and at.last() - at.first() == e.len() - 1 [#e.first()–#e.last()] else [#e.join(", ")]
}
#let speed(m, l) = n.speed.at(m).at(l).ratio
#let byspeed(m) = langs.filter(l => l in n.speed.at(m)).sorted(key: l => speed(m, l))
#let absolute(k, l) = n.speed_absolute.at(k).at(l)
#let idx(k, l) = n.index.at(k).at(l)
#let dim(d, l) = n.index.dimensions.at(d).at(l).score
#let by-index(k) = langs.sorted(key: l => -idx(k, l).score)
#let lead = by-index("all").first()
#let trail = by-index("all").slice(6)
#let without-runtime(l) = 100 * calc.exp(("pass", "tokens", "time", "simplicity").map(d => calc.ln(dim(d, l) / 100)).sum() / 4)
#let lead-without-runtime = langs.sorted(key: l => -without-runtime(l)).first()
#let same-lead = models.all(k => by-index(k).first() == lead)
#let zig = n.diagnostic_kinds.zig
#let rust = n.diagnostic_kinds.rust
#let ts = n.diagnostic_kinds.typescript
#let overall(m) = pass(m, "all").rate
// The models that pass nearly everything, and the two that leave room for the language to matter.
#let strong = models.filter(m => overall(m) >= 90)
#let weak = models.filter(m => overall(m) < 90)
#let names(ms) = ms.map(m => mname.at(m)).join(", ", last: " and ")
#let shorts(ms) = ms.map(m => short.at(m)).join(", ", last: " and ")
#let lo-of(ms, f) = calc.min(..ms.map(f))
#let hi-of(ms, f) = calc.max(..ms.map(f))
#let argmin(ms, f) = ms.sorted(key: f).first()
#let argmax(ms, f) = ms.sorted(key: f).last()
#let strong-lo = lo-of(strong, overall)
#let strong-hi = hi-of(strong, overall)
#let zig-tokens(m) = cost(m, "zig", "tokens")
#let zig-builds(m) = sess(m, "zig", "failed_builds")
#let rust-builds(m) = sess(m, "rust", "failed_builds")
#let zig-most = models.all(m => langs.filter(l => l != "zig").all(l => sess(m, l, "failed_builds") < zig-builds(m)))
#let zig-strong-pass = strong.all(m => pass(m, "zig").rate >= 95)
#let six = ("python", "javascript", "typescript", "go", "rust", "cpp")
#let mainstream = models.map(m => six.map(l => cost(m, l, "tokens"))).flatten().sorted()
#let odds = n.logistic.odds_vs_python
#let clear = langs.slice(1).filter(l => odds.at(l).hi < 1 or odds.at(l).lo > 1)
#let odds-lo = langs.slice(1).sorted(key: l => odds.at(l).or).first()
#let odds-hi = langs.slice(1).sorted(key: l => odds.at(l).or).last()
#let more(x) = pct(100 * (x - 1))
// Rust's tokens over Python's for Luna, and over the cheaper of Python and JavaScript's at most
// for the stronger models.
#let rust-luna = cost("luna", "rust", "tokens") / cost("luna", "python", "tokens")
#let rust-strong = calc.max(..strong.map(m => ("python", "javascript").map(l => cost(m, "rust", "tokens") / cost(m, l, "tokens"))).flatten())
#let python-closest = models.all(m => byspeed(m).first() == "python")
// The models whose programs ran about as fast as the references, and the fastest of them.
#let near = strong.filter(m => m != "terra" and m != "deepseek")
#let near-lo = calc.min(..near.map(m => langs.map(l => speed(m, l))).flatten())
#let near-hi = calc.max(..near.map(m => langs.map(l => speed(m, l))).flatten())
#let opus-faster = langs.all(l => speed("opus", l) < 1)
#let og(g, l, k) = n.outcomes_group.at(g).at(l).at(k, default: 0)
#let er(m) = n.effort_models.at(m).values()
#let strong-effort-lo = calc.min(..strong.map(m => calc.min(..er(m))))
#let strong-effort-hi = calc.max(..strong.map(m => calc.max(..er(m))))
#let effort-lo(m) = calc.min(..er(m))
#let effort-hi(m) = calc.max(..er(m))
#let slow = n.strong_python_timeouts
#let slow-task = slow.keys().filter(k => k != "total").sorted(key: k => -slow.at(k)).first()
#let all-pairs(a, b) = n.paired.all.at(a + "-" + b)
#let strong-pairs(a, b) = n.paired.strong.at(a + "-" + b)
#let recursion-claude = ("haiku", "sonnet", "opus").filter(m => crash-kind(m, "RecursionError") > 0)

#set page(footer: context align(center, text(size: 8pt, fill: rgb("555555"))[#counter(page).display()]))

#show: ieee.with(
  title: [Does the Language Matter to a Coding Agent? Correctness, Cost and Speed in Eight Languages],
  authors: (
    (
      name: "Gustavo Wagon Widman",
      department: [Engenharia da Computação],
      organization: [Instituto de Tecnologia e Liderança (Inteli)],
      location: [São Paulo, Brazil],
      email: "gustavo.widman@sou.inteli.edu.br",
    ),
    (
      name: "Laíza Ribeiro da Silva",
      organization: [Instituto de Tecnologia e Liderança (Inteli)],
      location: [São Paulo, Brazil],
      email: "laiza@inteli.edu.br",
    ),
  ),
  abstract: [
    Developers now hand whole programs to coding agents, and they still choose the language the agent writes. Two arguments compete over that choice: a strict compiler such as Rust's should catch an agent's mistakes before they ship, while Python and JavaScript dominate the code the models learned from. We tested both. #Count(models.len()) models from four vendors, driven through command-line agents across reasoning-effort settings, wrote #num(n.data.trials) programs for eight specification-only tasks in eight languages. We graded each program against hidden tests, timed it against a reference implementation on isolated cores, and read every session's transcript for the compiler errors the agent saw. #Count(strong.len()) of the models passed every hidden test in #pct(strong-lo) to #pct(strong-hi) of their trials. For the #count(weak.len()) that passed about half, the language mattered mostly through the task: a big-integer type in the standard library moved pass rates more than any type system, and JavaScript and TypeScript, which differ only by the type checker, passed no differently. Python, the language the models have seen most, trailed JavaScript, TypeScript and Go, even among the strongest models, and for its runtime rather than its typing: the weaker models' Python programs crashed at its limits on recursion depth and on printing large integers, and the stronger models' ran out of time. Agents rarely tripped the Rust compiler. The compiler earned its keep in Zig, where every model wrote an older version of the language and spent #times(lo-of(models, zig-tokens))–#times(hi-of(models, zig-tokens)) the tokens of an average language to recover. The strongest models wrote programs as fast as the reference implementations or faster. On the gungnir index, which folds correctness, tokens, time, runtime and code complexity into one score where an average language scores 100, #lname.at(lead) leads at #r0(idx("all", lead).score)#if same-lead [ for every model], and #lname.at(trail.first()) and #lname.at(trail.last()) trail at #r0(idx("all", trail.first()).score) and #r0(idx("all", trail.last()).score). The language decides what a correct program costs and how fast it runs, far more than whether the agent gets it right.
  ],
  index-terms: ([coding agents], [programming languages], [type systems], [benchmarking], [large language models]),
  bibliography: bibliography("refs.bib", style: "ieee"),
)

#set text(font: "TeX Gyre Termes", lang: "en")
#show raw: set text(font: "TeX Gyre Cursor")
#show table.cell: set text(size: 8pt)
#set table(inset: (x: 4pt, y: 2pt))
#show figure.where(kind: table): set figure.caption(position: top)
// Number each line of an aligned equation, and give display equations room to breathe.
#show: equate.with(breakable: false, sub-numbering: false, number-mode: "line")
#show math.equation.where(block: true): set block(spacing: 1.2em)
#set math.equation(supplement: none)

= Introduction

A coding agent is a language model in a loop with a shell. It writes a program, builds it, runs it on examples, reads what went wrong and tries again. The developer who hands it a task still decides the language it writes in, and that decision is now argued on new grounds.

One argument says to pick the strictest compiler available. Rust's type system and borrow checker reject whole classes of programs before they run, and an agent that reads compiler errors gets a reviewer that never tires. If the compiler works as a crutch, an agent should be more often right in Rust than in Python, where the same mistakes surface only when a test happens to execute them. The opposite argument says to pick the language the model knows best. Python and JavaScript dominate public code, and function-level benchmarks have found model performance correlated with a language's popularity @cassano2023multiple and tilted toward Python @multilcb2026. On this view, a model that has seen far more Python than Rust will write better Python, whatever the compiler offers.

The evidence for either view in the agent setting is thin. Function-level benchmarks sample one completion, with no loop in which a compiler could help. The agent studies of 2026 found accuracy nearly flat across languages @wu2026tokenmaxxing @endoh2026langbench, but each fixed the model's reasoning effort, covered a few languages or a single model, and neither timed the programs the agents produced. We therefore ask two questions. Does the language an agent writes in change whether its program is correct, what the program costs to produce, and how fast it runs? Where it does, is the difference explained by the strictness of the language's compiler, or by how much of the language the model has seen?

To answer them we gave #count(models.len()) models from OpenAI, Anthropic, NVIDIA and DeepSeek, driven through the Codex CLI and Claude Code, eight tasks specified only in prose: a big-integer calculator, an exact-cover solver, JSON path queries, an event-sourced ledger, a line diff, a small regular-expression engine, a fuel-constrained route planner and a stack virtual machine. Each task was posed in Python, JavaScript, TypeScript, Go, Rust, Zig, C and C++, across the models' reasoning-effort settings, and repeated over independent generations because none of the models can be seeded. Every program was graded against 73 to 120 hidden test cases per task and timed against a reference implementation in the same language on isolated, clock-pinned cores. We also parsed every session's transcript to count the compiler and type errors the agent met on the way. Three pairs of languages act as natural experiments. JavaScript and TypeScript share a runtime and a standard library and differ only by the type checker; Python and JavaScript are both dynamically typed and both among the most common languages in public code, but run on different runtimes; and Python and Rust sit at opposite ends of both arguments.

The answer to the first question is no for correctness and yes for everything else. #Count(strong.len()) of the #count(models.len()) models passed every hidden test in #pct(strong-lo) to #pct(strong-hi) of their trials. GPT-6 Luna and Nemotron 3.5 Super VL passed #pr("luna", "all") and #pr("nemotron", "all") and separated the languages, but mostly through the task: where the standard library offers arbitrary-precision integers, Luna's big-integer calculator passed in #pct(n.scenario_grid.luna.at("bigint-calc").go) of Go trials, against #pct(n.scenario_grid.luna.at("bigint-calc").rust) in Rust. JavaScript and TypeScript passed equally often, as did Python and Rust. Python, the most familiar language of all, trailed JavaScript, TypeScript and Go for weak and strong models alike, and not for want of types: #pct(og("weak", "python", "crash")) of the weaker models' Python programs crashed on a hidden test, against #pct(og("weak", "javascript", "crash")) in JavaScript, mostly at limits of Python's runtime on recursion depth and on printing large integers, and the stronger models' Python programs were the likeliest of any to run past the time limit. The second question has a sharper answer than either argument expected. The agents seldom needed the Rust compiler, which reported only #num(rust.borrow) ownership or borrowing errors in #num(rust.sessions) sessions. The compiler mattered in Zig, the least common language in the study and the one whose standard library changed most recently. Every model wrote Zig as it was a few releases ago and relied on the compiler to find out, at #times(lo-of(models, zig-tokens)) to #times(hi-of(models, zig-tokens)) the tokens of an average language for the same task. Training data showed up as price mostly at that far end: among the mainstream languages, Rust cost the stronger models at most #more(rust-strong) more tokens than Python or JavaScript. The language also set how fast the result ran, and the strongest models wrote programs that matched or beat the references. The gungnir index, which weighs correctness, tokens, time, runtime and code complexity together, puts #lname.at(lead) first#if same-lead [ for every model].

= Study design <design>

We ran the study with gungnir, a harness we built for it. It drives each vendor's command-line agent headless in a sandbox, grades and times the program the agent leaves, and keeps every transcript. The harness, the tasks, the references and every record behind this paper are available at #link("https://github.com/GustavoWidman/gungnir")[github.com/GustavoWidman/gungnir].

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

A system under test is a harness, a model and a reasoning-effort setting (@tab:models). OpenAI's models ran through the Codex CLI 0.166.0 against OpenAI's endpoint, Nemotron 3.5 Super VL and DeepSeek V4.1 Flash through the same CLI against their vendors' endpoints, and Anthropic's models through Claude Code 2.1.316. None of the models accepts a seed, so we treat each session as a draw and repeat trials over independent generations. A trial is one task, one language, one system and one generation, and a generation counts only once every effort of its model has completed it, so that no model's results lean toward the efforts that finished first. The agent starts in a workspace that already builds an empty program and receives the specification, the visible examples, and the build and run commands. Each session may run for four hours and may sit idle for thirty minutes. A session counts once the agent declares itself finished; one cut short by the provider or the harness is run again, so every sample is a session the agent completed.

Every session runs in a bubblewrap sandbox with a private file system, no network except the model provider's endpoint through an allowlisting proxy, and limits on memory, processes and CPU. The hidden tests, the references and the grader never enter it.

== Grading and timing

A program passes when it builds offline with the language's build command and produces the exact expected output for every hidden case, each case run in its own sandbox with ten seconds and one gibibyte. A program whose only failures were timeouts is graded again on an idle machine, so a timeout reflects the program and not the load. We also record which cases fail and how: a wrong answer, a crash, a timeout, or exhausted memory.

Every passing program is timed against a reference implementation of the same task in the same language. The references were written with the tasks, by Claude Opus 5.5 in Claude Code: one algorithm per task, implemented in each of the eight languages as idiomatic code over the standard library alone. Each passes every hidden test, and because every language implements the same algorithm, the references differ by language and not by method. Timing runs on an Intel Core i9-13900K whose four measurement cores are isolated from the scheduler, timer ticks, RCU callbacks and interrupts and held at a fixed 3.0 GHz. Each comparison first checks the candidate's output on every workload, then runs hyperfine in six ABBA blocks of three runs each, alternating candidate and reference on one pinned core. We report the ratio of median wall times with a _t_ interval over blocks. Timing the reference against itself on the same cores puts the noise floor at a median 95% half-width of 0.7%, far below the differences we report.

== Analysis

The tasks are few and the generations of one task are correlated, so every interval we report is a 95% two-stage bootstrap interval: we resample the eight tasks with replacement, then the trials within each. Comparisons between two languages are paired: each trial in one language is matched with the trial of the same task, system and generation in the other, and an exact McNemar test asks whether one language passes where the other fails more often than the reverse. Logistic regressions of pass or fail on task, system and language, over all #count(models.len()) models, test the language's main effect and its interaction with the task by likelihood ratio, with standard errors clustered by task and language. GPT-6 Luna, whose pass rates leave room to move and which ran every effort over #count(gens("luna")) generations, gets a further regression that tests the language's interaction with effort. Costs are compared within a task: each session's tokens and time are divided by the geometric mean of the same task, system and generation across the eight languages, so 1× is an average language. Finally, we parse every transcript and count the commands whose output carried a compiler, type-checker or syntax diagnostic, the distinct diagnostics, and the runs in which the agent's own test crashed.

= Results <results>

The analysis covers #num(n.data.trials) trials from #count(models.len()) models (@tab:models). They consumed #r1(n.data.tokens / 1e9) billion tokens over #r0(n.data.agent_hours) hours of agent time.

== Strong models pass in every language

#Count(strong.len()) models, #names(strong), wrote a correct program in nearly every trial, in every language (@fig:pass). Their overall pass rates ran from #pct(strong-lo) to #pct(strong-hi), and the lowest single figure, #mname.at(argmin(strong, m => lowest(m)))'s #pct(lowest(argmin(strong, m => lowest(m)))) in #lname.at(langs.sorted(key: l => pass(argmin(strong, m => lowest(m)), l).rate).first()), rests on #count(pass(argmin(strong, m => lowest(m)), langs.sorted(key: l => pass(argmin(strong, m => lowest(m)), l).rate).first()).n) trials. At this difficulty a capable agent's language does not decide whether it gets the program right.

GPT-6 Luna and Nemotron 3.5 Super VL did separate the languages. Luna passed #pr("luna", "all") of all its trials, from #prci("luna", "go") in Go to #prci("luna", "zig") in Zig, and Nemotron #pr("nemotron", "all"), from #prci("nemotron", "go") in Go to #prci("nemotron", "zig") in Zig. The intervals are wide because they include the variation between tasks, which is where most of the differences come from.

#figure(
  table(
    columns: (auto, auto, auto, auto, auto),
    align: (left, left, left, right, right),
    inset: (x: 4pt, y: 1.5pt),
    stroke: none,
    table.hline(stroke: 0.6pt),
    [*Model*], [*Agent*], [*Efforts*], [*Gen.*], [*Trials*],
    table.hline(stroke: 0.4pt),
    ..models.map(m => ([#mname.at(m)], [#agent.at(m)], [#effort-span(m)], [#gens(m)],
      [#num(n.data.models.at(m).trials)])).flatten(),
    table.hline(stroke: 0.6pt),
  ),
  caption: [The models, their agents and efforts, and the generations and trials analysed.],
) <tab:models>

#figure(
  image("figures/pass-by-language.svg", width: 100%),
  caption: [Share of each model's trials passing every hidden test (%), by language, over all its efforts and generations. Darker cells pass more often.],
) <fig:pass>

== The standard library separates the languages; the type checker does not

The differences between languages depend on the task far more than on the language itself (@fig:scenarios). Adding language to a logistic model of all #count(models.len()) models' outcomes that already accounts for task and system improves it (χ² = #r1(n.logistic.language.chi2), #n.logistic.language.df df, #pv(n.logistic.language.p)), but letting the language effect differ by task improves it much more (χ² = #r1(n.logistic.scenario_x_language.chi2), #n.logistic.scenario_x_language.df df, #pv(n.logistic.scenario_x_language.p)). The interaction explains #r1((n.logistic.r2.scenario_x_language - n.logistic.r2.language) / (n.logistic.r2.language - n.logistic.r2.base)) times as much deviance as the main effect. Once the variation between tasks is allowed for, #if clear.len() == 0 [no language's odds of passing differ from Python's: the odds ratios run from #r2(odds.at(odds-lo).or) for #lname.at(odds-lo) to #r2(odds.at(odds-hi).or) for #lname.at(odds-hi), and every 95% interval includes one.] else [only #clear.map(l => lname.at(l)).join(" and ")'s odds of passing differ from Python's.]

#figure(
  image("figures/weak-scenarios.svg", width: 100%),
  caption: [Pass rate (%) by task and language of the two models that leave the language room to matter, over all their efforts and generations: #count(int(n.data.models.luna.trials / 64)) trials per cell for GPT-6 Luna and #count(int(n.data.models.nemotron.trials / 64)) for Nemotron 3.5 Super VL. Darker cells pass more often.],
) <fig:scenarios>

The big-integer calculator shows why. In Go, Python and JavaScript, whose standard libraries carry arbitrary-precision integers, Luna passed #pct(n.scenario_grid.luna.at("bigint-calc").go), #pct(n.scenario_grid.luna.at("bigint-calc").python) and #pct(n.scenario_grid.luna.at("bigint-calc").javascript) of its trials. In Rust, C and C++, where the agent must write its own multiplication and division over limbs, it passed #pct(n.scenario_grid.luna.at("bigint-calc").rust), #pct(n.scenario_grid.luna.at("bigint-calc").c) and #pct(n.scenario_grid.luna.at("bigint-calc").cpp). Nemotron passed the task in #pct(n.scenario_grid.nemotron.at("bigint-calc").go) of its Go, Python and JavaScript trials and in #pct(n.scenario_grid.nemotron.at("bigint-calc").rust) of its Rust and C++ ones, though its C programs passed as often as its Go ones. The stack machine reverses the ranking: Luna's Go and Rust programs passed #pct(n.scenario_grid.luna.at("tiny-vm").go) and #pct(n.scenario_grid.luna.at("tiny-vm").rust), its Python ones #pct(n.scenario_grid.luna.at("tiny-vm").python), because the task's 64-bit overflow rules come free with fixed-width integers and must be emulated over Python's unbounded ones.

JavaScript and TypeScript isolate the type checker: same runtime, same library, same tasks. Of #num(all-pairs("javascript", "typescript").pairs) trials matched across all #count(models.len()) models, #all-pairs("javascript", "typescript").only_a passed only in JavaScript and #all-pairs("javascript", "typescript").only_b only in TypeScript (#pv(all-pairs("javascript", "typescript").p)). Python and Rust, the two poles of the debate, came out even: #all-pairs("python", "rust").only_a trials passed only in Python and #all-pairs("python", "rust").only_b only in Rust (#pv(all-pairs("python", "rust").p)). C and C++, which differ less in their checking than in their standard library, #if all-pairs("c", "cpp").p < 0.05 [favoured] else [leaned toward] C++: #all-pairs("c", "cpp").only_b trials passed only in C++ and #all-pairs("c", "cpp").only_a only in C (#pv(all-pairs("c", "cpp").p)).

== Python's runtime, not its typing, costs it trials

Python, the language with the most public code, fell behind JavaScript, TypeScript and Go. Of #num(all-pairs("python", "javascript").pairs) trials matched across the models, #all-pairs("python", "javascript").only_b passed only in JavaScript and #all-pairs("python", "javascript").only_a only in Python (#pv(all-pairs("python", "javascript").p)), and TypeScript and Go beat it by similar margins, #all-pairs("python", "typescript").only_b to #all-pairs("python", "typescript").only_a (#pv(all-pairs("python", "typescript").p)) and #all-pairs("python", "go").only_b to #all-pairs("python", "go").only_a (#pv(all-pairs("python", "go").p)). The gap holds among the #count(strong.len()) stronger models alone, where #strong-pairs("python", "javascript").only_b trials passed only in JavaScript and #strong-pairs("python", "javascript").only_a only in Python (#pv(strong-pairs("python", "javascript").p)). JavaScript shares Python's dynamic typing and its place in public code, so neither explains the gap. How the programs failed does (@tab:outcomes). The weaker models' Python programs crashed on a hidden test in #pct(og("weak", "python", "crash")) of trials, against #pct(og("weak", "javascript", "crash")) in JavaScript, #pct(og("weak", "typescript", "crash")) in TypeScript, #pct(og("weak", "go", "crash")) in Go and #pct(og("weak", "rust", "crash")) in Rust. The stronger models' rarely failed at all, and when their Python programs did, they mostly ran out of time: #r1(og("strong", "python", "timeout"))% of their Python trials timed out, against #if og("strong", "javascript", "timeout") == 0 [none] else [#r1(og("strong", "javascript", "timeout"))%] in JavaScript, #if slow.at(slow-task) == slow.total [every one] else [#slow.at(slow-task) of #slow.total] on the route planner's largest cases, which the interpreter could not finish in ten seconds with the algorithms the agents chose.

#figure(
  table(
    columns: (auto, auto, auto, auto, auto, auto, auto, auto),
    align: (left, right, right, right, right, right, right, right),
    stroke: none,
    table.hline(stroke: 0.6pt),
    [], table.cell(colspan: 5, align: center)[*Two weaker models*], table.cell(colspan: 2, align: center)[*#Count(strong.len()) stronger*],
    table.hline(start: 1, end: 6, stroke: 0.4pt), table.hline(start: 6, end: 8, stroke: 0.4pt),
    [], [*Pass*], [*Wrong*], [*Time*], [*Crash*], [*Build*], [*Pass*], [*Time*],
    table.hline(stroke: 0.4pt),
    ..langs.map(l => (
      [#lname.at(l)],
      [#r0(og("weak", l, "pass"))],
      [#r0(og("weak", l, "wrong-answer"))],
      [#r0(og("weak", l, "timeout"))],
      [#r0(og("weak", l, "crash"))],
      [#r0(og("weak", l, "build-error"))],
      [#r1(og("strong", l, "pass"))],
      [#r1(og("strong", l, "timeout"))],
    )).flatten(),
    table.hline(stroke: 0.6pt),
  ),
  caption: [How the programs ended (% of trials): passing every hidden test, or failing first by a wrong answer, timeout, crash or failed build. The rest exhausted memory or printed past the output cap; the stronger models' wrong answers and crashes stay under 1% in every language.],
) <tab:outcomes>

The crashes are not the errors a type checker catches. We replayed one crashing case of each of the #crashes.luna.trials Python trials of Luna's that crashed and read the exception. Of these, #count(crash-kind("luna", "RecursionError")) hit Python's default limit of 1,000 nested calls on the regular-expression engine's deeply nested patterns, and #count(crash-kind("luna", "IntDigitLimit")) hit the interpreter's default guard against converting integers of more than 4,300 digits to text, which the big-integer calculator's answers exceed. Only #count(crash-kind("luna", "TypeError")) raised a `TypeError`. Both common causes are runtime defaults that a program can raise in one line, and no type system would have flagged either. #if recursion-claude.len() == 3 [The recursion limit reached even the strongest models: it cost each of the three Claude models a Python trial on the regular-expression engine.] JavaScript, whose runtime has neither limit at these sizes, crashed in #pct(og("weak", "javascript", "crash")) of the weaker models' trials. Type checking itself changed little: TypeScript's programs passed and failed much as JavaScript's did.

== Agents rarely need the Rust compiler

If the Rust compiler were a crutch, the agents would lean on it. They hardly did (@fig:builds). A Rust session met a failing build between #r1(lo-of(strong, rust-builds)) and #r1(hi-of(strong, rust-builds)) times on average for the #count(strong.len()) stronger models and #r1(rust-builds("luna")) times for Luna; only Nemotron, at #r1(rust-builds("nemotron")), met the compiler often. Across all #num(rust.sessions) Rust sessions the compiler reported #num(rust.total) distinct errors, and #num(rust.borrow) of them concerned ownership or borrowing; the rest were type mismatches and calls to methods that do not exist on the receiver's type. The models nearly always write Rust that satisfies the borrow checker at the first attempt, and what remains wrong in their programs is logic the compiler cannot see.

#figure(
  image("figures/failed-builds.svg", width: 100%),
  caption: [Commands per session whose output carried a compiler, type-checker or syntax error, mean by model and language, shaded on a log scale. Python and JavaScript surface such errors only at run time.],
) <fig:builds>

TypeScript's checker spoke up more often, #r1(sess("luna", "typescript", "failed_builds")) failing builds per Luna session, and #num(ts.null) of its #num(ts.total) errors were strict null checks: an array read or map lookup that might be `undefined`. The agent answers them with a guard, the build passes, and, as the paired comparison shows, the program is no more often correct for it.

== Zig's compiler corrects what the models remember wrongly

Zig is where the compiler did the work its advocates claim. #if zig-most [Every model met more failing builds in Zig than in any other language:] else [The models met more failing builds in Zig than elsewhere:] from #r1(lo-of(strong, zig-builds)) to #r1(hi-of(strong, zig-builds)) per session for the stronger models, #r1(zig-builds("luna")) for Luna and #r1(zig-builds("nemotron")) for Nemotron, whose Zig sessions were mostly a dialogue with the compiler. Of the #num(zig.total) errors, #num(zig.api) named a declaration the standard library no longer has. The models reach for `std.io.getStdOut`, `std.fs.File` and the `writer` method of a managed `ArrayList`, all renamed or removed in the releases before 0.17, although the prompt names the version. A further #num(zig.rules) errors enforced Zig's own discipline: a variable declared `var` but never mutated, an unused local, whitespace on one side of a binary operator. These catch no bug; they are a toll the language charges every writer, and the agents paid it repeatedly.

The compiler turns this stale knowledge into working code. #if zig-strong-pass [The stronger models passed at least 95% of their Zig trials.] else [The stronger models passed between #pct(lo-of(strong, m => pass(m, "zig").rate)) and #pct(hi-of(strong, m => pass(m, "zig").rate)) of their Zig trials.] Luna and Nemotron, with less to recover with, passed #pr("luna", "zig") and #pr("nemotron", "zig"), and #pct(outcome("luna", "zig", "build-error")) of Luna's Zig programs never built at all, the most of any language.

== More reasoning closes the gap between languages

Reasoning effort mattered only where the model left room for it (@fig:effort-models). The #count(strong.len()) stronger models passed #pct(strong-effort-lo) to #pct(strong-effort-hi) of their trials at every effort they ran, while Luna rose from #pct(effort-lo("luna")) to #pct(effort-hi("luna")) and Nemotron from #pct(effort-lo("nemotron")) to #pct(effort-hi("nemotron")).

Luna's spread between languages shrank as its reasoning effort rose (@fig:effort), and letting the language effect differ by effort improves a logistic model of its outcomes (χ² = #r1(n.logistic_luna.effort_x_language.chi2), #n.logistic_luna.effort_x_language.df df, #pv(n.logistic_luna.effort_x_language.p)). Without reasoning its languages passed between #pct(calc.min(..langs.map(l => n.effort.at(l).none))) and #pct(calc.max(..langs.map(l => n.effort.at(l).none))) of trials; at the highest setting, between #pct(calc.min(..langs.map(l => n.effort.at(l).max))) and #pct(calc.max(..langs.map(l => n.effort.at(l).max))). Zig was the slowest to come up, at #pct(n.effort.zig.medium) at medium effort against #pct(n.effort.all.medium) for the average language, and caught up only at the two highest settings. Effort, like model strength, buys the agent out of most of what the language costs it.

#figure(
  image("figures/effort-models.svg", width: 100%),
  caption: [Pass rate over all languages by reasoning effort, one line per model across the efforts it ran. Grey: the #count(strong.len()) stronger models.],
) <fig:effort-models>

#figure(
  image("figures/luna-effort.svg", width: 100%),
  caption: [GPT-6 Luna's pass rate by reasoning effort, one panel per language. Black: the language; dashed: the mean over languages; band: 95% two-stage bootstrap interval.],
) <fig:effort>

== The least familiar language costs the most

#figure(
  image("figures/tokens-by-language.svg", width: 100%),
  caption: [Tokens per session relative to the geometric mean of the same task, system and generation across the eight languages, so 1.0 is an average language: geometric mean by model and language. Darker cells cost more.],
) <fig:tokens>

Correctness comes at a different price in each language (@fig:tokens). Zig sessions used #times(lo-of(strong, zig-tokens)) to #times(hi-of(strong, zig-tokens)) the tokens of an average language for the stronger models, #times(zig-tokens("luna")) for Luna and #times(zig-tokens("nemotron")) for Nemotron, and took #times(lo-of(models, m => cost(m, "zig", "wall_s"))) to #times(hi-of(models, m => cost(m, "zig", "wall_s"))) the time. C was the next most expensive, at #times(lo-of(models, m => cost(m, "c", "tokens"))) to #times(hi-of(models, m => cost(m, "c", "tokens"))) the tokens. The other six languages sat between #times(mainstream.first()) and #times(mainstream.last()). Python and JavaScript were the cheapest for Luna, at #times(cost("luna", "python", "tokens")) and #times(cost("luna", "javascript", "tokens")), and Luna's Rust sessions took #more(rust-luna) more tokens than its Python ones. For the stronger models Rust cost at most #more(rust-strong) more than Python or JavaScript.

== The strongest models match the references

Measured against a reference in its own language, a program's speed followed the model (@fig:speed). Luna's passing programs ran at #times(speed("luna", byspeed("luna").first())) the reference's time in #lname.at(byspeed("luna").first()) and up to #times(speed("luna", byspeed("luna").last())) in #lname.at(byspeed("luna").last()), and Terra's at #times(speed("terra", "python")) in Python against #times(speed("terra", "rust")) in Rust, #times(speed("terra", "c")) in C and #times(speed("terra", "zig")) in Zig. #shorts(near) wrote programs that ran at #times(near-lo) to #times(near-hi) the reference's time#if opus-faster [, and Opus's were faster than the reference in every language]. The references are idiomatic implementations of one algorithm per task, not the fastest programs the tasks admit, and the strongest agents found faster ones. #if python-closest [For every model, Python is where its programs came closest to the reference or furthest beyond it.] In an interpreted language much of the runtime is the interpreter's, so a weaker algorithm costs less in relative terms; a compiled language exposes every extra allocation and copy.

#figure(
  image("figures/speed.svg", width: 100%),
  caption: [Runtime of the passing programs relative to the reference in the same language, geometric mean by model and language. Below 1.0 the agents' programs ran faster than the reference; darker cells ran slower.],
) <fig:speed>

The choice of language still decides the order of magnitude (@fig:absolute). Against the fastest reference for each workload, Terra's Rust programs took #times(absolute("terra", "rust")) its time and its C programs #times(absolute("terra", "c")), while its Python programs took #times(absolute("terra", "python")) and the Python references themselves #times(absolute("reference", "python")). An agent writing Rust or C leaves more on the table than one writing Python, and its program is still an order of magnitude faster. Reasoning effort bought speed as well: Luna's programs went from #times(n.speed.luna.by_effort.none) the reference's time at no reasoning to #times(n.speed.luna.by_effort.max) at the highest setting.

#figure(
  image("figures/speed-absolute.svg", width: 100%),
  caption: [Runtime relative to the fastest reference in any language for the same workload, geometric mean over workloads. Bars mark the references; open markers the agents' passing programs.],
) <fig:absolute>

== The gungnir index

A developer choosing a language weighs all of these at once, so we fold them into one score, the gungnir index, which the harness computes from its records. It has five dimensions: the share of sessions whose program passes every hidden test, the tokens and the wall time of a session, the runtime of a passing program against the fastest reference for each workload, and the total cyclomatic complexity of a passing program as lizard counts it, top-level code included. For model $m$, task $t$, language $l$ and dimension $d$, let $x_(m,t,l,d)$ be the geometric mean of the dimension over the cell's sessions, or over its passing programs for runtime and complexity, with the pass rate estimated as $(k + 1 slash 2) slash (n + 1)$ from $k$ passes in $n$ sessions so that a language that never passed a task stays finite. The index is then built in three steps:

$ a_(m,t,l,d) &= s_d (ln x_(m,t,l,d) - 1/8 sum_(l') ln x_(m,t,l',d)) #<eq:advantage> \
  S_(l,d) &= 100 dot exp(1 / (|M| |T|) sum_(m in M, thin t in T) a_(m,t,l,d)) #<eq:score> \
  G_l &= root(5, product_(d in D) S_(l,d)) #<eq:index> $

Equation (@eq:advantage) is a language's advantage on one dimension: its distance from the mean of the eight languages for the same model and task, so that a model's own speed and price cancel, with $s_d$ equal to $+1$ for the pass rate and $-1$ for the others so that higher is better on every dimension. Equation (@eq:score) averages that advantage over the models $M$ and tasks $T$ into a score, so every model weighs the same however many generations it ran, and (@eq:index) takes the geometric mean of the five scores over the dimensions $D$. An average language scores 100 on each, and one whose programs run in half the average language's time scores 200 on runtime. @fig:index shows the index, and `gungnir index` prints the five scores behind it.

#figure(
  image("figures/gungnir-index.svg", width: 100%),
  caption: [The gungnir index of each language over the #count(models.len()) models, with its 95% interval. The dashed line marks the average language at 100; a dark bar's interval lies above it and a light bar's below.],
) <fig:index>

#lname.at(lead) leads the index at #r0(idx("all", lead).score) (#r0(idx("all", lead).lo)–#r0(idx("all", lead).hi))#if same-lead [, and every one of the #count(models.len()) models puts it first]. Its programs run in #times(100 / dim("speed", lead)) the average language's time and carry #times(100 / dim("simplicity", lead)) its decision points, while a session takes #times(100 / dim("tokens", lead)) the average language's tokens. The dimensions pull the other languages apart. JavaScript is the cheapest and quickest language in which to get a correct program, at #times(100 / dim("tokens", "javascript")) the average language's tokens and #times(100 / dim("time", "javascript")) its time, but its programs run #times(100 / dim("speed", "javascript")) the average language's time, and Python's #times(100 / dim("speed", "python")), so Python scores #r0(idx("all", "python").score). Zig's programs are fast, but a session takes #times(100 / dim("tokens", "zig")) the tokens and #times(100 / dim("time", "zig")) the time of an average language's, and Zig scores #r0(idx("all", "zig").score). C's programs are fast too, but a C session takes #times(100 / dim("tokens", "c")) the average language's tokens#if langs.all(l => dim("simplicity", "c") <= dim("simplicity", l)) [, and its programs carry the most decision points of all].

The index weighs its five dimensions equally, which suits compute-bound programs such as these. A developer who cares little how fast the program runs can drop the speed score: on the other four, #lname.at(lead-without-runtime) comes first at #r0(without-runtime(lead-without-runtime)), Python scores #r0(without-runtime("python")) and Zig #r0(without-runtime("zig")).

= Discussion <discussion>

The compiler-as-crutch argument assumes that an agent's mistakes are the kind a compiler catches. For these models and tasks they mostly are not. The agents write well-typed, borrow-checked code from the start; what goes wrong is an algorithm that times out, an edge case the specification spells out and the program gets wrong, or a limit of the language's runtime. Adding a type checker to JavaScript changed none of these counts. Where the compiler did help, in Zig, it helped by correcting stale knowledge of the language rather than errors of reasoning, and between those corrections and its own style rules it charged the stronger models #times(lo-of(strong, zig-tokens)) to #times(hi-of(strong, zig-tokens)) the tokens and the weakest #times(zig-tokens("nemotron")).

The training-data argument assumes that familiarity buys correctness. The most familiar language, Python, trailed JavaScript, TypeScript and Go for weak and strong models alike, and for reasons unrelated to how much Python they had seen: a recursion limit and a cap on printing large integers, each a default the program could have raised, and an interpreter too slow for the largest cases of one task. For capable models familiarity bought, at most, a cheaper session, and only at the extreme: the least familiar language, Zig, cost the most, while Rust cost at most #more(rust-strong) more than Python or JavaScript. Familiarity in the sense that matters is also a matter of date. Zig's standard library has moved under the models' feet, and they write the version they learned.

For a developer choosing a language for an agent, the results point away from both slogans. A capable agent will get the program right in any of these languages, and the strongest will write it as fast as our references or faster. The standard library and the runtime's defaults decide how much the agent must build or work around, and so how often a weaker agent gets it right; the language's popularity decides how much the session costs; and the language's performance ceiling decides how fast the result runs, whichever agent writes it.

= Related work <related>

Multilingual benchmarks of code generation translate function-level problems into many languages and score single completions. MultiPL-E @cassano2023multiple translated two Python benchmarks into eighteen languages and found performance correlated with a language's popularity, with some niche languages doing as well as popular ones, and static type checking neither helping nor hurting. Multi-LCB @multilcb2026 extended a contest benchmark to twelve languages and found evidence of overfitting to Python. These studies fix the setting our questions turn on: a single completion has no compiler to consult. That MultiPL-E's verdict on type checking survives the move to agents, which do consult one, is one of our results.

Agent studies of 2026 moved to the loop. Wu, Anderson and Guha @wu2026tokenmaxxing ran five models in Python, Java, Rust and OCaml and found accuracy almost independent of language while OCaml cost 1.28 to 1.69 times Python's tokens. Acher and Jézéquel @acher2026chess had Claude Code and Codex write chess engines in seventeen languages, one engine per language, and measured their strength. Endoh's benchmark @endoh2026langbench repeated one task twenty times in thirteen languages with one model, reported 597 passes in 600 runs, and found that adding a type checker made sessions 1.6 to 3.2 times slower. Our study adds a sweep over reasoning effort, nine models from four vendors, the measured speed of every passing program, Zig, and a reading of the sessions themselves, which is what lets us say why the languages differ where they do.

The question of what static types buy predates agents. Gao, Bird and Barr @gao2017type found that TypeScript and Flow would have caught 15% of the public bugs they studied in JavaScript projects. Ray and colleagues @ray2014large associated language with defect rates across GitHub projects, and Berger and colleagues @berger2019impact showed in a reproduction that most of those associations do not survive careful analysis. Our JavaScript and TypeScript comparison asks the same question of agents and finds no difference in correctness. On measurement, we follow the advice of Mytkowicz and colleagues @mytkowicz2009wrong to control the environment that a timing silently depends on, and we time every program against a reference on isolated, clock-pinned cores.

The harness itself matters: the same model scores differently in different agents @merrill2026terminalbench, and rankings of complete systems are far more stable than rankings of bare models @hardy2026reliability. We therefore compare languages within a system and report each model through the agent it ran in.

= Conclusion

We asked whether the language a coding agent writes in changes whether its program is correct, what it costs and how fast it runs, and whether a strict compiler or a large body of training data explains the differences. For #count(strong.len()) of the #count(models.len()) models in our study, correctness did not depend on the language: each passed every hidden test in nearly all trials in all eight languages. For the two weakest, the differences followed the standard library and the runtime, not the type system. Across all the models, pairs of languages that differ only in their checking passed equally often, and Python, the most familiar language, trailed JavaScript because its runtime's limits crashed the weaker models' programs and its speed timed out the stronger models'.

The strict compiler was a crutch only where the model's knowledge was out of date. The agents seldom tripped Rust's compiler, and in Zig they leaned on it to unlearn a standard library that had changed, at #times(lo-of(strong, zig-tokens)) to #times(hi-of(models, zig-tokens)) the cost. The training data showed up as price mostly at that far end; among the mainstream languages, Rust cost the stronger models at most #more(rust-strong) more than Python or JavaScript, and #lname.at(lead) comes first on the gungnir index#if same-lead [ for every model]. The language an agent writes in decides what a correct program costs and how fast it runs; whether the agent gets it right depends on the agent.
