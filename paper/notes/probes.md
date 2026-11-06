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

## NVIDIA Nemotron 3.5 Super VL effort passthrough (2026-11-06)

The endpoint is `https://integrate.api.nvidia.com/v1` (`POST /responses`, `Authorization: Bearer`), model `nvidia/nemotron-3.5-super-vl-120b-a12b`, with effort sent as `reasoning.effort`, `store: false` and no streaming. Responses name the model `private/nvidia/nemotron-3.5-super-120b-a12b`.

The endpoint accepts `none`, `minimal`, `low`, `medium`, `high` and `xhigh`. It also takes `max`, and its response then reads `reasoning.effort: "xhigh"`. Anything else is a 400 that lists the six. A `max_output_tokens` above 1,048,576 is a 400 too.

Reasoning tokens per request. Every answer was right.

| Prompt | none | minimal | low | medium | high | xhigh |
|---|---|---|---|---|---|---|
| "How many positive divisors does 360 have? Answer with the number only." (n = 1) | 0 | 119 | 174 | 195 | 158 | 204 (`max`: 111) |
| "How many integers from 1 to 1000 inclusive are divisible by none of 2, 3, 5 and 7? Answer with the number only." (n = 2) | | 2393, 1735 | 2209, 790 | 967, 1928 | 1509, 1597 | 1923, 1014 |
| "How many subsets of {1, 2, ..., 15}, the empty set included, have a sum of elements divisible by 5? Answer with the number only." (n = 4) | | 1597, 1606, 2116, 4196 | 1370, 1504, 1619, 3408 | 1565, 2270, 2308, 3273 | 2000, 2024, 2142, 4078 | 1202, 2186, 2958, 3228 |

`none` turns reasoning off. The levels from `minimal` to `xhigh` reason alike: on the third prompt their medians run 1861, 1562, 2289, 2083 and 2572, in no order, and the spread within each level is wider than the differences between them. The catalog therefore offers `none` and `high` for this model.

## Codex 0.166.0 against an API-key Responses endpoint (2026-11-06)

Codex reaches an endpoint other than OpenAI's through a model provider in `CODEX_HOME/config.toml`: `model_provider` names a `[model_providers.<id>]` table with `name`, `base_url`, `env_key` (the variable holding the API key) and `wire_api = "responses"`. Codex refuses to start when `CODEX_HOME` does not exist, and refuses a provider configured under one of its own ids ("model_providers contains reserved built-in provider IDs: `openai`"). Against NVIDIA it made no request but `POST /v1/responses`.

Codex has no metadata for the Nemotron model. `exec --json` says so in an item, ``{"type":"item.completed","item":{"type":"error","message":"Model metadata for `nvidia/nemotron-3.5-super-vl-120b-a12b` not found. Defaulting to fallback metadata; this can degrade performance and cause issues."}}``, which gungnir reads as noise. The session rollout reports a context window of 258,400 tokens, so Codex compacts its context long before the 1,048,576 tokens the endpoint accepts as `max_output_tokens` alone.

A request carries `reasoning: {effort, summary: "auto"}`, `include: ["reasoning.encrypted_content"]`, `store: false`, `stream: true`, `tool_choice: "auto"`, `parallel_tool_calls: true`, `prompt_cache_key` and `client_metadata`, with seven function tools and one `namespace` tool (`multi_agent_v1`), and the system prompt as developer messages instead of `instructions`. NVIDIA accepted such a request unchanged and streamed a reasoning item with its summary. A real session of shell commands completed, and its usage counted cached input (8,672 of 20,784 input tokens).

How Codex ends a turn against a local stub endpoint that answers every request the same way:

| The stub answers | Requests | Codex's last line |
|---|---|---|
| 429 | 1 | `turn.failed`: "exceeded retry limit, last status: 429 Too Many Requests" |
| 500 | 30 | `turn.failed`: "We’re currently experiencing high demand, which may cause temporary errors." |
| a stream closed before `response.completed` | 6 | `turn.failed`: "stream disconnected before completion: stream closed before response.completed" |
| `response.created`, then nothing for 330 s | a reconnect at 300 s | "Reconnecting... 1/5 (stream disconnected before completion: idle timeout waiting for SSE)" |
| its response headers after 330 s | 1 | `turn.completed` |

The stream idle timeout, 300 s by default, runs only between events, so a request the endpoint queues before it answers never trips it. gungnir counts the 429 as a rate limit and the other two failures as errors, as it does for OpenAI, and leaves Codex's retry and timeout settings at their defaults.
