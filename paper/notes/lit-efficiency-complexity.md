# Related work: efficiency and complexity of generated code

Collected 2026-11-03. "fetched" entries were opened on the primary page; "snippet-only" entries were not. Check every citation against the primary source before it goes into the paper.

## Bottom line

No paper I found already does most of this study. Efficiency across many languages is partly covered, by one benchmark with six languages. Complexity across languages is covered only thinly. A reasoning-effort sweep has been measured against code complexity and cost (in Java only), but nobody has measured it against how fast the generated code runs.

- **Closest overlap: EffiBench-X** (NeurIPS 2025 D&B). It covers 6 languages and compares runtime and memory to a human expert solution in the same language. It has no Rust, C, TypeScript or Zig, no reasoning-effort levels, no complexity metrics, no cost or tokens, and is single-shot.
- **Closest on effort × complexity: Sonar's GPT-5 study** (vendor blog). It sweeps reasoning effort and reports complexity, size and cost, but only on Java, with no runtime and no repeated runs.

## Sources found

### A. Efficiency benchmarks for LLM-generated code

**1. EffiBench-X: A Multi-Language Benchmark for Measuring Efficiency of LLM-Generated Code** (2025; v2 posted 27 Oct 2026). Fetched.
- **Where:** arXiv 2505.13004, comments say NeurIPS 2025 D&B Track. Opened https://arxiv.org/abs/2505.13004, https://arxiv.org/html/2505.13004v1 and https://arxiv.org/html/2505.13004v2.
- **Languages:** Python, C++, Java, JavaScript, Ruby, Go. 623 tasks from Aizu, AtCoder, CodeChef, Codeforces and LeetCode, released after Oct 2023. Each task has one human-expert solution per language.
- **Models:** 26–28 (the paper says 26, the v2 table lists 28). Includes DeepSeek-R1/V3, Qwen3, QwQ, Llama-4, Gemma-3, Phi-4 (plain and reasoning), GPT-4o/4o-mini, Claude 3.5 Haiku / 3.7 Sonnet, Gemini 2.0/2.5. No GPT-5 or later, no Claude 4 or later, no GLM, no effort levels.
- **Normalization:**
  - Per problem, time score = clip(T_human / T_LLM, 0, 1). Memory peak and memory integral (∫M(t)dt) use the same form.
  - The score is averaged over problems as a percentage. A failing solution scores 0.
  - Because of the clip, code faster than the human scores only 1.
- **Measurement:** AWS i7ie.metal, Docker with workers pinned to cores, sampling every 0.1 ms, 10 s timeout, 1 GiB memory limit. C++ was built with gcc 14.2 `-O2 -fsanitize=address`. Java, Node (`--harmony`), Ruby, Go and Python ran with default settings. Temperature 0 (v2).
- **Repeat runs:** only a 3-run reliability check for two models. DeepSeek-R1 time score 62.45% (range 61.33–63.55).
- **Setting:** single-turn generation. An appendix tests iterative refinement with performance feedback (EffiLearner).
- **Findings:** best model reaches about 62% of expert efficiency (Qwen3-32B). Models are relatively more efficient in Python, Ruby and JavaScript than in Java, C++ and Go.

**2. EffiBench: Benchmarking the Efficiency of Automatically Generated Code** (2024). Fetched.
- **Where:** arXiv 2402.02037. Opened https://arxiv.org/html/2402.02037v1. No venue stated on the page.
- **Scope:** Python only, 1,000 LeetCode problems. v1 tested 21 models (13 open, 8 closed); later versions report 42.
- **Metrics:** ET, MU and TMU (memory integrated over time by the trapezoidal rule). The normalized versions NET, NMU and NTMU are the arithmetic mean of generated/canonical ratios, where above 1 means slower than the human solution.
- **Measurement:** mean of 5 executions with standard deviation, on a Xeon 4216 server. 100 generated tests per problem.
- **Setting:** few-shot prompt, single generation.

**3. Mercury: A Code Efficiency Benchmark for Code LLMs** (2024). Fetched (arXiv). Venue NeurIPS 2024 D&B is snippet-only, from the proceedings URL.
- **Where:** arXiv 2402.07844. Opened https://arxiv.org/html/2402.07844v4.
- **Scope:** Python only, 1,889 tasks (256 in the eval split, about 18 human solutions per task). 10 open models from 1.3B to 34B.
- **Beyond metric:** p = (max R − clip(r, min R, max R)) / (max R − min R), averaged over N tasks and K = 5 samples. R is the set of runtimes of historical human solutions. A failing solution counts as r = ∞, which gives 0.
- **Measurement:** sandbox, single-threaded, 30 s timeout. Temperature 0.2, single-shot.

**4. Evaluating Language Models for Efficient Code Generation (DPE / EvalPerf)** (2024). Fetched (arXiv). Venue COLM 2024 is snippet-only.
- **Where:** arXiv 2408.06450. Opened https://arxiv.org/html/2408.06450v1.
- **Scope:** Python, about 118–121 tasks from HumanEval+ and MBPP+. Models: CodeLlama, DeepSeekCoder, StarCoder 1/2, GPT-4 Turbo.
- **Metric (DPS):** reference solutions are clustered into performance levels. The score is the cumulative ratio of the nearest reference that is slower than the new solution: DPS = max({0} ∪ {r_i | t_i > t*}).
- **Measurement:** instruction counts from hardware counters (Linux perf_event), not wall-clock time. The authors cite run-to-run variance of only a few hundred instructions on runs of billions.
- **Inputs:** an LLM writes an input generator, scaled up in size by doubling until a 20 s or 16 GB limit.
- **Setting:** single-shot. Prompt variants are plain, "write a fast implementation", and "think step by step".

**5. How Efficient is LLM-Generated Code? A Rigorous & High-Standard Benchmark (ENAMEL)** (2024). Fetched (arXiv). Venue ICLR 2025 is snippet-only.
- **Where:** arXiv 2406.06647. Opened https://arxiv.org/html/2406.06647v4.
- **Scope:** Python, 142 HumanEval-derived problems, 30 models.
- **Metric (eff@k):**
  - Per test level, f = (T − max_m t)⁺ / (T − max_m t*), where t* is the expert reference time.
  - The time limit is T = α·max t*, with α = 2.
  - Levels are weighted by hardness, and the score is 0 if the code is incorrect.
  - Uses an unbiased Rao–Blackwellized estimator and treats timeouts as censored.
- **Measurement:** each test run R = 6 times, summarized with the outlier-robust Hodges–Lehmann estimator. Single-shot sampling.

**6. ECCO: Can We Improve Model-Generated Code Efficiency Without Sacrificing Functional Correctness?** (2024). Fetched (arXiv). Venue EMNLP 2024 is snippet-only, from the ACL Anthology.
- **Where:** arXiv 2407.14044. Opened https://arxiv.org/html/2407.14044v2.
- **Scope:** Python only, built on IBM CodeNet. Models: CodeLlama, DeepSeekCoder, CodeGemma, WizardCoder, StarCoder2, GPT-4o.
- **Metrics:**
  - Editing task: speedup = runtime(input program) / runtime(output program).
  - Generation task: runtime percentile among human submissions.
- **Measurement:** Judge0 on an AWS m7i.large. About ±1% 95% CI on runtime metrics across 3 generations.
- **Setting:** single-shot plus multi-turn refinement (self-refine, execution feedback) and fine-tuning.

**7. COFFE: A Code Efficiency Benchmark for Code Generation** (2025). Fetched.
- **Where:** arXiv 2502.02827. Abstract says accepted at FSE 2025. Opened https://arxiv.org/html/2502.02827v1.
- **Scope:** Python, 398 function-level and 358 file-level problems. 14 models (GPT-4o, Claude 3.5 Sonnet, Gemini 1.5 Pro, Llama 3.1 405B, DeepSeek V2, others).
- **Metric:** efficient@k = E[1 − C(n − c_f, k)/C(n, k)], where c_f counts samples that are correct and use fewer CPU instructions than the best ground-truth solution.
- **Why instructions:** relative standard deviation of execution time was 2.4–5.7%, versus 0.003–0.005% for instruction count. Correlation between the two was 0.96–1.00.

**8. Learning Performance-Improving Code Edits (PIE)** (2023/2024). Fetched.
- **Where:** arXiv 2302.07867, ICLR 2024 Spotlight. Opened https://arxiv.org/abs/2302.07867v5 and https://arxiv.org/html/2302.07867v5.
- **Scope:** C++, over 77k CodeNet pairs. Models: CodeLlama 7/13/34B, GPT-3.5, GPT-4.
- **Metrics:** speedup = old/new against the slow input program. %Opt requires the program to be correct and at least 10% faster.
- **Measurement:** gem5 simulator (Skylake model), GCC 9.3 `-O3`.
- **Key methodology finding:** they used hyperfine on 500 pairs of identical programs on real hardware. These showed an apparent mean "speedup" of 1.12× (SD 0.36), with the top 5% reaching 1.91×.
- **Setting:** Best@1 and Best@8 sampling.

**9. SWE-Perf: Can Language Models Optimize Code Performance on Real-World Repositories?** (2025). Fetched (arXiv). Venue ICML 2026 is snippet-only, from the poster listing.
- **Where:** arXiv 2507.12415. Opened https://arxiv.org/html/2507.12415v1.
- **Scope:** Python repositories, 140 instances.
- **Settings:** an "oracle" single-pass mode with 10 models (Claude 3.7/4, o1, o3, GPT-4o, DeepSeek R1/V3, Gemini 2.5 Pro, Qwen3), and agentic runs (Agentless, OpenHands with Claude 3.7).
- **Metric:** δ is the largest pessimistic runtime gain that stays significant under a one-sided Mann–Whitney U test (α = 0.1).
- **Measurement:** 20 runs after 3 warm-up tests, with IQR outlier removal. The text is inconsistent on core limits (1 core vs 5 cores).
- **Results:** expert 10.85%, best model 1.76%.

**10. GSO: Challenging Software Optimization Tasks for Evaluating SWE-Agents** (2025). Fetched (arXiv). Venue NeurIPS 2025 D&B is snippet-only.
- **Where:** arXiv 2505.23671. Opened https://arxiv.org/html/2505.23671v3.
- **Scope:** 102 tasks in 10 repositories. Code involves Python, C, C++, Cython and Rust; about 58.8% of tasks touch non-Python code.
- **Models:** GPT-4o, o3-mini, o4-mini, Claude 3.5v2 / 3.7 / 4.0, run agentically with OpenHands.
- **Metric:** speedup is the harmonic mean of per-test ratios (chosen over the geometric mean because of outliers). Opt@K requires at least 95% of the expert's speedup.
- **Cost:** not reported as tokens or dollars; the authors say cost limited their scaling experiments.

**11. SWE-fficiency: Can Language Models Optimize Real-World Repositories on Real Workloads?** (2025). Fetched.
- **Where:** arXiv 2511.06090, comments say ICML 2026. Opened https://arxiv.org/html/2511.06090.
- **Scope:** Python and Cython, 498 tasks. 15 models including Claude 4.5 Opus/Sonnet, GPT-5, GPT-5.2, Gemini 3, GLM-4.6. Harnesses: OpenHands and SWE-agent.
- **Metric:** SR = Speedup_model / Speedup_expert, aggregated by harmonic mean, with a floor of 0.001.
- **Measurement:** exclusive cores pinned per NUMA node. Instances kept only if the expert speedup exceeds 2× the measurement standard deviation. Reported SR variance under 0.5%.
- **Cost:** SWE-agent capped at $1 per task.
- **Inconsistency:** the abstract says "<0.23×" (Opus 4.5 scored 0.225); an earlier snippet said 0.15×.

**12. trace: Evaluating Execution Efficiency of LLM-Based Code Translation** (2026). Fetched.
- **Where:** arXiv 2603.16479, no venue. Opened https://arxiv.org/html/2603.16479v1.
- **Scope:** translation among C++, Java and Python; 28 models, including Claude-4 with and without thinking, o3 and DeepSeek-reasoner.
- **Metric:** Beyond-style min–max normalization against about 23 reference translations per task.
- **Measurement:** `perf stat` and psutil RSS sampling, mean of 5 runs, `taskset` pinning, coefficient of variation under 3%. C++ built with `g++ -O2`.
- **Reasoning finding:** compares reasoning vs non-reasoning variants (not effort levels) and concludes reasoning does not reliably improve efficiency.

**13. EvoCodeBench: A Human-Performance Benchmark for Self-Evolving LLM-Driven Coding Systems** (2026). Fetched.
- **Where:** arXiv 2602.10171, KDD 2026. Opened https://arxiv.org/html/2602.10171v1.
- **Scope:** Python3, C++, Java, Go, Kotlin. Models: GPT-5.2, Claude Opus/Sonnet 4.5, Gemini 3, Grok 4.1, DeepSeek-v3.2.
- **Metric:** LeetCode's own "beats %" for runtime and memory.
- **Setting:** single-pass, plus up to 3 rounds of self-revision.
- **Caveat:** no repeated runs and no cost.

**14. A Performance Study of LLM-Generated Code on Leetcode** (Coignion, Quinton, Rouvoy). Fetched.
- **Where:** EASE 2024, arXiv 2407.21579. Opened https://arxiv.org/html/2407.21579v1.
- **Scope:** Python, 18 models, 204 problems.
- **Measurement:** pytest-benchmark, at least 10 runs and at least 1 s total, one core.
- **Lessons:**
  - LeetCode timing has a coefficient of variation of 0.089, versus 0.035 when measured locally, and local vs LeetCode times correlate at only 0.28.
  - LeetCode's percentile rank drifts over time.
  - Temperature does not change mean runtime but correlates with variance (0.41).

**15. Improving Assembly Code Performance with LLMs via Reinforcement Learning** (2025). Fetched (arXiv). Venue NeurIPS 2025 is snippet-only.
- **Where:** arXiv 2505.11480. Opened https://arxiv.org/html/2505.11480v1.
- **Scope:** x86-64 assembly generated from CodeNet C programs. Baseline is gcc `-O3` output. 21 models.
- **Measurement:** hyperfine, discarding 3 warm-up runs and averaging the next 10.
- **Setting:** single-shot. Best model 1.47× over the baseline.

**16. Beyond Single LLMs: … Multi-Stage Performance-Guided LLM Orchestration (PerfOrch)** (2025). Fetched (abstract only).
- **Where:** arXiv 2510.01379. Opened https://arxiv.org/abs/2510.01379v1.
- **Scope:** Python, Java, C++, Go, Rust on HumanEval-X and EffiBench-X; 17 models. It is a method paper, and its measurement method is not described on the abstract page.

### B. Energy and performance across languages

**17. Energy Efficiency across Programming Languages** (Pereira et al.). Fetched (abstract page and repository).
- **Where:** SLE 2017, DOI 10.1145/3136014.3136031. Opened https://greenlab.di.uminho.pt/?p=217 and https://github.com/greensoftwarelab/Energy-Languages.
- **Scope:** 27 languages, 10 Benchmarks Game programs. The repository lists C, C++, C#, Go, Java, JavaScript, TypeScript, Python, Rust and others. Zig is not included.
- **Measurement:** RAPL plus `/usr/bin/time` for memory. A secondary blog says 10 runs per benchmark with median and mean reported.
- **Not verified:** I could not read the normalized results table (it is an image).

**18. Ranking Programming Languages by Energy Efficiency** (Pereira et al.). Snippet-only; the PDF was not machine-readable.
- **Where:** Science of Computer Programming 205, 2021.
- **Scope:** up to 27 languages, validated against Rosetta Code.

**19. It's Not Easy Being Green: On the Energy Efficiency of Programming Languages** (van Kempen, Kwon, Nguyen, Berger). Fetched.
- **Where:** arXiv 2410.05460, "to appear at ASE'25". Opened https://www.arxiv.org/abs/2410.05460 and https://ar5iv.labs.arxiv.org/html/2410.05460.
- **Critique of Pereira et al.:**
  - Implementations of the same benchmark differ across languages in algorithm, parallelism and libraries. Example: TypeScript mandelbrot is sequential while JavaScript uses about 28 cores.
  - C++ regex uses Boost, which alone explains the C vs C++ gap.
  - Idle cores add RAPL energy; the RAPL reading code had bugs; startup, JIT and GC effects were not controlled.
- **Finding:** energy is roughly power × time and does not depend on the language.

**20. AI-Powered, But Power-Hungry? Energy Efficiency of LLM-Generated Code** (Solovyeva, Weidmann, Castor). Fetched.
- **Where:** arXiv 2502.02412; no venue on the page. Opened https://arxiv.org/html/2502.02412.
- **Scope:** Python, Java, C++. Models: Copilot, GPT-4o, o1-mini. 53 LeetCode Hard problems, with one top-voted human solution per language as baseline.
- **Measurement:**
  - RAPL via perf, and powermetrics on macOS.
  - 10 runs; Java runs 13 times and drops the first 3.
  - 5 s cool-down between runs.
  - Mann–Whitney U and Hedges' g for statistics.
  - Built with `-O0`, which is a notable weakness for C++.

**21. A Controlled Experiment on the Energy Efficiency of the Source Code Generated by Code Llama** (Cursaru et al.). Snippet-only.
- **Where:** arXiv 2405.03616.
- **Scope:** C++, JavaScript, Python; 3 problems; Raspberry Pi with a Monsoon power monitor.

**22. Energy-Efficient Code Generation Using LLMs: A Systematic Literature Review** (2026). Fetched (abstract).
- **Where:** arXiv 2603.00989. Opened https://arxiv.org/abs/2603.00989.
- **Findings:** evidence is "limited and highly heterogeneous". Languages beyond the usual few are under-studied, and direct hardware measurement is rare.

### C. Complexity and quality of LLM-generated code

**23. Sonar, "How Reasoning Impacts LLM Coding Models"** (Sept 2025, vendor blog). Fetched.
- **Where:** https://www.sonarsource.com/blog/how-reasoning-impacts-llm-coding-models/
- **Scope:** Java only. GPT-5 at minimal, low, medium and high effort, plus GPT-4o. Analyzed with SonarQube.
- **Numbers by effort (minimal → high):**
  - Pass rate 75.4% → 81.8%
  - Lines of code 490k → 727k
  - Cost $22 → $189
  - Issues per passing task 3.90 → 5.50
  - Cognitive-complexity density flat at 0.227–0.233
- **Caveats:** single run per mode, no variance reported. Task count is inconsistent (the text says ">4,400" but the table lists 543 tasks).

**24. Sonar, "New data on code quality: GPT-5.2 high, Opus 4.5, Gemini 3…"** (Dec 2025). Fetched.
- **Where:** https://www.sonarsource.com/blog/new-data-on-code-quality-gpt-5-2-high-opus-4-5-gemini-3-and-more/
- **Scope:** Java, 4,000+ tasks. Reports cognitive complexity, lines of code and issue density per million lines. No repetitions.

**25. Assessing the Quality and Security of AI-Generated Code: A Quantitative Analysis** (Sabra, Schmitt, Tyler). Fetched (abstract).
- **Where:** arXiv 2508.14727. Opened https://arxiv.org/abs/2508.14727.
- **Scope:** Java, 4,442 tasks, 5 models, SonarQube.
- **Finding:** Pass@1 does not correlate with quality.

**26. Security and Quality in LLM-Generated Code: A Multi-Language, Multi-Model Analysis** (Kharma et al.). Fetched.
- **Where:** arXiv 2502.01853. Opened https://arxiv.org/html/2502.01853v1.
- **Scope:** Python, Java, C++, C. Models: Claude 3.5, Gemini 1.5, Codestral, GPT-4o, Llama-3. 200 tasks.
- **Metrics:** SonarQube lines of code, cyclomatic complexity and its density, cognitive complexity, maintainability. No runtime measurement.

**27. An Empirical Evaluation of GitHub Copilot's Code Suggestions** (Nguyen & Nadi). Snippet-only; the PDF was not readable.
- **Where:** MSR 2022.
- **Scope:** Python, Java, JavaScript, C on 33 LeetCode problems. SonarQube cyclomatic and cognitive complexity, reported medians about 5 and 6, with "no notable differences between languages".

**28. SlopCodeBench: Benchmarking How Coding Agents Degrade Over Long-Horizon Iterative Tasks** (2026). Fetched.
- **Where:** arXiv 2603.24755. Opened https://arxiv.org/html/2603.24755v1.
- **Scope:** agentic (Claude Code and Codex CLI), only the Python track evaluated. 11 models, including Opus 4.6, GPT-5.4 and GLM 4.7, all at high effort.
- **Metrics:** "erosion" = share of complexity mass (CC × √SLOC) held in functions with CC above 10; verbosity; dollars and minutes per checkpoint.
- **Findings:** quality erodes over iterations, and agent code is about 2.2× more verbose than human repositories.

**29. Rethinking Code Complexity Through the Lens of Large Language Models** (2026). Fetched.
- **Where:** arXiv 2602.07882. Opened https://arxiv.org/html/2602.07882.
- **Finding:** cyclomatic complexity, cognitive complexity, Halstead and the maintainability index mostly lose their correlation with LLM task difficulty once code length is controlled for.

**30. Does AI-Assisted Coding Deliver? A Difference-in-Differences Study of Cursor's Impact** (He et al.). Snippet-only.
- **Where:** arXiv 2511.04427.
- **Finding:** static-analysis warnings rose about 30% and code complexity about 41% after adoption (v2 figures; versions differ).

**31. A Comparative Study of Code Generation using ChatGPT 3.5 across 10 Programming Languages** (Buscemi, 2023). Fetched (abstract only).
- **Where:** arXiv 2308.04477.
- **Scope:** 10 languages, 40 tasks, measures executability, runtime and length. I could not confirm which 10 languages.

### D. Measurement methodology

- **Kalibera & Jones, "Rigorous Benchmarking in Reasonable Time"** (ISMM 2013). Fetched the Kent repository page (https://kar.kent.ac.uk/33611/), which carries a corrected version. Advises repeating at each level (build, execution, iteration) and reporting effect sizes with confidence intervals.
- **Mytkowicz et al., "Producing Wrong Data Without Doing Anything Obviously Wrong!"** (ASPLOS 2009, DOI 10.1145/1508284.1508275). Fetched the USI page. It confirms that measurement bias is "significant and commonplace" and recommends setup randomization. The usual examples (UNIX environment size, link order) were not on the page I opened.
- **Georges, Buytaert, Eeckhout** (OOPSLA 2007), snippet-only. **Barrett et al., "Virtual Machine Warmup Blows Hot and Cold"** (OOPSLA 2017), snippet-only: only about 30–56% of VM/benchmark pairs warm up in the traditional way. **Curtsinger & Berger, STABILIZER** (ASPLOS 2013), snippet-only. **Laaber et al.** (EMSE 2019, cloud microbenchmarking), snippet-only: coefficient of variation 0.03% to over 100%; randomized interleaving detects slowdowns of 10% or less. **Hoefler & Belli** (SC15), snippet-only. **Fleming & Wallace** (CACM 1986), snippet-only: use the geometric mean for normalized results.
- **Complexity-metric validity:** Landman et al. (ICSME 2014), snippet-only: cyclomatic complexity vs SLOC correlation is only moderate per method and stronger when aggregated. Muñoz Barón et al. (ESEM 2020), snippet-only: validates cognitive complexity against comprehension time.
- **hyperfine** (fetched https://github.com/sharkdp/hyperfine and its CITATION.cff). Author David Peter; I found no DOI. Features: `--warmup`, `--prepare`, `-N` (no shell), at least 10 runs (about 3 s by default), outlier warnings, JSON export of every run, hardware counters.
- **Academic uses of hyperfine I found:** PIE (#8) and the assembly RL paper (#15).
- **lizard** (fetched https://github.com/terryyin/lizard). Computes cyclomatic complexity, NLOC, tokens and parameters for C/C++, C#, Go, Java, JavaScript, TypeScript, Python, Rust and Zig.

## Coverage map

Y = yes, P = partial, – = no. "Many langs" means 5 or more.

| Work | Many langs | Zig | Asm | Agentic | Effort sweep | Runtime eff. | Complexity | Cost/tokens | Repeats + variance |
|---|---|---|---|---|---|---|---|---|---|
| EffiBench-X | Y (6) | – | – | P (refine appendix) | – (reasoning vs not) | Y | – | – | P (3 runs, 2 models) |
| EffiBench / Mercury / EvalPerf / ENAMEL / COFFE | – (Python) | – | – | – | – | Y | – | – | P |
| ECCO | – | – | – | P (multi-turn) | – | Y | – | – | P |
| PIE | – (C++) | – | – | – | – | Y | – | – | Y (variance study) |
| SWE-Perf / SWE-fficiency | – | – | – | Y | – | Y | – | P | Y (timing) |
| GSO | P (mixed in repos) | – | – | Y | – | Y | – | – | P |
| trace | – (3) | – | – | – | P (thinking on/off) | Y | – | – | Y (5 runs, CV) |
| EvoCodeBench | Y (5) | – | – | P | – | Y (LeetCode %) | – | – | – |
| Assembly RL | – | – | Y | – | – | Y | – | – | P |
| Coignion et al. | – | – | – | – | – | Y | – | – | Y |
| Pereira 2017/2021 | Y (27) | – | – | n/a | n/a | Y | – | n/a | Y |
| Solovyeva et al. | – (3) | – | – | – | – | Y (+energy) | – | – | Y |
| Sonar reasoning blog | – (Java) | – | – | – | Y | – | Y | Y | – |
| Kharma et al. | P (4) | – | – | – | – | – | Y | – | – |
| Nguyen & Nadi | P (4) | – | – | – | – | – | Y | – | – |
| SlopCodeBench | – (Python run) | – | – | Y | – (fixed high) | – | Y | Y | P |

## Gaps

1. **No study combines a reasoning-effort sweep with the runtime of the generated code**, in any language. Effort-sweep evidence is limited to Sonar's Java study (complexity, issues, cost) and accuracy/token studies. trace only compares thinking on vs off and finds no reliable efficiency gain.
2. **No efficiency benchmark covers more than 6 languages**, and none includes Rust together with C, TypeScript or Zig. I found no Zig efficiency or quality study at all; Zig appears only in blog anecdotes.
3. **No study measures runtime and complexity on the same LLM outputs across languages.** Kharma et al. has complexity in 4 languages but no runtime. EffiBench-X has runtime but no complexity.
4. **Cost, tokens and wall-clock time are almost never reported alongside efficiency.** Sonar and SlopCodeBench report cost, but not runtime.
5. **Repeated generations per configuration with variance reporting are rare.** Most benchmarks use temperature 0 or a single sample per configuration; Mercury (K = 5) and EvalPerf (50 samples) are exceptions.
6. **Frontier-2026 models are missing from efficiency benchmarks.** The newest I saw are GPT-5.2, Claude 4.5 and GLM-4.6 (SWE-fficiency, EvoCodeBench).

## Methodology lessons

1. **Normalize against a reference in the same language, and also report absolute numbers.** EffiBench-X's clip(T_ref/T_llm, 0, 1) is simple but has three problems:
   - It hides results that beat the reference.
   - Scoring failures as 0 mixes correctness into the efficiency number.
   - It assumes the references are equally good in every language, and the paper gives no procedure to ensure that.

   Suggested approach: report efficiency only on passing runs, with pass rate separately. Use log-ratios against the same-language reference, summarized by geometric mean (Fleming & Wallace). Add a second ratio against one global reference (e.g., the C or Rust solution) so that "LLM vs expert" stays separate from "language vs language". For pairwise model comparisons, use only tasks both models passed, as EvalPerf does.
2. **Make the reference solutions comparable across languages.** Use the same algorithm and the same threading, and no third-party libraries unless they are equivalent across languages. van Kempen et al. showed that the TypeScript vs JavaScript and C vs C++ gaps in Pereira's data came from implementation differences, not from the languages.
3. **Fix and report build flags; never benchmark with sanitizers on.** EffiBench-X built C++ with `-fsanitize=address`, and Solovyeva built with `-O0`. Pin compiler and runtime versions and use release builds (Rust release, Zig ReleaseFast, `-O2`/`-O3`).
4. **Plan for process startup and JIT warm-up.** hyperfine starts a new process for every run. Its `--warmup` warms OS caches but not a language runtime's JIT. Either make inputs large enough that runtime is far longer than startup, or measure a trivial "hello" program per language as a startup baseline. Barrett et al. show steady state is not guaranteed even in-process. State which you measure: end-to-end time or steady-state time.
5. **Size inputs so they actually stress the algorithm.** Prior work scales inputs until a time limit (EvalPerf, up to 20 s), uses hardness levels (ENAMEL), or generates stress tests (COFFE). Tiny inputs make all solutions look the same.
6. **Treat noise as a first-class problem.**
   - Pin cores (taskset or cpuset), fix CPU frequency or disable turbo, use an otherwise idle machine.
   - Randomize or interleave the order of runs (Mytkowicz; Laaber).
   - Take at least 10 timed runs after warm-up and report median with a confidence interval (Kalibera & Jones).
   - Only call a difference real if it passes a significance test (SWE-Perf uses Mann–Whitney) or exceeds a threshold. PIE's identical programs differed by up to 1.91×, which is why it requires at least 10% for %Opt.
   - Optionally record `perf stat` instruction counts as a low-noise secondary metric (EvalPerf, COFFE). They miss memory and cache effects and include JIT work, so do not use them as the only metric.
7. **Keep two kinds of variance separate.** Timing noise comes from re-running the same binary. Generation variance comes from different outputs of the same model and prompt; Coignion found temperature changes the spread of runtimes. Use bootstrap or mixed-effects models with the task as a random effect.
8. **Use one complexity tool across all languages.** lizard covers all eight languages of the study. Cyclomatic complexity tracks code size (Landman; Sonar; 2602.07882): Sonar found that higher effort mainly produces more code while complexity per line stays flat.
9. **Avoid contamination.** Prior benchmarks use tasks released after the models' training cutoffs; the hand-written scenarios also help here.

## Open questions

- **Not verified:** Pereira's normalized numbers (the table is an image); the 10 languages in Buscemi's study; Nguyen & Nadi's per-language medians; the environment-size and link-order examples in Mytkowicz (not on the page I opened).
- **Venues seen only in search results or listings:** Mercury (NeurIPS 2024), EvalPerf (COLM 2024), ENAMEL (ICLR 2025), ECCO (EMNLP 2024), GSO and the assembly RL paper (NeurIPS 2025), SWE-Perf (ICML 2026).
- **EffiBench-X v2** (27 Oct 2026): I did not check what changed from v1 beyond confirming the same 6 languages and no new frontier models.
- **Inconsistent numbers inside papers:** SWE-fficiency (0.15× vs 0.23×; a GPT-5 Mini figure that differs between text and table); SWE-Perf (1 core vs 5 cores); Sonar (543 vs over 4,400 tasks).
- **Other gaps:** whether SonarQube or any cognitive-complexity tool supports Zig; whether hyperfine has a DOI (none in its CITATION.cff). Very recent or unindexed effort-sweep work could exist; extended searches found none that measure runtime.