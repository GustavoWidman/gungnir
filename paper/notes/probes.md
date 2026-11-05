# Endpoint probes

## Z.ai GLM effort passthrough (2026-11-04, one request per cell)

Prompt: "Count the primes below 300 and list the last five. Be careful." `max_tokens` 16000.

The Anthropic-compatible endpoint is `https://api.z.ai/api/anthropic` (`POST /v1/messages`, `x-api-key`). Effort was sent as `output_config.effort`. The OpenAI-compatible coding endpoint is `https://api.z.ai/api/coding/paas/v4` (`POST /chat/completions`), with effort sent as `reasoning_effort`.

| Model | Endpoint | Effort | Output tokens | Reasoning tokens |
|---|---|---|---|---|
| glm-5.3-flash | anthropic | low | 214 | not reported |
| glm-5.3-flash | anthropic | max | 2278 | not reported |
| glm-5.3-flash | openai | low | 308 | 35 |
| glm-5.3-flash | openai | max | 1891 | 1508 |
| glm-5.3 | anthropic | low | 217 | not reported |
| glm-5.3 | anthropic | max | 2516 | not reported |
| glm-5.3 | openai | low | 177 | 109 |
| glm-5.3 | openai | max | 1380 | 1255 |

Both endpoints honor the effort setting (roughly 10x more output at max on this prompt, n = 1 per cell). The Anthropic endpoint returns thinking blocks but does not split thinking tokens out of `output_tokens`. The OpenAI coding endpoint reports `completion_tokens_details.reasoning_tokens`.

## Claude Code forwards effort to GLM (2026-11-03)

Claude Code 2.1.316 ran headless against Z.ai through a local logging proxy (`ANTHROPIC_BASE_URL`), model `glm-5.3-flash`, prompt "Count the primes below 300 and list the last five. Do not use tools.", one run per level.

| `--effort` | `output_config` in the request | Output tokens | Input tokens |
|---|---|---|---|
| low | `{"effort": "low"}` | 84 | 10,741 |
| max | `{"effort": "max"}` | 1,437 | 10,741 |

Both requests also carried `thinking: {"type": "adaptive"}` and `max_tokens: 32000`. The effort setting reaches GLM through Claude Code. Claude Code's own `total_cost_usd` for these runs is computed at Claude prices and is wrong for GLM, so gungnir prices runs from token counts. Claude Code reports `thinking_tokens: 0` for GLM because Z.ai does not split them out.
