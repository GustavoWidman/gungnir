# Facts: models, pricing, tooling

Collected 2026-11-03. "fetched" entries were opened on the primary page; "snippet-only" entries were not. Check every citation against the primary source before it goes into the paper.

## LLM benchmark research for gungnir (checked 2026-11-03)

All ten models exist and can be called through an API today. Two names are easy to get wrong: the current Fable is **Claude Fable 5.1**, and "GPT 6 Luna" is the real name (there is no 6.1 Luna). The biggest constraint for the study design is that **no provider offers deterministic seeds for these reasoning models**. That means repeated samples per configuration, which multiplies the budget.

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

### Tooling table
lizard and bubblewrap come from the nixpkgs revision that `flake.lock` pins (0bb2eed, last modified 2026-11-01). The toolchains are those of the study's toolchain set, with the Rust nightly from the fenix input. The flake input is pinned, because a different channel will give different versions.

| Tool | Version | Coverage / notes |
|---|---|---|
| lizard | **No top-level `lizard` attribute.** `python3Packages.lizard` is 1.24.1 (PyPI has 1.25.0 from 2026-11-02) | Covers Rust, Go, Python, JS, TS/TSX, C, C++, Java, C# and Zig, so every language of the study. Measures cyclomatic complexity, lines of code, token count and parameter count. Add-ons cover cognitive complexity (`-Ecognitive`, SonarSource spec), Halstead and nesting depth. Outputs XML, CSV, HTML and checkstyle. It is Python, so gungnir runs it as a subprocess, from `PATH` on the host. |
| Toolchains | | clang 23.1.3 for C and C++; go 1.27.3; bun 1.4.2; typescript 7.0.2 (`typescript-bun`); python 3.15.0 with uv 0.13.5; rustc 1.101.0-nightly (2026-11-02) with gcc 16.2.0; zig 0.17.0. hyperfine 2.0.0 and GNU time 1.10 for measuring, perf 7.3.1 and inferno 0.12.8 for profiling |
| Sandboxing | | bubblewrap 0.12.0, which gungnir runs without root through unprivileged user namespaces |

### Surprises
1. **Model outputs are not reproducible.** No seeds, and temperature is mostly rejected. Plan for k repeats per configuration, which multiplies cost by k.
2. **Reasoning levels don't match across vendors.** The ten models have **48 effort settings in total**, or 54 if you add the Sonnet and Haiku thinking-off modes. Defaults differ too: medium for Opus 5.5, Haiku 5.5, Sol, Terra and Luna (Astra's is not stated); high for Fable and Sonnet; max for GLM. GPT-6 has no `minimal`, Astra and Sol have no `none`, and GLM has only 3 levels.
3. **GLM doesn't report reasoning tokens separately.**
4. **Unprivileged perf needs `perf_event_paranoid` at 2 or less** (or `CAP_PERFMON`). Above that, hyperfine leaves out its instruction count and `gungnir profile` refuses to run.
5. **Forced tool use is rejected by Opus 5.5, Sonnet 5.5 and Fable 5.1** (`tool_choice` "any"/"tool" returns a 400).
6. **Claude can refuse with `stop_reason: "refusal"`**, and the "cyber" category can trigger on harmless work. gungnir records a refusal as its own outcome.
7. **Output tokens drive the cost.** As a rough example, a call with 2k input and 10k output (reasoning included) costs about $0.52 on Astra or Fable 5.1, $0.21 on Opus 5.5, $0.10 on Sol or Sonnet 5.5, $0.05 on Terra or GLM-5.3, and about $0.005 on Luna, Haiku 5.5 or GLM-Flash.

### Open questions
- GPT-6 Astra's default effort isn't stated on the official model page. Some third-party pages say "medium" and one says "low".

GLM-5.3 and GLM-5.3-Flash are open-weight on Hugging Face (`zai-org`; GLM-5.3 under a custom licence, Flash under MIT).

### Sources
- OpenAI: [model pages](https://developers.openai.com/api/docs/models/gpt-6-astra) (also `/gpt-6.1-sol`, `/gpt-6.1-terra`, `/gpt-6-luna`), [pricing](https://developers.openai.com/api/docs/pricing), [reasoning guide](https://developers.openai.com/api/docs/guides/reasoning), [latest-model guide](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra), [changelog](https://developers.openai.com/api/docs/changelog), [Responses API reference](https://developers.openai.com/api/reference/resources/responses/methods/create)
- Anthropic: [models overview](https://platform.claude.com/docs/en/about-claude/models/overview), [pricing](https://platform.claude.com/docs/en/about-claude/pricing), [effort](https://platform.claude.com/docs/en/build-with-claude/effort), [thinking](https://platform.claude.com/docs/en/build-with-claude/thinking), [Messages API](https://platform.claude.com/docs/en/api/messages/create), [Fable 5.1](https://platform.claude.com/docs/en/models/fable-5-1/overview), [Fable 5](https://platform.claude.com/docs/en/models/fable-5/overview), [Opus 5.5 migration guide](https://platform.claude.com/docs/en/models/opus-5-5/migration-guide)
- Z.ai: [pricing](https://docs.z.ai/guides/overview/pricing), [GLM-5.3](https://docs.z.ai/guides/llm/glm-5.3), [GLM-5.3-Flash](https://docs.z.ai/guides/llm/glm-5.3-flash), [chat API reference](https://docs.z.ai/api-reference/llm/chat-completion), [thinking mode](https://docs.z.ai/guides/capabilities/thinking-mode), [release notes](https://docs.z.ai/release-notes/new-released)
- Tools: [lizard](https://github.com/terryyin/lizard)
- Background coverage used only as leads, not as facts: [aiweekly Terra](https://aiweekly.co/alerts/openai-ships-gpt-61-terra-at-15-per-million-half-of-sol), [emergent GLM pricing](https://emergent.sh/learn/glm-5-3-pricing), [vorplabs Astra](https://vorplabs.com/models/releases/gpt-6-astra), [llm-stats Astra](https://llm-stats.com/blog/research/gpt-6-astra-launch)