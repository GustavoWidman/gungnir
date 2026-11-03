# Facts: models, pricing, tooling

Collected 2026-11-03. "fetched" entries were opened on the primary page; "snippet-only" entries were not. Check every citation against the primary source before it goes into the paper.

## LLM benchmark research for gungnir (checked 2026-11-03)

All ten models exist and can be called through an API today. Two names are easy to get wrong: the current Fable is **Claude Fable 5.1**, and "GPT 6 Luna" is the real name (there is no 6.1 Luna). The biggest constraint for the study design is that **no provider offers deterministic seeds for these reasoning models**. That means repeated samples per configuration, which multiplies the budget. Assembly is the gap on the tooling side: no complexity tool measures it properly.

### Models table
Prices are USD per million tokens (MTok): input / output / cached input. "Verified" means it was read on the vendor's own documentation page.

| Model | API id | Provider | Reasoning levels (default) | Price in / out / cached | Context / max output | Reasoning tokens in usage | Status |
|---|---|---|---|---|---|---|---|
| GPT-6 Astra | `gpt-6-astra` | OpenAI | low, medium, high, xhigh, max. No `none`, no `minimal`. Default not stated on the official page | $10 / $50 / $1 (cache write $12.50). Batch/Flex is half price; Fast mode is double | 1,050,000 (max input 922k) / 128k | Yes, `output_tokens_details.reasoning_tokens`. Raw reasoning is never returned; summaries are opt-in | verified, except the default |
| GPT-6.1 Sol | `gpt-6.1-sol` | OpenAI | low, medium (default), high, xhigh, max. No `none` or `minimal` | $2 / $10 / $0.10 (write $2.50) | 1.05M / 128k | Yes (same field) | verified |
| GPT-6.1 Terra | `gpt-6.1-terra` | OpenAI | none, low, medium (default), high, xhigh, max | $1 / $5 / $0.05 (write $1.25) | 1.05M / 128k | Yes | verified (released Oct 26, 2026) |
| GPT-6 Luna | `gpt-6-luna` | OpenAI | none, low, medium (default), high, xhigh, max | $0.10 / $0.50 / $0.01. Prompts over 272k cost 2x input, 1.5x output | 1.05M / 128k | Yes | verified |
| GLM-5.3 | `glm-5.3` | Z.ai | `reasoning_effort` low, high, max (default max). Thinking is forced on and cannot be disabled | $1.40 / $4.40 / $0.26 (cache storage free for a limited time) | 1M / 128K | **No.** The usage object only has prompt, completion and cached tokens. `reasoning_content` is streamed | verified |
| GLM-5.3-Flash | `glm-5.3-flash` (a FlashX variant is $0.37 / $1.25) | Z.ai | low, high, max (default max). Thinking forced on | $0.15 / $0.50 / $0.03 | 1M / 128K | No (same schema) | verified |
| Claude Fable 5.1 (current) | `claude-fable-5-1` | Anthropic | low, medium, high (default), xhigh, max. Thinking always on | $10 / $50 / $0.25 (5-min write $12.50, 1-hour write $20) | 1M / 128K | Yes, `usage.output_tokens_details.thinking_tokens`. Anthropic says it is approximate (re-tokenized) | verified (released Sep 1, 2026) |
| Claude Fable 5 (legacy) | `claude-fable-5` | Anthropic | low to max (default high). Thinking always on | $10 / $50 / $1 | 1M / 128K | Yes | verified. Legacy, retirement not before Jun 9, 2027, knowledge cutoff Jan 2026 |
| Claude Opus 5.5 | `claude-opus-5-5` | Anthropic | low, medium (default), high, xhigh, max. Thinking always on | $4 / $20 / $0.20 (writes $5 / $8) | 1M / 128K | Yes | verified |
| Claude Sonnet 5.5 | `claude-sonnet-5-5` | Anthropic | low, medium, high (default), xhigh, max. Thinking can't be fully disabled; `between_tools` skips up-front thinking, only at high or below | $2 / $10 / $0.10 (writes $2.50 / $4) | 1M / 128K | Yes | verified |
| Claude Haiku 5.5 | `claude-haiku-5-5` | Anthropic | low, medium (default), high, xhigh, max. Thinking can be disabled at high or below | $0.10 / $0.50 / $0.01 for prompts up to 100k tokens; $0.50 / $2.50 / $0.05 above | 1M / 128K | Yes | verified |

All Anthropic models get 50% off through the Batch API.

**Seeds and temperature:**
- **OpenAI:** the Responses API has no `seed`. The Chat Completions `seed` is marked "Deprecated", best-effort only. Custom `temperature` and `top_p` are only allowed when effort is `none`, and Astra never accepts them.
- **Anthropic:** there is no seed parameter. On every 5.x model, a non-default `temperature`, `top_p` or `top_k` returns a 400 error. From API version `2026-10-21`, including them at any value returns a 400 on every model.
- **Z.ai:** `temperature` is accepted in the range [0, 1] (default 1.0), plus `top_p` and `do_sample`. No seed is documented.

**OpenRouter:** it carries all ten models, plus Fable 5, under one API. Ids are `openai/gpt-6-astra`, `openai/gpt-6.1-sol`, `openai/gpt-6.1-terra`, `openai/gpt-6-luna`, `z-ai/glm-5.3`, `z-ai/glm-5.3-flash`, and `anthropic/claude-{fable-5.1,opus-5.5,sonnet-5.5,haiku-5.5}`. These come from its public `/api/v1/models` endpoint. OpenAI and Anthropic prices there match the vendors' own. GLM-5.3 is served by about 50 hosts, many running fp4 or fp8 quantized weights, at prices from $0.07 to $2.80 input. Only the Z.AI endpoint charges the official $1.40 / $4.40.

### Tooling table
These versions come from the nixpkgs registry on this machine (revision 1c89083, last modified 2026-10-30). The flake input is pinned, because a different channel will give different versions.

| Tool | Version | Coverage / notes |
|---|---|---|
| lizard | **No top-level `lizard` attribute.** `python3Packages.lizard` is 1.24.1 (PyPI has 1.25.0 from 2026-11-02) | Covers Rust, Go, Python, JS, TS/TSX, C, C++, Java, C# and Zig, but **not assembly**. Measures cyclomatic complexity, lines of code, token count and parameter count. Add-ons cover cognitive complexity (`-Ecognitive`, SonarSource spec), Halstead and nesting depth. Outputs XML, CSV, HTML and checkstyle. It is Python, so you would call it as a subprocess. |
| rust-code-analysis (Mozilla) | 0.0.25. That is also the last crates.io release, from Jan 2023; the repo was still getting commits in 2026 | Covers Rust, Python, JS, TS/TSX, C/C++, Java and Kotlin. **No Go, C#, Zig or assembly.** Has the most metrics: cyclomatic, cognitive, Halstead, several line counts, ABC, maintainability index, and NOM/NARGS/NEXITS/WMC/NPM/NPA. Usable as a Rust library. |
| cccc (moznion) | crates `cccc-core`, `cccc-rs`, `cccc-go`, `cccc-es`, `cccc-py`, `cccc-c`, `cccc-cpp`, `cccc-java`, `cccc-cs`, `cccc-zig`, all 1.8.6 (released 2026-11-03). **Not in nixpkgs.** | Covers every benchmarked language except assembly. Only measures cyclomatic and cognitive complexity (no Halstead). Each language is a separate library crate. **Name collision:** `nixpkgs#cccc` (3.2.0) is an unrelated old "C and C++ Code Counter". |
| arborist-metrics | crate 0.1.3 (May 2026) | Uses tree-sitter. Rust, Python, JS, TS, Java and Go by default; C, C++ and C# behind feature flags. No Zig or assembly. Cognitive, cyclomatic and lines of code. Few users. |
| scc | 4.1.0 | Gives a rough per-file complexity count for all listed languages, including Zig and "Assembly". For assembly it just matches C-style words such as `if `, `for ` and `==`, so the number means nothing there. |
| tokei | 15.0.0 | Line counts only, all languages including assembly. |
| Tree-sitter grammars | | **Zig:** crate `tree-sitter-zig` 1.1.2 (last published Dec 2024; repo active Sep 2026); nixpkgs grammar 1.1.2-unstable-2025-09-10. **Generic assembly:** crate `tree-sitter-asm` 0.24.0 (repo active Nov 2025). **NASM:** naclsn grammar, not on crates.io, in nixpkgs, last commit Nov 2024. **GAS:** sirius94 grammar, not on crates.io, 7 stars. The crate `tree-sitter-language-pack` 1.23.1 bundles ASM, NASM, Zig and C# in one dependency. Core `tree-sitter` crate is 0.27.1; the nixpkgs CLI is 0.26.11. |
| Toolchains | | hyperfine 2.0.0; zig 0.17.0 (0.16.0 also packaged); bun 1.4.2; uv 0.13.5; python3 3.14.8; rustc/cargo 1.99.0; go 1.26.10 by default (`go_1_27` is 1.27.3); clang/llvm 21.1.8 by default (22.1.8 and 23.1.3 also packaged); gcc 16.2.0; jdk 21.0.12 by default (jdk25 is 25.0.4); dotnet-sdk 8.0.427 by default (`_9` is 9.0.320, `_10` is 10.0.403); nasm 3.02; poop 0.5.0-unstable-2026-05-04; perf 7.3; valgrind 3.27.1 |
| Sandboxing | | All in nixpkgs: bubblewrap 0.12.0, nsjail 3.6, podman 6.1.4, docker 29.9.2, firejail 0.9.80 (needs setuid, so on NixOS use the `programs.firejail` module). Also gvisor 20260406.0, crun 1.30.1, firecracker 1.16.1. Unprivileged user namespaces are enabled on this machine, so `bwrap --unshare-net` should work without root. |

### Surprises
1. **Model outputs are not reproducible.** No seeds, and temperature is mostly rejected. Plan for k repeats per configuration, which multiplies cost by k.
2. **Reasoning levels don't match across vendors.** The ten models have **48 effort settings in total**, or 54 if you add the Sonnet and Haiku thinking-off modes. Defaults differ too: medium for Opus 5.5, Haiku 5.5 and the OpenAI models; high for Fable and Sonnet; max for GLM. GPT-6 has no `minimal`, Astra and Sol have no `none`, and GLM has only 3 levels.
3. **GLM doesn't report reasoning tokens separately**, and OpenRouter will spread GLM requests across quantized hosts unless you pin it to the Z.AI provider.
4. **Assembly has no real complexity tool.** A custom tool is needed, for example counting conditional jumps through `tree-sitter-asm` or the NASM grammar.
5. **Only cccc covers all the other languages as a Rust library.** rust-code-analysis is missing Go, C# and Zig, and lizard needs a Python subprocess.
6. **`perf_event_paranoid` is 3 on this machine.** `poop` and unprivileged `perf stat` won't run until it is lowered to 2 or less (or the process gets `CAP_PERFMON`). hyperfine is unaffected.
7. **Forced tool use is rejected by Opus 5.5, Sonnet 5.5 and Fable 5.1** (`tool_choice` "any"/"tool" returns a 400). Use structured outputs instead.
8. **Claude can refuse with `stop_reason: "refusal"`**, and the "cyber" category can trigger on harmless work. The harness should record refusals as their own outcome.
9. **Output tokens drive the cost.** As a rough example, a call with 2k input and 10k output (reasoning included) costs about $0.52 on Astra or Fable 5.1, $0.21 on Opus 5.5, $0.10 on Sol or Sonnet 5.5, $0.05 on Terra or GLM-5.3, and about $0.005 on Luna, Haiku 5.5 or GLM-Flash.

### Open questions
- GPT-6 Astra's default effort isn't stated on the official model page. Some third-party pages say "medium" and one says "low". It is also unclear whether ordinary or student API keys have Astra yet: it launched Sep 3 as a staged rollout through OpenAI's Trusted Access Program. OpenRouter does list OpenAI as a provider for it.
- It is unconfirmed how OpenRouter passes `max` and `xhigh` through to each vendor. Its parameter docs leave `max` out of `reasoning_effort` and map Claude's effort setting via `verbosity`, while its reasoning guide lists `max`. A small test call per model would settle it.
- Whether OpenRouter's usage numbers include GLM reasoning tokens.
- Whether OpenRouter's `seed` actually works on GLM; it depends on the host.
- GLM-5.3-Prime ($2.80 / $8.80) appears on OpenRouter but not on Z.ai's pricing page.
- Whether the Zig grammar handles Zig 0.17 syntax.
- Rate limits for each account tier.

GLM-5.3 and GLM-5.3-Flash are open-weight on Hugging Face (`zai-org`; GLM-5.3 under a custom licence, Flash under MIT).

### Sources
- OpenAI: [model pages](https://developers.openai.com/api/docs/models/gpt-6-astra) (also `/gpt-6.1-sol`, `/gpt-6.1-terra`, `/gpt-6-luna`), [pricing](https://developers.openai.com/api/docs/pricing), [reasoning guide](https://developers.openai.com/api/docs/guides/reasoning), [latest-model guide](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra), [changelog](https://developers.openai.com/api/docs/changelog), [Responses API reference](https://developers.openai.com/api/reference/resources/responses/methods/create)
- Anthropic: [models overview](https://platform.claude.com/docs/en/about-claude/models/overview), [pricing](https://platform.claude.com/docs/en/about-claude/pricing), [effort](https://platform.claude.com/docs/en/build-with-claude/effort), [thinking](https://platform.claude.com/docs/en/build-with-claude/thinking), [Messages API](https://platform.claude.com/docs/en/api/messages/create), [Fable 5.1](https://platform.claude.com/docs/en/models/fable-5-1/overview), [Fable 5](https://platform.claude.com/docs/en/models/fable-5/overview), [Opus 5.5 migration guide](https://platform.claude.com/docs/en/models/opus-5-5/migration-guide)
- Z.ai: [pricing](https://docs.z.ai/guides/overview/pricing), [GLM-5.3](https://docs.z.ai/guides/llm/glm-5.3), [GLM-5.3-Flash](https://docs.z.ai/guides/llm/glm-5.3-flash), [chat API reference](https://docs.z.ai/api-reference/llm/chat-completion), [thinking mode](https://docs.z.ai/guides/capabilities/thinking-mode), [release notes](https://docs.z.ai/release-notes/new-released)
- OpenRouter: [models API](https://openrouter.ai/api/v1/models), [parameters](https://openrouter.ai/docs/api/reference/parameters), [reasoning tokens](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)
- Tools: [lizard](https://github.com/terryyin/lizard), [rust-code-analysis](https://github.com/mozilla/rust-code-analysis), [cccc](https://github.com/moznion/cccc), [arborist-metrics](https://docs.rs/arborist-metrics), [scc languages.json](https://github.com/boyter/scc), [tree-sitter-zig](https://github.com/tree-sitter-grammars/tree-sitter-zig), [tree-sitter-asm](https://github.com/RubixDev/tree-sitter-asm), [tree-sitter-nasm](https://github.com/naclsn/tree-sitter-nasm), [tree-sitter-gas](https://github.com/sirius94/tree-sitter-gas), [tree-sitter-language-pack](https://github.com/xberg-io/tree-sitter-language-pack)
- Background coverage used only as leads, not as facts: [aiweekly Terra](https://aiweekly.co/alerts/openai-ships-gpt-61-terra-at-15-per-million-half-of-sol), [emergent GLM pricing](https://emergent.sh/learn/glm-5-3-pricing), [vorplabs Astra](https://vorplabs.com/models/releases/gpt-6-astra), [llm-stats Astra](https://llm-stats.com/blog/research/gpt-6-astra-launch)