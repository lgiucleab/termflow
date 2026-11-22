# Provider wire formats (verified 2026-11-22)

Summarised from the vendors' current docs. Anthropic is from the Claude API reference; OpenAI is from developers.openai.com, where platform.openai.com now redirects. Items marked *(unverified)* came from summaries, not verbatim text; confirm them with a live request before relying on them. Fixtures in `tests/fixtures/http/` follow these shapes. They are **written from the docs, not recorded from live traffic**; recordings get added when the owner does the manual real-key run.

## Anthropic Messages — `POST /v1/messages`
- Headers: `x-api-key`, `anthropic-version: 2026-10-21`, `content-type: application/json`.
- On 2026-10-21, `temperature`/`top_p`/`top_k` are rejected.
- Thinking: `{"type":"adaptive","display":"summarized"}`; effort lives at `output_config.effort`. `budget_tokens` is rejected on current models.
- Thinking blocks `{type:"thinking", thinking, signature}` go back unchanged. Blocks the model can't read are dropped server-side, unbilled.
- `tool_choice` `any`/`tool` returns 400 on current models: use `auto`. Assistant prefill returns 400.
- SSE events:
  - `message_start` (message.usage)
  - `content_block_start`: `text`, `thinking`, `tool_use{id,name}`
  - `content_block_delta`: `text_delta.text`, `thinking_delta.thinking`, `signature_delta.signature`, `input_json_delta.partial_json`
  - `content_block_stop`
  - `message_delta`: `delta.stop_reason`; `usage.output_tokens` (plus cache fields on `message_start`)
  - `message_stop`, `ping`, `error{error:{type,message}}`
- Errors:
  - `{type:"error", error:{type, message, details?}}`
  - 429 `rate_limit_error`. When `error.details.error_code = enforced_spend_limit_reached` and there's no `retry-after`, it's a spend cap.
  - 529 `overloaded_error`; 413 `request_too_large`; 401 `authentication_error`; 403 `permission_error`; 402 `billing_error`.
- Rate-limit headers: `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}`, `retry-after`.
- `GET /v1/models` returns `data[]{id, display_name, created_at, max_input_tokens, max_tokens, capabilities}`.

## OpenAI Responses — `POST /v1/responses`
- Body:
  - `model`, `instructions`, `input[]`, `tools[{type:"function",name,description,parameters,strict}]`
  - `tool_choice` (`auto`), `store:false`, `include:["reasoning.encrypted_content"]` (now the default for stateless use, still accepted)
  - `reasoning{effort, summary:"auto"}`, `max_output_tokens`, `stream:true`
- Input items:
  - user `{role:"user", content:[{type:"input_text",text}]}`
  - assistant message `{type:"message", role:"assistant", content:[{type:"output_text",text}]}`. Keep its `phase` when replaying.
  - `{type:"function_call", call_id, name, arguments:"<json string>"}`
  - `{type:"function_call_output", call_id, output:"<string>"}`
  - `{type:"reasoning", id, summary[], encrypted_content}`
- Stateless replay: pass back every previous output item **in original order**, then the outputs and the new user message. Reasoning items have to accompany their tool calls.
- Stream events (all carry `type`, `sequence_number`):
  - `response.created`
  - `response.output_item.added{output_index, item}` and `response.output_item.done{output_index, item}`. Take a reasoning item's final `encrypted_content` from `.done`.
  - `response.output_text.delta{item_id, output_index, content_index, delta}`
  - `response.function_call_arguments.delta{item_id, output_index, delta}` and `.done{item_id, arguments}`. These have no name; it comes from `output_item.added`.
  - `response.reasoning_summary_text.delta{item_id, summary_index, delta}` (name confirmed)
  - `response.completed{response}`: usage is `input_tokens`, `input_tokens_details.cached_tokens`/`cache_write_tokens`, `output_tokens`, `output_tokens_details.reasoning_tokens`
  - `response.incomplete{response.incomplete_details.reason ∈ max_output_tokens|max_messages|content_filter|steered}`
  - `response.failed{response.error{code,message,param,type}}`
  - `error{code, message, param}`
- Errors:
  - Envelope `{error:{message,type,param,code}}` *(implied, unverified)*.
  - 429 `rate_limit_error`/`slow_down` (honour `Retry-After`).
  - 429 codes `credit_balance_exhausted`, `organization_spend_limit_exceeded`, `project_spend_limit_exceeded`, `organization_usage_limit_exceeded`, `insufficient_quota`: quota, not retryable.
  - 503 `service_unavailable_error`/`server_is_overloaded`.
- Headers: `x-ratelimit-{limit,remaining,reset}-{requests,tokens}` (reset like `6m0s`), `retry-after`.
- `GET /v1/models` returns `{data[]{id, created, owned_by}}`.

## OpenAI Chat Completions — `POST /chat/completions` (also OpenRouter, Groq, DeepSeek, LM Studio, vLLM, Ollama `/v1`)
- Messages:
  - roles `system|developer|user|assistant|tool`
  - assistant `tool_calls[{id, type:"function", function:{name, arguments:"<json>"}}]`
  - tool `{role:"tool", tool_call_id, content}`
- Request fields: `tools[{type:"function",function:{name,description,parameters}}]`, `stream_options:{include_usage:true}`, `max_completion_tokens` (`max_tokens` is deprecated but is what many compatible servers expect).
- Chunks: `choices[{delta{role?,content?,tool_calls[{index,id?,function{name?,arguments?}}]}, finish_reason}]`. `finish_reason` is `stop|length|tool_calls|content_filter`.
- Usage: a final chunk (empty `choices` on OpenAI) carries `usage{prompt_tokens, completion_tokens, prompt_tokens_details.cached_tokens, completion_tokens_details.reasoning_tokens}`, then `data: [DONE]`.
- **OpenRouter specifics:**
  - Base `https://openrouter.ai/api/v1`; optional `HTTP-Referer`, `X-OpenRouter-Title`.
  - `model` in the response is the model that actually served the request.
  - `: OPENROUTER PROCESSING` comment lines.
  - The usage chunk keeps one choice.
  - Mid-stream errors are a chunk with a top-level `error{code,message,metadata{error_type}}` plus `finish_reason:"error"`.
  - `reasoning{effort}` / `reasoning_details[]` (`reasoning.text|summary|encrypted`, with `id`, `format`, `index`) must go back **unchanged and in order** *(summary-sourced)*.
  - Request-level errors are `{error:{code:<http>, message, metadata?}}`.

## Ollama native — `POST /api/chat` (NDJSON)
- Request:
  - `model`
  - `messages[{role, content, thinking?, images?, tool_calls?, tool_name?}]`
  - `tools[{type:"function",function:{...}}]`, `think`, `stream:true`, `options{num_ctx,...}`
- Lines:
  - `{message:{role:"assistant", content, thinking?, tool_calls?[{function:{name, arguments:{...object}}}]}, done:false}`
  - final `{done:true, done_reason, prompt_eval_count, eval_count, ...}`
- Tool calls come whole, in one line, with no id: synthesize one.
- Tool results are sent as `{role:"tool", content, tool_name}`.
- `GET /api/tags` returns `models[]{name, model, size, details{family, parameter_size, quantization_level}}` *(summary-sourced)*.
- `POST /api/show` returns `capabilities[]` plus `model_info["<arch>.context_length"]` *(summary-sourced)*.
