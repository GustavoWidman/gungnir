# Related work: reasoning effort, cost, harness effects, statistics

Collected 2026-11-03. "fetched" entries were opened on the primary page; "snippet-only" entries were not. Check every citation against the primary source before it goes into the paper.

## Research report: reasoning effort, cost, harness effects and statistics for a multi-language LLM coding study

**Short answer on the key question:** I found no study that runs several reasoning-effort levels across several programming languages and several frontier vendors, and then tests the effort × language interaction. Five works come close (listed in section B and under Gaps). The closest is Multi-LCB (ICLR 2026). It compares only GPT-OSS at low vs medium effort across 12 languages. In its data the gain from more effort is not larger in the less common languages, and the authors do not analyze this.

"Fetched" means I opened the page or PDF. "Snippet-only" means I saw only search-result text. Venues are given only where a page showed them.

---

### Sources found

**A. Reasoning effort, test-time compute and overthinking on code**

1. **Sonar, "How reasoning impacts LLM coding models"** (blog, 3 Sep 2025). https://www.sonarsource.com/blog/how-reasoning-impacts-llm-coding-models/ (fetched)
   - Tested GPT-5 at minimal, low, medium and high effort on about 4,400 Java tasks, analyzed with SonarQube.
   - Pass rate: 75.4% / 80.5% / 82.0% / 81.8% (minimal / low / medium / high). Medium is the best value.
   - Cost per benchmark run: $22 / $47 / $64 / $189.
   - Total cyclomatic complexity rose from 145k to 204k and lines of code from 490k to 727k as effort went from minimal to high. Complexity per line stayed nearly flat (about 0.23).
   - This matters directly for this study's complexity metric: total complexity mostly follows code volume. It is a vendor blog, not peer-reviewed, and covers Java only.

2. **Aider polyglot leaderboard** (225 Exercism exercises in C++, Go, Java, JS, Python and Rust). https://aider.chat/docs/leaderboards/ (fetched)
   - GPT-5: low 81.3% ($10.37), medium 86.7% ($17.69), high 88.0% ($29.08).
   - o3-mini: medium 53.8%, high 60.4%.
   - Claude Sonnet 4: 56.4% without thinking, 61.3% with 32k thinking. Gemini 2.5 Flash: 44.0% without thinking, 55.1% with 24k.
   - No per-language breakdown is published. The raw logs might allow one.

3. **Kapoor et al., "Holistic Agent Leaderboard (HAL)"**, arXiv 2510.11977 (Oct 2025). https://arxiv.org/html/2510.11977 (fetched). ICLR 2026 poster per iclr.cc (snippet-only).
   - Across 36 model-scaffold-benchmark runs, higher reasoning effort gave equal or lower accuracy in 21 (about 58%).
   - Caveats: "high" for Anthropic models meant LiteLLM's default 4,096-token budget, and no significance tests were reported.
   - Costs used a price snapshot dated 24 Sep 2025. Prompt caching was not accounted for. The authors warn that providers sometimes swap weights or quantizations.

4. **Cuadron et al., "The Danger of Overthinking"**, arXiv 2502.08235 (Feb 2025). https://arxiv.org/abs/2502.08235 (fetched abstract)
   - 4,018 SWE-bench Verified trajectories. Higher "overthinking" scores correlate with lower resolve rates, and reasoning models overthink more.
   - Picking the candidate with the lowest overthinking score improved performance by almost 30% and cut compute cost by 43%.

5. **Gema et al., "Inverse Scaling in Test-Time Compute"**, arXiv 2507.14417 (TMLR, 12/2025). https://arxiv.org/abs/2507.14417v2 (fetched)
   - Builds tasks where longer reasoning lowers accuracy and recommends evaluating models across a range of reasoning lengths.
   - None of the tasks are coding tasks.

6. **Chen et al., "Do NOT Think That Much for 2+3=?"**, arXiv 2412.21187. https://arxiv.org/abs/2412.21187 (fetched)
   - Defines "overthinking" and proposes outcome- and process-based token-efficiency metrics.
   - Uses math and science benchmarks, not code.

7. **Wong et al., "Confucius Code Agent"**, arXiv 2512.10398. https://arxiv.org/html/2512.10398 (fetched)
   - Claude 4 Sonnet on a SWE-bench Verified subset with thinking budgets of 8k / 16k / 32k scored 67.3 / 68.4 / 68.7, so returns diminish beyond 16k.
   - The authors note the budget does not precisely control reasoning length.
   - Also gives same-model scaffold results: Claude 4 Sonnet on SWE-bench Verified scored 66.6 with SWE-Agent, 72.8 with OpenHands and 74.6 with their own agent.

8. **Hu & Wang, "Effort as Ceiling, Not Dial"**, arXiv 2605.16938 (May 2026). https://arxiv.org/abs/2605.16938 (fetched)
   - For GPT-OSS models, the effort parameter "sets an upper budget on generation rather than driving real-time allocation."
   - The tasks are not coding tasks.

9. **Iacobacci et al., "Increasing the Thinking Budget is Not All You Need"**, arXiv 2512.19585. https://arxiv.org/abs/2512.19585 (fetched abstract)
   - Self-consistency or self-reflection used compute better than a larger thinking budget.
   - The abstract does not say whether coding tasks were included.

10. **Ma et al., "Reasoning Models Can Be Effective Without Thinking"**, arXiv 2504.09858. https://arxiv.org/abs/2504.09858 (fetched)
    - At matched token budgets, skipping the thinking phase beat thinking on seven datasets that included coding. The gap was largest at low budgets.

11. **Du et al., "OckBench"**, arXiv 2511.05722. https://arxiv.org/abs/2511.05722 (fetched)
    - Covers reasoning and coding tasks. Models with similar accuracy differed in token length by up to 5.0×.

12. **Xue, Uddin & Wang, "An Empirical Study of Reasoning Steps in Thinking Code LLMs"**, arXiv 2511.05874. https://arxiv.org/abs/2511.05874 (abstract fetched; effort-level claims snippet-only)
    - According to the search snippet, o3-mini at low, medium and high effort on 100 BigCodeBench tasks showed little difference in correctness. I could not confirm this on the abstract page.

13. **Anthropic, "Introducing Claude Opus 4.5"** (24 Nov 2025). https://www.anthropic.com/news/claude-opus-4-5 (fetched)
    - Vendor claim: at medium effort, Opus 4.5 matches Sonnet 4.5's best SWE-bench Verified score with 76% fewer output tokens.

**B. Effort (or reasoning mode) × language: the closest works**

14. **Ivanova et al., "Multi-LCB"**, arXiv 2606.20517 (ICLR 2026 per the arXiv comments field). https://arxiv.org/html/2606.20517 (fetched)
    - LiveCodeBench problems converted into 12 languages. Table 1 reports Pass@1 averaged over 10 runs at t=0.2.
    - GPT-OSS-120B averaged 53.1 at low effort and 67.8 at medium. GPT-OSS-20B averaged 43.3 at low and 59.8 at medium. I verified these averages.
    - Per-language gains, read from the table and not checked by hand:
      - 120B: about 11.6 (C#) to 18.1 (Go).
      - 20B: about 11.2 (C#) to 19.9 (Rust).
    - The smallest gains were in C# and Scala, the weakest languages, so this data does not suggest that more effort helps less common languages more.
    - The authors do not analyze the interaction.

15. **AutoCodeBench**, arXiv 2508.09101. https://arxiv.org/html/2508.09101v1 (fetched)
    - Covers 20 languages, with some models run in both "reasoning mode" and "non-reasoning mode." The paper does not say how the mode was switched.
    - Claude Opus 4 reasoning minus non-reasoning, per language, is mixed: Racket +4.1, Ruby +7.0, R +5.5, Elixir −2.0, Kotlin −3.0, Dart −3.0, C++ −1.6. My mapping of values to columns may be off.
    - All results are single-run at greedy or default decoding, and the interaction is not analyzed.

16. **Jin et al., "Understanding Chain-of-Thought Effectiveness in Code Generation"**, arXiv 2512.09679. https://arxiv.org/html/2512.09679v1 (fetched)
    - Prompting styles (including chain-of-thought) across 12 languages; this is prompting, not effort levels.
    - Structured prompting helped statically typed languages more (for example Self-Planning: +7.03% static vs +4.54% dynamic).
    - A condition using DeepSeek-R1 reasoning used 2,000–7,000 tokens per problem, versus 200–700 for structured prompting, for only +1.26% accuracy.

17. **Wu, Anderson & Guha, "The Best Programming Language for Tokenmaxxing"**, arXiv 2607.22807 (Jul 2026; LMPL 2026 per researchr, snippet-only). https://arxiv.org/html/2607.22807 (fetched)
    - Setup: mini-swe-agent, 5 models, Python / Java / Rust / OCaml, 100 LiveCodeBench tasks, 1 run per cell, effort fixed per model.
    - Statistics: a linear mixed-effects model on log tokens with a random intercept for each problem. Problem identity explains 73–97% of the token variance.
    - OCaml cost the most tokens for every model (1.28–1.69× Python for open models). GPT-5.5 used 1.30–1.36× and Sonnet 4.6 1.16–1.28× in all non-Python languages.
    - Agents often wrote code that did not compile in unfamiliar languages.
    - Accuracy was mostly similar across languages. This is the best method template for this study, but effort is not varied.

18. **Sapenov & Savelka, "mmPISA-bench"**, arXiv 2606.07069. https://arxiv.org/html/2606.07069v1 (fetched)
    - Crosses effort with *natural* language: GPT-5.1 at none / low / medium / high and Opus 4.5 at low / medium / high, across 43 languages.
    - GPT-5.1 accuracy went 78.9 → 93.7 → 94.7 → 95.7 while output tokens went 59k → 582k → 1,434k → 3,709k.
    - The largest effort gains were in languages with the lowest no-reasoning baseline, though the authors do not test this.
    - A useful methodological analogue only.

19. **CodeChemist**, arXiv 2510.00501. https://arxiv.org/html/2510.00501v3 (fetched)
    - Has only a thinking-mode run (DeepSeek-V3 on C++, Java and Lua), with no matching thinking-off run, so it gives no usable effort × language evidence.

**C. Cost-aware evaluation**

20. **Erol et al., "Cost-of-Pass"**, arXiv 2504.13359 (v2 Feb 2026; ICLR 2026 poster per iclr.cc, snippet-only). https://arxiv.org/html/2504.13359v2 (fetched)
    - Defines cost-of-pass as expected cost per attempt divided by success probability, which is infinite when the success rate is 0. Per-attempt cost = input tokens × input price + output tokens × output price.
    - Majority voting gave no economic gain on any dataset.
    - No code tasks were included.

21. **Kapoor et al., "AI Agents That Matter"**, arXiv 2407.01502 (TMLR 2025 per mlanthology, snippet-only). https://arxiv.org/html/2407.01502v1 (fetched)
    - Recommends accuracy–cost Pareto curves and 5 runs per agent with t-distribution 95% confidence intervals.
    - On HumanEval, simple baselines matched complex agents: 93.2% for about $2.45 versus LATS at 88.0% for about $134.50.
    - Recommends reporting input and output token counts alongside dollar costs, because prices change.

**D. Harness and scaffold effects**

22. **Merrill et al., "Terminal-Bench"**, arXiv 2601.11868. https://arxiv.org/html/2601.11868v1 (fetched)
    - At least 5 trials per model-agent pair, with 95% confidence intervals.
    - The same model scores very differently by harness:

      | Model | Agent | Resolution rate |
      |---|---|---|
      | Gemini 2.5 Pro | OpenHands | 15.7% |
      | Gemini 2.5 Pro | Terminus 2 | 32.6% |
      | GPT-5 | Mini-SWE-Agent | 33.9% |
      | GPT-5 | Codex CLI | 49.6% |
      | Opus 4.5 | Claude Code | 52.1% |
      | Opus 4.5 | Terminus 2 | 57.8% |
      | GPT-5.2 | Terminus 2 | 54.0% |
      | GPT-5.2 | Codex CLI | 62.9% |

    - Vendor harnesses are not reliably best: Codex CLI helped GPT-5.2, but Claude Code was worse than Terminus 2 for Opus 4.5.
    - The authors conclude that model choice usually matters more than the agent. They built Terminus 2, which uses bash only, as "a neutral testbed."

23. **Brand & Denain (Epoch AI), "Why benchmarking is hard"** (23 Dec 2025). https://epoch.ai/gradient-updates/why-benchmarking-is-hard (fetched)
    - Changing the scaffold alone moved SWE-bench Verified by up to 11 points for GPT-5 and up to 15 points for Kimi K2 Thinking.
    - The API provider was the biggest source of variance for open models.
    - "A standardized scaffold (like mini-SWE-agent) is usually enough" for comparing models.
    - Default temperatures differ between eval libraries.
    - Epoch's own SWE-bench page notes that a February 2026 scaffold upgrade "led to model performance improving significantly": https://epoch.ai/benchmarks/swe-bench-verified (fetched)

24. **Liu et al., "Coding Agents Have Converged"**, arXiv 2609.17394. https://arxiv.org/abs/2609.17394 (fetched)
    - Scaffold ranges for a single model reach 29.8 points, versus an 8.8-point spread across the top 30 SWE-bench entries.
    - Model × scaffold interaction effects remain significant after correction for multiple comparisons.
    - A snippet also gave an additive-model estimate (scaffold range 37.6 points vs model range 47.9), which I did not confirm.

25. **Hardy et al., "Agent Evaluation Reliability"**, arXiv 2610.00651 (Sep 2026). https://arxiv.org/abs/2610.00651 (fetched)
    - Rankings of fixed model+scaffold systems are reliable (0.935–0.994); rankings of the underlying models are much less so (0.148–0.841).
    - Even infinitely many more tasks would add at most 0.097 reliability when scaffold coverage is the bottleneck.
    - "Evaluation design should follow the intended claim."

26. **Arjmandi, "Harness or Model?"**, arXiv 2609.11987. https://arxiv.org/abs/2609.11987 (fetched)
    - Vendor harness vs a neutral harness on the same model: −1.25 points for Opus 4.8 and +1.25 for GPT-5.5, both with wide confidence intervals.
    - Effects were opposite on different task types, but that split was chosen after seeing the data.
    - The neutral harness cost 1.2–1.6× more per solved task.

27. **mini-swe-agent.** https://github.com/SWE-agent/mini-swe-agent (fetched)
    - About 100 lines of agent code, bash only, no tool-calling API, each action run by `subprocess.run`.
    - Described as "perfect as a baseline system" and used for the SWE-bench bash-only leaderboard.

28. **Xia et al., "Agentless"**, arXiv 2407.01489. https://arxiv.org/abs/2407.01489 (fetched)
    - A fixed three-step pipeline with no autonomous agent reached 32.00% on SWE-bench Lite at a reported cost of $0.70.

29. **Olausson et al., "Is Self-Repair a Silver Bullet for Code Generation?"** (ICLR 2024; snippet-only). https://arxiv.org/abs/2306.09896
    - At equal sampling budget, self-repair gains are small, uneven, or absent.

30. **Zheng et al., "What Makes LLMs Reason in (Multi-Turn) Code Generation?"** (ICLR 2025; snippet-only). https://arxiv.org/abs/2410.08105
    - Multi-turn execution feedback alone gives modest gains and is sometimes worse than single-turn sampling at equal budget.

**E. Statistics, repeated runs and nondeterminism**

31. **Miller, "Adding Error Bars to Evals"**, arXiv 2411.00640 (arXiv only). https://arxiv.org/html/2411.00640v1 (fetched)
    - Report the standard error with each score.
    - Use clustered standard errors when questions are related, such as translations of the same question. In his example they were more than 3× the naive ones.
    - Repeating each question K times reduces variance with diminishing returns. Do not pool all K×N answers as if independent.
    - Compare models with paired, per-question differences.
    - Power analysis: detecting a 3-point difference needs about 969 questions.
    - Do not change temperature to reduce variance.

32. **Bowyer, Aitchison & Ivanova, "Don't Use the CLT in LLM Evals With Fewer Than a Few Hundred Datapoints"**, arXiv 2503.01747 (ICML 2025 spotlight position paper). https://arxiv.org/html/2503.01747 (fetched)
    - With small benchmarks, CLT-based and bootstrap intervals are too narrow.
    - Recommends Wilson or Beta-Bernoulli Bayesian intervals, a paired Bayesian method for comparing models, and a Bayesian hierarchical model for clustered questions.
    - Library: github.com/sambowyer/bayes_evals.

33. **Bjarnason, Silva & Monperrus, "On Randomness in Agentic Evals"**, arXiv 2602.07150. https://arxiv.org/abs/2602.07150 (fetched)
    - 60,000 SWE-bench Verified trajectories.
    - Single-run pass@1 shifts by 2.2–6.0 points depending on which run you pick. Standard deviation exceeds 1.5 points even at temperature 0.
    - Trajectories diverge early.
    - Recommends several runs per task, power analysis, and reporting both pass@k and pass^k.

34. **Ariño de la Rubia & Pafka, "Identical Runs, Different Results"**, arXiv 2609.33812. https://arxiv.org/html/2609.33812 (fetched)
    - Repeated runs of one agent-model pair varied more than the different pairs differed from each other.
    - Comparing on 3 runs put the weaker pair ahead in 28–44% of draws.
    - Resolving the observed differences would need 39–113 runs per arm.
    - It quotes Bjarnason et al. as saying about 9 runs detect a 2-point gain and 36 runs a 1-point gain. I did not see those numbers in Bjarnason's abstract.

35. **Hochlehnert et al., "A Sober Look at Progress in LM Reasoning"**, arXiv 2504.07086. https://arxiv.org/html/2504.07086v2 (fetched)
    - Hardware cluster alone changed scores by up to 8%; temperature by up to about 15%; too low a max-token limit truncates answers.
    - Seed advice: the fetched v2 text says "at least 30 random seeds" for small benchmarks, while snippets said 10. The authors themselves used 10 for AIME/AMC and 3 elsewhere.
    - Report mean ± standard deviation.

36. **Alvarado Gonzalez et al., "Do Repetitions Matter?"**, arXiv 2509.24086. https://arxiv.org/html/2509.24086v1 (fetched)
    - A single run produced at least one ranking inversion in 10 of 12 slices.
    - Two runs removed about 83% of those inversions. Recommends at least 2 repetitions.
    - Model spec: logistic mixed-effects regression (`glmer`) with random intercepts for problem and for problem × run.

37. **Zhang, Cai & Chen, "Beyond the Singular"**, arXiv 2502.08943 (Findings of ACL 2026 per the comments field). https://arxiv.org/abs/2502.08943 (fetched)
    - Multiple generations per prompt reduce variance. Proposes a hierarchical model of benchmark and sampling randomness.

38. **Atil et al., "Non-Determinism of 'Deterministic' LLM Settings"**, arXiv 2408.04667. https://arxiv.org/abs/2408.04667 (fetched)
    - Five models run 10 times with "deterministic" settings: accuracy varied by up to 15%, and the best-to-worst gap reached 70%.

39. **He / Thinking Machines, "Defeating Nondeterminism in LLM Inference"** (10 Sep 2025). https://thinkingmachines.ai/blog/defeating-nondeterminism-in-llm-inference/ (fetched)
    - The cause is batch-size dependence driven by server load. 1,000 completions at temperature 0 gave 80 unique outputs.
    - Users of commercial APIs cannot fix this from the client side.

40. **Chen et al., "Evaluating LLMs Trained on Code"**, arXiv 2107.03374. https://arxiv.org/abs/2107.03374 (abstract fetched)
    - Introduces HumanEval and the pass@k metric. The unbiased estimator is 1 − C(n−c,k)/C(n,k), stated from secondary sources and memory, not seen on the abstract page.

41. **Yao et al., "τ-bench"**, arXiv 2406.12045 (snippet-only).
    - Defines pass^k, the probability that all k trials succeed, with estimator C(c,k)/C(n,k).

42. **Rabanser et al., "Towards a Science of AI Agent Reliability"**, arXiv 2602.16666 (ICML 2026). https://arxiv.org/abs/2602.16666 (fetched)
    - Proposes 12 reliability metrics, including consistency across runs. Capability gains brought only small reliability gains.

**F. Speed, providers and vendor token accounting**

43. **Artificial Analysis performance methodology.** https://artificialanalysis.ai/methodology/performance-benchmarking (fetched)
    - Defines time to first token, time to first answer token (after thinking), output speed, and end-to-end time.
    - Re-tokenizes all output with tiktoken `o200k_base` so speeds are comparable across models.
    - Tests each provider endpoint 8 times a day and reports the 72-hour median.

44. **Chandrasekar & Kramberger**, arXiv 2605.24217. https://arxiv.org/abs/2605.24217 (fetched)
    - Single-process asyncio benchmark clients add their own queuing under high concurrency, which inflates measured latency.

45. **Gao, Liang & Guestrin, "Model Equality Testing"**, arXiv 2410.20247 (ICLR 2025). https://arxiv.org/abs/2410.20247 (fetched)
    - 11 of 31 API endpoints serving Llama models produced output distributions different from Meta's reference weights.

46. **OpenRouter, "Provider variance / Exacto"** (21 Oct 2025). https://openrouter.ai/blog/announcements/provider-variance-introducing-exacto/ (fetched)
    - Tool-calling accuracy varies by provider much more than standard benchmarks suggest.

47. **Huang et al., "Latency and Token-Aware Test-Time Compute"**, arXiv 2509.09864. https://arxiv.org/abs/2509.09864 (fetched)
    - Treats token cost and wall-clock latency as separate cost terms.

48. **Vendor documentation (all fetched):**
    - **Anthropic effort docs** (https://platform.claude.com/docs/en/build-with-claude/effort):
      - Levels are low / medium / high / xhigh / max.
      - Defaults differ: Opus 5.5 and Haiku 5.5 default to medium, other models to high.
      - "Effort is a behavioral signal, not a strict token budget."
      - Effort also changes how many tool calls the model makes.
      - Sonnet 5.5's levels are "recalibrated" relative to Sonnet 5. Anthropic recommends a fresh effort sweep for each model.
    - **Anthropic extended-thinking docs** (https://platform.claude.com/docs/en/build-with-claude/extended-thinking):
      - `budget_tokens` is "a target rather than a strict cap."
      - Thinking volume is reported in `usage.output_tokens_details.thinking_tokens`.
      - On newer models, earlier turns' thinking is kept and billed as input.
    - **OpenAI reasoning guide** (https://developers.openai.com/api/docs/guides/reasoning):
      - Levels are none / minimal / low / medium / high / xhigh / max, and which ones exist depends on the model (GPT-6.1 Sol has no none or minimal).
      - Reasoning tokens are hidden, billed as output, and reported in `output_tokens_details.reasoning_tokens`.
    - **Z.ai thinking docs** (https://docs.z.ai/guides/capabilities/thinking-mode.md):
      - GLM-5.3 uses forced thinking that cannot be disabled.
      - The page documents no `reasoning_effort` parameter.
      - "Preserved Thinking" is on by default on the Coding Plan endpoint and off on the standard API.

---

### Findings that should shape the study design

**1. Harness: a result describes the harness and the model together.**
- The same model moves 6–17 points between harnesses on Terminal-Bench (#22), and up to 11–15 (Epoch, #23) or 29.8 points (#24) on SWE-bench.
- Which harness is best depends on the model (#22, #3), so a result from a vendor CLI mixes harness effects with model effects.
- Epoch and Terminal-Bench both recommend one standardized, bash-only scaffold for comparing models (#23, #22, #27).
- Report results as model+harness systems (#25).
- Watch for hard-coded test answers in the logs. HAL found agents doing this (#3).

**2. Reasoning effort: treat it as a setting within each model, not one scale shared across vendors.**
- Levels are not comparable across vendors or even across versions of one model (#48, #8). GLM-5.3 cannot turn thinking off.
- Always set the effort explicitly and log it, because defaults differ (Opus 5.5 defaults to medium).
- Set a large max-token limit so answers are not cut off. Thinking counts toward the limit (#48), and truncation lowers scores (#35).
- Expect flat returns at the top end: #1, #2, #7, #3. Some settings may even get worse (#4, #5).

**3. Repetitions and statistics.**
- The number of distinct scenarios limits power more than repetitions do. Miller needs about 1,000 items to detect 3 points (#31), and adding repetitions has diminishing returns.
- Each scenario is implemented in eight languages, which is exactly Miller's translated-questions case. Cluster by scenario.
- Temperature 0 does not make runs reproducible (#38, #39, #33).
- Runtime speed and complexity can only be measured on passing solutions, so different effort levels are judged on different subsets. This is my inference, not something a source states.

**4. Cost.**
- Log, per call: input tokens, cached input, output tokens, and reasoning tokens (#48).
- Convert to dollars using a dated price snapshot, and publish the token counts so readers can reprice (#21, #3).
- In a multi-turn harness, account for prompt caching and thinking re-billed as input (#48). HAL left caching out (#3).
- Token counts are not comparable across vendors' tokenizers. Hidden reasoning tokens can only be taken as the vendor reports them.

**5. Time.**
- Wall-clock time depends on provider load and batching (#39), rate limits during parallel runs (#3), the benchmark client itself (#44), and which provider serves the model (#23, #45, #46).
- Report token counts, as the measure of work that does not depend on the provider, beside the measured end-to-end time.
- In a harness, report model time separately from compile and test time.
- Call each vendor's own API directly, including Z.ai for GLM (#23).

---

### Gaps

- **Effort × programming language:** no study systematically crosses several effort levels with several programming languages and several frontier vendors and tests the interaction. What exists:
  - Multi-LCB (#14): two effort levels, one model family, no interaction analysis, and no sign that less common languages gain more.
  - AutoCodeBench (#15): reasoning on/off only, single run, mixed per-language deltas.
  - Jin et al. (#16): prompted chain-of-thought rather than effort levels.
  - Tokenmaxxing (#17): language effects on tokens, but effort is held fixed.
  - mmPISA (#18): effort × *natural* language, an analogue only.
- This looks like a real gap this paper could fill.
- I found no effort studies at all for Zig, and no Zig benchmark scoring frontier models.
- I found no peer-reviewed method for reporting time independently of the provider.

### Open questions (could not verify)

- The per-language Multi-LCB gains have not been checked against Table 1 of the paper by hand.
- The AutoCodeBench per-language values were matched to columns by position and may be misaligned.
- The "9 runs for 2 points, 36 runs for 1 point" figures come only from #34 quoting #33.
- Hochlehnert's seed advice is unclear: 10 or 30 seeds.
- The effort-level findings in #12 are snippet-only.
- Venues that are snippet-only: Kapoor et al. (TMLR), HAL (ICLR 2026), Cost-of-Pass (ICLR 2026), Tokenmaxxing (LMPL 2026).
- Terminal-Bench gives inconsistent task counts (89 vs 74).
