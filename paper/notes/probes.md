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


