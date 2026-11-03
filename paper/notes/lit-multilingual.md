# Related work: multilingual code generation

Collected 2026-11-03. "fetched" entries were opened on the primary page; "snippet-only" entries were not. Check every citation against the primary source before it goes into the paper.

I found no paper that already does this study. Correctness-only "how do LLMs do across languages" is well covered, so on its own it won't be new. What's open is putting together a within-model reasoning-effort sweep, many languages including Zig and x86-64 assembly, repeated runs, measured runtime of the produced code, complexity, and cost/tokens. The three closest 2026 works each cover a different slice, and this paper should cite them:

- Endoh's mame/ai-coding-lang-bench: 13 languages, 20 runs each, wall-clock time, cost and lines of code, but one model, no effort sweep, no runtime measurement of the produced code, and no Zig or assembly.
- Wu, Anderson & Guha, "Tokenmaxxing" (arXiv 2607.22807): agentic, tokens per language, 5 current models, but 4 languages and a fixed effort per model.
- Acher & Jézéquel's chess-engine study (arXiv 2606.13763): 17 languages including x86-64 assembly, with cost and tokens, but one run per engine and no effort sweep.

---

## Sources found

**Status:** "fetched" means the URL shown was opened. "snippet-only" means it was seen only in search results.

### A. Classic multilingual function-level benchmarks (2022–2024)

These were all tested on non-reasoning models, single-shot, and none varies reasoning effort.

**1. MultiPL-E: A Scalable and Polyglot Approach to Benchmarking Neural Code Generation**
- **Year / venue:** 2022/2023, arXiv 2208.08227. Published in IEEE TSE 49(7), 2023; the venue comes from search snippets.
- **URLs (fetched):** https://arxiv.org/abs/2208.08227, https://ar5iv.labs.arxiv.org/html/2208.08227, https://github.com/nuprl/MultiPL-E
- **Languages:** Python plus 18 translated targets: Bash, C++, C#, D, Go, Java, JS, Julia, Lua, Perl, PHP, R, Racket, Ruby, Rust, Scala, Swift, TS. The repo later added Ada, Dart, Clojure and Elixir.
- **Models:** Codex, CodeGen, InCoder.
- **Metrics:** pass@k only, from 200 samples at T=0.2 and T=0.8.

**2. CodeGeeX / HumanEval-X**
- **Year / venue:** 2023, arXiv 2303.17568. KDD 2023 per a snippet (DOI 10.1145/3580305.3599790).
- **URL (fetched):** https://arxiv.org/abs/2303.17568
- **Languages:** Python, C++, Java, JS, Go, with 820 hand-written solutions.
- **Models / metrics:** CodeGeeX against models of similar size; pass@k.

**3. MBXP / Multilingual HumanEval / MathQA-X ("Multi-lingual Evaluation of Code Generation Models")**
- **Year / venue:** arXiv 2210.14868. ICLR 2023 per a snippet from iclr.cc.
- **URLs (fetched):** https://arxiv.org/abs/2210.14868, https://arxiv.org/html/2210.14868
- **Languages:** MBXP covers 13: Python, Java, JS, TS, Go, Ruby, Kotlin, PHP, C#, Scala, C++, Swift, Perl. Prompts and tests were converted from Python by rule-based translation.
- **Models:** The authors' own 125M–13B models, plus InCoder.
- **Metrics:** pass@k.

**4. BabelCode / TP3 ("Measuring The Impact Of Programming Language Distribution")**
- **Year / venue:** arXiv 2302.01973. ICML 2023 per the arXiv comment.
- **URL (fetched):** https://arxiv.org/abs/2302.01973
- **Languages:** 14. The page does not list them; it names Go and Rust as examples.
- **Models:** The authors' own trained LLMs.
- **Metrics:** pass@k. The framework also records memory and runtime.

**5. xCodeEval**
- **Year / venue:** arXiv 2303.03004. ACL 2024 per an ACL Anthology snippet.
- **URL (fetched):** https://arxiv.org/abs/2303.03004
- **Languages:** 11 for execution-based tasks (17 for retrieval). The page I opened didn't list them.
- **Models:** OpenAI models and open LLMs.
- **Metrics:** Execution against unit tests (ExecEval engine).

**6. McEval: Massively Multilingual Code Evaluation**
- **Year / venue:** arXiv 2406.07436. ICLR 2025 per the proceedings snippet.
- **URL (fetched):** https://arxiv.org/html/2406.07436
- **Languages:** 40 (AWK through VimScript). No Zig, no assembly.
- **How built:** Written by human annotators, not translated from Python.
- **Models:** 23 models, including GPT-4o.
- **Metrics:** Greedy pass@1.

**7. CRUXEval-X**
- **Year / venue:** arXiv 2408.13001, ACL 2025.
- **URLs (fetched):** https://arxiv.org/abs/2408.13001, https://arxiv.org/html/2408.13001v2
- **Languages:** 19 (MultiPL-E's set plus Shell).
- **Task:** Code reasoning (predicting input or output), not generation.
- **Models / metrics:** 24 models, none of them reasoning models; pass@1.

**8. MultiPL-T (Knowledge Transfer from High- to Low-Resource PLs)**
- **Year / venue:** arXiv 2308.09895, OOPSLA 2024 (PACMPL). **snippet-only.**
- **Content:** Fine-tuning work for Julia, Lua, OCaml, R and Racket, evaluated on MultiPL-E.

### B. 2025–2026 problem-level studies that include reasoning models

**9. AutoCodeBench**
- **Year / venue:** 2025, arXiv 2508.09101. I couldn't confirm a venue.
- **URLs (fetched):** https://arxiv.org/abs/2508.09101, https://arxiv.org/html/2508.09101
- **Content:** 3,920 problems in 20 languages, generated by an LLM and checked in a sandbox. No Zig or assembly.
- **Models:** More than 30, including o3, o4-mini, Claude Opus 4 and Sonnet 4, Gemini 2.5 Pro and GLM.
- **Effort:** Reasoning modes are reported as on vs. off only. There is no effort sweep.
- **Setting / metrics:** Single-shot pass@1. One section adds multi-turn repair with sandbox feedback (DeepSeek-V3 went from 48.1% to 59.7% after 3 turns).

**10. Multi-LCB: Extending LiveCodeBench to Multiple Programming Languages**
- **Year / venue:** arXiv 2606.20517, ICLR 2026 poster.
- **URLs (fetched):** https://arxiv.org/abs/2606.20517, https://arxiv.org/html/2606.20517, https://iclr.cc/virtual/2026/poster/10009956. The OpenReview page was blocked by a browser check.
- **Languages:** 12: C++, C#, Python, Java, Rust, Go, TS, JS, Ruby, PHP, Kotlin, Scala.
- **Models:** 24 open models only. gpt-oss was run at Low and Medium reasoning, which is the only partial effort variation in this group.
- **Metrics:** pass@1 averaged over 10 runs with standard deviations.
- **Finding:** Python overfitting. For example, one model scored 77.8% on Python but 10.7% on Rust (figures from a secondary page, benchmarklist.com).

**11. Agnostics / Ag-LiveCodeBench-X**
- **Year / venue:** arXiv 2508.04865. ICLR 2026 per the arXiv comment.
- **URL (fetched):** https://arxiv.org/abs/2508.04865
- **Content:** RL training for Lua, Julia, R, OCaml and Fortran on small Qwen and DeepSeek models. Released a multi-language version of LiveCodeBench; metric is pass@1.

**12. EffiBench-X**
- **Year / venue:** arXiv 2505.13004, NeurIPS 2025 Datasets & Benchmarks track.
- **URLs (fetched):** https://arxiv.org/abs/2505.13004, https://arxiv.org/html/2505.13004
- **Content:** 623 tasks in 6 languages (Python, C++, Java, JS, Ruby, Go).
- **Models:** 26, including DeepSeek-R1, Qwen3 and Gemini 2.5. Run at temperature 0, with no effort sweep.
- **Metrics:** Execution time, peak memory and memory integral relative to human solutions, plus pass@1.

**13. XL-CoGen ("Multi-Agent Strategies for Bridging Programming Language Gaps")**
- **Year / venue:** arXiv 2509.19918. The version date on the page looks inconsistent.
- **URL (fetched):** https://arxiv.org/html/2509.19918
- **Content:** 17 languages; GPT-4.1-mini, DeepSeek-V3.2 and Gemini 2.5 Pro.
- **Setting / metrics:** Multi-agent pipeline with test feedback; pass@1, partial pass and error rate.

**14. PROBE**
- **Year / venue:** arXiv 2607.13820. Accepted at Empirical Software Engineering, per the arXiv comment.
- **URL (fetched):** https://arxiv.org/abs/2607.13820
- **Languages:** 5. The Hugging Face dataset card (snippet) lists Python, C++, Java, C and Rust.
- **Models:** 6, with 3 prompting strategies.
- **Metrics:** Correctness, closeness to a valid solution, and code quality.

**15. EsoLang-Bench**
- **Year / venue:** arXiv 2603.09678.
- **URL (fetched):** https://arxiv.org/abs/2603.09678
- **Content:** 80 problems in Python, JS and 5 esoteric languages.
- **Finding:** Frontier models score 100% on Python and JS but 0–11% on the esoteric versions.

**16. Programming Language Confusion** (arXiv 2503.13620) — **snippet-only.**
- Models drift toward Python and toward languages with similar syntax.

**17. LangChoiceBench** (arXiv 2608.06041) — **snippet-only.**
- Measures how often models choose Python when another language would suit the project better.

### C. Agentic and repository-level benchmarks

**18. Aider Polyglot**
- **Year:** December 2024.
- **URLs (fetched):** https://aider.chat/2024/12/21/polyglot.html, https://aider.chat/docs/leaderboards/, https://aider.chat/docs/benchmarks.html, https://epoch.ai/benchmarks/aider-polyglot, https://github.com/Aider-AI/polyglot-benchmark
- **Content:** 225 Exercism exercises in C++, Go, Java, JS, Python and Rust.
- **Setting:** Two attempts. After a failed first attempt the model sees the test output (truncated to 50 lines in the original benchmark docs).
- **Metrics:** Percent correct, cost, edit-format accuracy.
- **Partial effort data:** GPT-5 low / medium / high scored 81.3% / 86.7% / 88.0% at $10.37 / $17.69 / $29.08. These are aggregate figures only; I saw no per-language breakdown on the leaderboard.
- **Recency:** Last updated 20 November 2025.

**19. Multi-SWE-bench**
- **Year / venue:** arXiv 2504.02605. NeurIPS 2025 Datasets & Benchmarks per a snippet; that version has 8 languages including Python and 2,132 instances.
- **URLs (fetched):** https://arxiv.org/abs/2504.02605, https://arxiv.org/html/2504.02605
- **arXiv version:** 7 languages (Java, TS, JS, Go, Rust, C, C++), 1,632 instances.
- **Models:** 9, including o1, o3-mini-high, DeepSeek-R1 and Claude 3.7.
- **Agents:** Agentless, SWE-agent, OpenHands.
- **Metrics:** Resolved rate, cost, tokens. Runs appear to be single.

**20. SWE-bench Multilingual**
- **URL (fetched):** https://www.swebench.com/multilingual.html. The page asks readers to cite SWE-smith (arXiv 2504.21798, NeurIPS 2025 Datasets & Benchmarks).
- **Content:** 300 tasks in 9 languages.
- **Models:** Only Claude 3.7 Sonnet with SWE-agent, capped at $2.50 per task.
- **Per-language results:** Rust 58%, C/C++ 29%. Patch size did not explain the differences.

**21. SWE-PolyBench**
- **Year / venue:** arXiv 2504.08703.
- **URLs (fetched):** https://arxiv.org/abs/2504.08703, https://arxiv.org/html/2504.08703
- **Content:** 2,110 instances in Java, JS, TS and Python.
- **Models / agents:** 3 agents on Claude 3.5 Sonnet, plus others.
- **Metrics:** Pass rate and tokens, with bootstrap standard errors. The paper treats agents as deterministic.

**22. LiveRepoReflection** (arXiv 2507.09866, EMNLP 2025 Findings) — **snippet-only.**
- 6 languages, built from Exercism.

**23. SWE-rebench V2** (arXiv 2602.23866, ICML 2026) — **fetched** (https://arxiv.org/abs/2602.23866).
- 32k training tasks in 20 languages. The page does not list the languages.

**24. OdinEval** (arXiv 2608.18595) — **fetched** (https://arxiv.org/abs/2608.18595).
- Program repair in Odin, a low-resource systems language and the closest analogue to Zig I found.

### D. 2026 "language gap" studies of frontier agents (the most relevant group)

**25. The Best Programming Language for Tokenmaxxing** (Wu, Anderson, Guha)
- **Year / venue:** arXiv 2607.22807.
- **URLs (fetched):** https://arxiv.org/abs/2607.22807, https://arxiv.org/html/2607.22807
- **Languages:** Python, Java, Rust, OCaml.
- **Models:** Qwen3.6, Gemma-4, GLM-5.2 (high), GPT-5.5 (medium), Sonnet 4.6 (10K thinking budget). Effort is fixed per model and differs between models.
- **Setting:** mini-swe-agent on 100 LiveCodeBench problems.
- **Findings:** Accuracy barely depends on language (GPT-5.5: 100/99/98/97%). OCaml costs 1.28–1.69× Python's tokens.
- **Metrics:** Tokens and wall-clock time, analysed with mixed-effects models. Repeated runs are not stated.

**26. Do programming languages still matter to your AI coding agent teammate? Evidence at scale from chess engines** (Acher, Jézéquel)
- **Year / venue:** arXiv 2606.13763.
- **URLs (fetched):** https://arxiv.org/abs/2606.13763, https://arxiv.org/html/2606.13763
- **Languages:** 17, including x86-64 assembly. No Zig.
- **Models:** Claude Code with Opus 4.6/4.7 (extended thinking) and Codex with gpt-5.x (high effort).
- **Metrics:** Move-generation correctness (perft), externally measured Elo, normalised USD cost, tokens, lines of code, some nodes/s figures.
- **Design:** One engine per data point; no repeats.
- **Assembly engine:** Built by Codex; claimed Elo 2481, measured 1403 ± 196.

**27. Frontier Coding Agents Use Metaprogramming to Adapt to Unfamiliar Programming Languages**
- **Year / venue:** arXiv 2606.10933.
- **URL (fetched):** https://arxiv.org/abs/2606.10933
- **Content:** 6 agents (Opus 4.6, GPT-5.4 xhigh, and others) on 4 esoteric languages.
- **Finding:** The strongest agents write Python programs that generate the target-language code. Banning this causes large drops.

**28. mame/ai-coding-lang-bench** (Yusuke Endoh, March 2026; grey literature)
- **URL (fetched):** https://github.com/mame/ai-coding-lang-bench. The dev.to post is snippet-only.
- **Content:** Claude Code with Opus 4.6 (effort not stated) builds a mini-git in 13 languages (15 configurations), 20 runs per phase.
- **Metrics:** Pass rate, wall-clock ± SD, cost, LOC. No runtime performance of the generated programs, no Zig or assembly.
- **Results:** 597 of 600 runs passed. Type checkers made runs 1.6–3.2× slower.

**29. Kian-hdr/agentic-coding-language-benchmark** (October 2026; grey literature)
- **URL (fetched):** https://github.com/Kian-hdr/agentic-coding-language-benchmark
- **Content:** Codex with "gpt-6.1-sol" at medium effort, 9 languages (TS, Python, Go, Rust, Swift, Kotlin, MoonBit, BAML, Zerolang).
- **Metrics:** CPU time and peak memory (RSS) of the produced programs.
- **Design:** n ≈ 2, single host.

**30. Kun Chen, "Evaluating the Effectiveness of Programming Languages for Agents"** (June 2026 blog)
- **URL (fetched):** https://blog.kunchenguid.com/p/evaluating-the-effectiveness-of-programming
- **Content:** ProgramBench (arXiv 2605.03546, snippet-only) run with GPT-5.5 Codex in 8 languages plus a free-choice arm.
- **Design / metrics:** 1 sample per task per arm; Wilcoxon tests; pass rate, $ per task, turns.

**31. Svetkin, "Comparing LLMs' Coding Abilities Across Programming Languages"** (HackerNoon, March 2026)
- **URL (fetched):** https://hackernoon.com/comparing-llms-coding-abilities-across-programming-languages
- **Content:** 5 models on 100 recent LeetCode problems in Python, Java, Rust and Elixir.
- **Finding:** Only the Elixir gap was statistically significant.

**32. Akita, "AI Agents: Comparing the Top LLMs of 2026 on the Zig Challenge"** (January 2026)
- **URL (fetched):** https://akitaonrails.com/en/2026/01/11/ai-agents-comparing-top-llms-on-the-zig-challenge/
- **Content:** One anecdotal Zig 0.15 migration task (with llama.cpp bindings) across 8 models, with cost and time. **This is the only Zig LLM evaluation I found anywhere.**

### E. Assembly-related work

**33. SuperCoder: Assembly Program Superoptimization with Large Language Models**
- **Year / venue:** arXiv 2505.11480. COLM 2026 per one search snippet only.
- **URL (fetched):** https://arxiv.org/html/2505.11480
- **Task:** Optimise existing gcc -O3 x86-64 assembly. Models are given the C source and the baseline assembly.
- **Models:** 23, including Claude Opus 4 (51.5% of tests passed, 1.43× speedup), GPT-5 and o4-mini.
- **Finding:** Generating assembly from C alone collapses.

**34. CompilerEval ("Exploring the Feasibility of End-to-End LLM as a Compiler")**
- **Year / venue:** arXiv 2511.04132. IJCNN 2025 per the arXiv comment.
- **URL (fetched):** https://arxiv.org/abs/2511.04132
- **Content:** LLMs compiling source code to assembly. Snippets say x86 was harder than ARM and RISC-V.

**35. Snippet-only:**
- NeuComBack (arXiv 2511.01183): LLVM IR to assembly.
- LEGO-Compiler (arXiv 2505.20356): C to x86/ARM/RISC-V assembly.
- Meta LLM Compiler (arXiv 2407.02524).
- CodegenBench (arXiv 2606.04023): HPC code across architectures.

---

## Coverage map

Y = yes, P = partial, N = no.

| Work | Many langs | Zig | Asm | Agentic | Effort sweep | Runtime perf | Complexity/quality | Cost/tokens | Repeats + variance |
|---|---|---|---|---|---|---|---|---|---|
| MultiPL-E / MBXP / HumanEval-X / McEval / xCodeEval | Y | N | N | N | N | N | N | N | N (pass@k sampling only) |
| BabelCode | Y | N | N | N | N | P (harness records it) | N | N | N |
| CRUXEval-X | Y (reasoning, not generation) | N | N | N | N | N | N | N | N |
| AutoCodeBench | Y (20) | N | N | P (repair ablation) | P (thinking on/off) | N | N | N | N |
| Multi-LCB | Y (12) | N | N | N | P (gpt-oss Low/Med) | N | N | N | Y (10 runs ± SD) |
| EffiBench-X | P (6) | N | N | N | N | Y (relative to human) | N | N | N |
| PROBE | P (5) | N | N | N | N | N | Y | N | ? |
| Aider Polyglot | P (6) | N | N | P (2 tries) | P (some models, aggregate only) | N | N | Y cost | N |
| Multi-SWE / SWE-bench Multilingual / PolyBench | P–Y | N | N | Y | N | N | N | Y / P | N |
| Tokenmaxxing (2607.22807) | P (4) | N | N | Y | N (fixed, differs per model) | N | N | Y tokens, time | ? |
| Chess engines (2606.13763) | Y (17) | N | Y (1 engine) | Y | N | P (Elo, some nodes/s) | P (LOC) | Y | N |
| mame lang-bench | Y (13) | N | N | Y | N | N | P (LOC) | Y cost, time | Y (20 runs ± SD) |
| Kian-hdr bench | Y (9) | N | N | Y | N | Y (CPU, RSS) | N | N | P (n≈2) |
| SuperCoder | N | N | Y (optimising given asm) | P | N | Y (speedup) | N | N | P (best-of-8) |
| Akita Zig blog | N | Y (anecdotal) | N | Y | N | N | N | P | N |
| **This study** | Y | Y | Y | ? | Y | Y | Y | Y | Y |

---

## Gaps

1. **No formal evaluation includes Zig.** I searched arXiv, OpenReview, ACL and ACM and found none. The only Zig evidence is Akita's anecdotal blog post. The closest formal analogue is OdinEval. Zig is also absent from MultiPL-E, McEval (40 languages), AutoCodeBench, Multi-LCB, Aider and the SWE-bench variants.
2. **No work has frontier models write x86-64 assembly from a plain-language task description and grades it with hidden tests across models.** Existing assembly work is compiling (C or IR to assembly) or optimising given assembly (SuperCoder). The only from-scratch case is a single Codex chess engine.
3. **No within-model reasoning-effort sweep is reported per language.** Effort varies only incidentally:
   - Aider's GPT-5 low/medium/high, aggregated over 6 languages.
   - Multi-LCB's gpt-oss Low/Medium, open models only.
   - AutoCodeBench's thinking on vs. off.
   - "Does more reasoning close the low-resource-language gap?" A targeted search found no direct study.
4. **No work combines all of these:** cross-language correctness for current frontier models, measured runtime of the produced code (hyperfine-grade), cyclomatic complexity, and cost/tokens, with repeated runs. EffiBench-X measures runtime against human baselines but uses older models at temperature 0 with single runs. Kian-hdr measures CPU time and memory but with one model, n≈2, and no Zig or assembly.
5. **Repeated runs with variance are rare** (only Multi-LCB and mame). Most agentic studies use n=1.

**Warning:** for frontier models, correctness is near ceiling in mainstream languages. Tokenmaxxing got 97–100% for GPT-5.5, and mame got 597 of 600 runs passing. Hand-written hidden-test scenarios may show almost no correctness gap between Python, Go, Java, TS and similar languages. The interesting differences are likely to appear in tokens, cost and time, in Zig and assembly, and at low effort levels. Design the tasks to be hard enough.

---

## Non-obvious pitfalls these papers report

- **Bias from translating Python tasks.**
  - Python's `Any` and `Optional` types and inferred types don't map cleanly, so 5 HumanEval problems fail to compile in typed languages (MultiPL-E).
  - Doctests are left in Python format, and wording like "list" leaks across languages.
  - Different numbers of problems survive per language: CRUXEval-X kept 752–800 of 800, with a final aligned set of 500.
- **Format confounds.** Forcing everything into stdin/stdout I/O produces parsing and format failures that are not algorithmic errors. Multi-LCB also doesn't control language dialects or versions.
- **Python overfitting and per-language contamination.** A model's Python score does not predict its other scores (Multi-LCB).
- **Language confusion.** Models emit the wrong language, drifting toward Python (MBXP; arXiv 2503.13620).
- **Agents escape into Python.** Agents prototype in Python or write Python programs that generate the target code (2606.10933, 2607.22807). If Python is installed in the sandbox, the Zig or assembly measurement can be contaminated. Decide whether to allow it and record it either way.
- **Agents overspend.** They distrust the given tests, invent their own inputs, and revise solutions that already pass, which inflates token counts. Token counts are heavy-tailed, so use mixed-effects models with the problem as a random effect (2607.22807).
- **Cheating and leakage.**
  - A chess engine silently imported python-chess.
  - A self-reported Elo was 1,078 points too high (2606.13763).
  - One 2026 harness write-up (snippet-only) found a harness reading the hidden grader file in 14 of its runs.
- **Cost comparability.** Prices differ by vendor. The chess paper normalised to $1 / $5 per million input / output tokens; Tokenmaxxing reported no dollar figures at all. Reasoning budgets differ between models, so within-model language comparisons are cleaner than cross-model ones.
- **Efficiency is entangled with correctness.** Failed solutions score 0, and runtime is relative to each language's own baseline (EffiBench-X). Kian-hdr's CPU time includes startup and JIT (just-in-time compilation), which hits JVM and .NET languages and Node.
- **Benchmark selection bias.** Aider kept the 225 problems that 3 or fewer of 7 particular models solved. AutoCodeBench's filtering model is weaker on low-resource languages, which may leave easier problems in those languages.
- **Zig and assembly specifics.**
  - Zig is pre-1.0 with API churn: 0.15 broke ArrayList and the build system API, and 0.16 shipped in April 2026 (snippet). Pin the version and state it in the prompt.
  - For assembly, fix the assembler (NASM vs. GAS), syntax (Intel vs. AT&T) and ABI (System V). x86 was reported harder than ARM and RISC-V (CompilerEval).
- **Type checkers add agent time.** mypy and Steep made runs 1.6–3.2× slower (mame).

---

## Open questions

- **Multi-LCB:** I couldn't see how it measures per-language contamination. OpenReview was blocked by a browser check.
- **Venues:** Unconfirmed for AutoCodeBench and SWE-PolyBench. SuperCoder's COLM 2026 comes from a single snippet.
- **Odd version dates:** EffiBench-X ("Oct 2026") and XL-CoGen. xCodeEval's 11 languages weren't listed on the page I opened.
- **Missing details:**
  - Tokenmaxxing: whether runs were repeated.
  - Chess paper: wall-clock time.
  - mame: the effort setting used.
  - Aider: whether per-language results exist anywhere.
- **Grey literature:** Kian-hdr's "gpt-6.1-sol" alias can't be verified.
- **Unread pages:** I couldn't access the Springer or ScienceDirect versions (PROBE, Neurocomputing 2025). MultiPL-T, ProgramBench, LangChoiceBench and the other assembly papers are snippet-only.
- **Target models:** No multilingual paper I found evaluates the GPT-6/6.1 family, GLM 5.3 or Claude 5.x. The closest are GLM-5.2, GPT-5.5 and Opus 4.6/4.7.
- **Zig:** The absence of formal Zig evaluations rests on search-engine coverage. Unindexed or very recent work may exist.