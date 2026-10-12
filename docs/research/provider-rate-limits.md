# Provider rate-limit headers for pi Usage windows

Research for #105 (map #96). The question: which rate-limit or quota signals do pi's commercial providers send, and how do they map onto orch Usage windows (`UsageWindow { name, label, used_percent, resets_at_unix }`)? Can a pi extension see all of them through `after_provider_response`?

Sources were read on 2026-10-12. No live API calls were made, and no credential files were read.

## Answer in brief

- **Subscription providers carry real Usage windows. API-key providers carry only per-minute token buckets.**
  - The Claude subscription sends `anthropic-ratelimit-unified-{5h,7d,…}-utilization` and `-reset`.
  - The ChatGPT/Codex subscription sends `x-codex-{primary,secondary}-used-percent`, `-window-minutes` and `-reset-at`.
  - The Anthropic, OpenAI and Gemini API-key headers describe RPM/TPM token buckets that refill continuously. They are not quota periods.
- **`after_provider_response` alone is not enough:**
  - **Claude subscription:** visible, but only on successful (2xx) responses.
  - **Codex subscription:** pi's default transport is a WebSocket, and that path never fires `after_provider_response`. The same data does reach extensions as a `codex.rate_limits` frame through `provider_stream_event`.
  - **Gemini:** pi's Google adapter never fires `after_provider_response`, and the Gemini API documents no rate-limit headers anyway.
  - **Google subscription:** pi 1.1.0 has no built-in Google subscription provider.
- **Recommendation:**
  - orch's pi extension reports windows for the two subscriptions only.
  - Claude comes from `after_provider_response`.
  - Codex comes from `after_provider_response` (SSE) plus `provider_stream_event` (`codex.rate_limits`, WebSocket).
  - Window names are prefixed per provider, because the Daemon keys windows by (Agent, name).
  - API-key token buckets report no windows.

## pi 1.1.0: what an extension can see

All paths below are under `~/.pi/agent/install/releases/1.1.0/node_modules/@earendil-works/`.

**The event.** `pi-coding-agent/dist/core/extensions/types.d.ts:693-698`:

```ts
/** Fired after a provider response is received and before the response stream is consumed. */
export interface AfterProviderResponseEvent { type: "after_provider_response"; status: number; headers: Record<string, string>; }
```

The event carries no provider or model. pi-ai's callback does get the model (`onResponse?: (response, model)`, `pi-ai/dist/types.d.ts:118`). But `pi-coding-agent/dist/core/sdk.js:231-240` (`handleProviderResponse`) drops it, so the extension must use `ctx.model` to tell which provider answered. The event is not in `docs/extensions.md`. It exists only in the type declarations.

**Which adapters call `onResponse`** (`pi-ai/dist/api/*.js`):

| API adapter | Used by provider | Calls `onResponse`? | When |
|---|---|---|---|
| `anthropic-messages` | `anthropic` (API key **and** Claude Pro/Max OAuth) | yes, `:443` | only after `retryProviderRequest` succeeds |
| `openai-responses` | `openai` (API key **and** "Sign in with ChatGPT" OAuth) | yes, `:138` | only on success |
| `openai-completions` | OpenAI-compatible providers, llama.cpp | yes, `:202` | only on success |
| `openai-codex-responses` | `openai-codex` (ChatGPT Plus/Pro OAuth) | **SSE path only**, `:283` | every attempt, before the `ok` check, so 429s too |
| `google-generative-ai`, `google-vertex` | `google`, `google-vertex` | **no** | never |

**Errors.** The SDK-based adapters (Anthropic, OpenAI) wrap the request in `retryProviderRequest` (`pi-ai/dist/utils/provider-retry.js:76-98`). A non-2xx response makes the SDK throw, so `onResponse` runs only for the final successful response. On a 429, the headers that explain the limit never reach `after_provider_response`.

**Codex transport.**
- `openai-codex-responses.js:184-190`: the transport defaults to `options.transport || "auto"`, and anything other than `"sse"` tries the WebSocket first.
- `docs/settings.md:121`: the `transport` setting is `"auto" | "sse" | "websocket" | "websocket-cached"`, default `"auto"`.
- The WebSocket is opened in `connectWebSocket` (`:799-811`) with no hook on the upgrade response. Its frames go through `parseWebSocket` (`:1023-1040`), which queues **every** JSON frame, and then `mapCodexEvents` (`:535-543`), which calls `onProviderStreamEvent(event, model)` before it filters on `type`.
- So a `codex.rate_limits` frame reaches the `provider_stream_event` extension event. That event carries `provider`, `api` and `model` (`types.d.ts:699-706`, `docs/extensions.md` "provider_stream_event").
- The SSE fallback (`:248-283`) calls `onResponse` with the HTTP headers.

**Google subscription.**
- `pi-ai/dist/auth/oauth/` has subscription OAuth only for Anthropic (`anthropic.js`), OpenAI (`openai-codex.js`, `openai-chatgpt.js`), GitHub Copilot, Kimi, Meta, OpenRouter, Radius and xAI.
- `providers/google.js` takes only `GEMINI_API_KEY`.
- There is no Gemini CLI or Antigravity (Cloud Code Assist) provider in 1.1.0.

## Per provider

### Anthropic API (API key)

Primary source: [Claude API rate limits, "Response headers"](https://platform.claude.com/docs/en/api/rate-limits#response-headers).

- `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}`.
- `anthropic-priority-{input,output}-tokens-*` (Priority Tier only).
- `retry-after` (seconds).
- Reset values are RFC 3339 times.
- `remaining` for tokens is "rounded to the nearest thousand".
- The limits are token buckets: "capacity is continuously replenished up to your maximum limit, rather than being reset at fixed intervals". Rates are per minute, per model class.
- The monthly spend cap has **no** header. It shows up only as a 429 `rate_limit_error` with `error.details.error_code: "enforced_spend_limit_reached"` and no `retry-after`.
- Fast mode adds `anthropic-fast-*` headers.

**Mapping:** a per-minute bucket could be shown as `used = 1 − remaining/limit` with the reset taken from `*-reset`. That is not a Usage window in the glossary's sense ("a named limit on how much of an Agent's quota is used in a period … resets at a set time"). The "latest value until reset" rule would also flicker every minute. **Recommend no window.**

### Anthropic subscription (Claude Pro/Max via OAuth)

pi sends OAuth tokens (`sk-ant-oat…`) to the same Messages endpoint, using the beta flags `claude-code-20250219` and `oauth-2025-04-20` (`anthropic-messages.js:748-782, 839-840`). So the response is what Claude Code itself receives.

These headers are not publicly documented. The primary source is Anthropic's own client, Claude Code 2.1.296 (`~/.local/share/claude/versions/2.1.296`, a bundled JS binary). Its strings include:

- `anthropic-ratelimit-unified-status` (`allowed`, `allowed_warning`, `rejected`), `-reset`, `-representative-claim` (`five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet`), `-fallback` and `-upgrade-paths`.
- Per window: `anthropic-ratelimit-unified-{5h,7d,7d_oi,overage}-{utilization,reset,surpassed-threshold}`. The windows table is `[["five_hour","5h"],["seven_day","7d"],["seven_day_overage_included","7d_oi"],["overage","overage"]]`, and the parser reads `` `anthropic-ratelimit-unified-${s}-utilization` `` and `-reset` for each entry.
- Also `overage-*`, `grace-{5h,7d}-utilization` and `slow-*` (low-priority offers).
- The bundle documents the units: "utilization is the fraction of the window used (usually 0-1 …; values above 1 occur when usage legitimately runs past a window's cap …); resetsAt is unix epoch seconds … both windows are tracked on every observation … Windows absent from the account state are absent here."

**Mapping:**
- `used_percent = clamp(utilization × 100, 0, 100)` and `resets_at_unix = round(reset)`.
- One window per present `{5h,7d,7d_oi}` key.
- This is what Claude Code's statusline already turns into `rate_limits.five_hour` and `rate_limits.seven_day`, which orch parses in `crates/orch-agent/src/claude/statusline.rs` (names `five_hour`/`seven_day`, labels `5h`/`7d`, `used_percentage` and `resets_at` in epoch seconds).
- **Visible via `after_provider_response` on every successful turn.** It is not visible on the 429 that rejects a turn, but the previous turn's values will already be near 100%.

### OpenAI API (API key, and "Sign in with ChatGPT" on api.openai.com)

Primary source: [OpenAI rate limits, "Rate limits in headers"](https://developers.openai.com/api/docs/guides/rate-limits).

- `x-ratelimit-{limit,remaining,reset}-{requests,tokens}`.
- `x-ratelimit-*-project-tokens` when a project limit applies.
- `Retry-After`.
- Resets are **durations**, not timestamps (examples `1s`, `6m0s`), meaning time "until the rate limit resets to its initial state". They are per-minute buckets.

**Mapping:** same as the Anthropic API. **Recommend no window.**

pi 1.1.0's `openai` provider also offers "OpenAI (ChatGPT subscription)" OAuth. It sends the user token straight to `api.openai.com` (`auth/oauth/openai-chatgpt.js:1-5, 18`), through the same `openai-responses` adapter, so `after_provider_response` fires.

- [OpenAI's help page](https://help.openai.com/articles/20001542) says such requests count against the plan's Codex usage and can have a per-app weekly cap. A cap failure is `subscription_sharing_usage_limit_exceeded` (`openai-responses.js:166-170`).
- **No source says which headers, if any, report that subscription usage on `api.openai.com`.** Treat this as an open question. A future capture of real headers could show whether `x-codex-*` appears there.

### OpenAI Codex subscription (ChatGPT Plus/Pro, `openai-codex` provider)

Primary source: OpenAI's Codex client, [`openai/codex` `codex-rs/codex-api/src/rate_limits.rs`](https://github.com/openai/codex/blob/6e5f665feb9b71e1500a5721036fbdb8e871f4b0/codex-rs/codex-api/src/rate_limits.rs) at `6e5f665`.

**Headers:**
- `x-codex-{primary,secondary}-used-percent` (a float percentage, 0-100), `-window-minutes` and `-reset-at` (unix seconds).
- `x-codex-credits-{has-credits,unlimited,balance}`, `x-codex-limit-name` and `x-codex-rate-limit-reached-type`.
- Other metered limits use the same shape under another prefix: `parse_all_rate_limits` picks up any `x-<limit-id>-primary-…` family, such as `x-codex-other-…`.
- A window whose `used-percent` is 0, with a zero or missing `window-minutes` and no `reset-at`, counts as absent.

**WebSocket frame.** On the WebSocket, the same data comes as a JSON frame (`parse_rate_limit_event`, and `responses_websocket.rs:775-778`):

```json
{"type":"codex.rate_limits","plan_type":"plus",
 "rate_limits":{"allowed":true,"limit_reached":false,
   "primary":{"used_percent":42,"window_minutes":60,"reset_at":1700000000},"secondary":null},
 "credits":{"has_credits":true,"unlimited":false,"balance":"123"}}
```

(fixture from `codex-rs/core/tests/suite/client_websockets.rs`; `metered_limit_name`/`limit_name` pick a non-default limit id).

**User prior art.** The user's `~/.pi/agent/extensions/statusline/codexQuota.ts` parses the same headers.
- It classifies each bucket by `window-minutes` rather than by primary/secondary. Newer servers can make primary the 7-day limit and disable secondary, and `window-minutes <= 0` means absent.
- It works around the missing headers by POSTing its own probe to `/codex/responses` with the stored token and aborting the body. Its comment says pi "never surfaces those headers to extensions". That holds for `after_provider_response` on the WebSocket path. It overlooks `provider_stream_event`, which does carry `codex.rate_limits` frames, if the server sends them to pi's client.
- orch should not copy the probe. It spends a request and reads `auth.json`.

**Mapping:**
- One window per bucket, with `used_percent = used_percent` and `resets_at_unix = reset_at`.
- Label it from `window_minutes`, as `codexQuota.ts` does (≤300 → `5h`, ≤10080 → `7d`, and so on). Do not label from primary/secondary.
- Skip buckets with `window_minutes <= 0`.

**Visibility:**
- SSE transport: `after_provider_response`, on every attempt including 429s.
- WebSocket (the default): `provider_stream_event` with `provider === "openai-codex"` and `data.type === "codex.rate_limits"`.
- Neither path was checked against the live service for pi's client. pi identifies itself with its own `originator` and `User-Agent`, and it is unproven that the server sends `codex.rate_limits` to it. The pi fixtures ticket (#99) used a scripted model, so it does not settle this.

### Google Gemini API (API key)

Primary source: [Gemini API rate limits](https://ai.google.dev/gemini-api/docs/rate-limits) and [troubleshooting](https://ai.google.dev/gemini-api/docs/troubleshooting).

- Limits are RPM, TPM and RPD per project. "RPD quotas reset at midnight Pacific time."
- Over the limit, the API returns `429 RESOURCE_EXHAUSTED`.
- **The docs list no rate-limit response headers.** Current limits are visible only in AI Studio.
- pi's Google adapters (`google-generative-ai.js`, `google-vertex.js`, using `@google/genai`) never call `onResponse`.

**Mapping:** none. The Gemini API reports no windows.

### Google subscription (Gemini Code Assist / Antigravity)

Not reachable from pi 1.1.0, which has no such provider (see above). For reference, orch's Antigravity adapter takes these pools from agy's statusline `quota` map, not from HTTP headers: `crates/orch-agent/src/antigravity/usage.rs` (`gemini-weekly` → `gemini-wk`, `3p-weekly` → `3p-wk`; `used = 1 − remaining_fraction`, RFC 3339 `reset_time`).

## Mapping onto orch

`UsageWindow` (`crates/orch-core/src/event.rs:70-75`) is `name`, `label`, `used_percent` (0-100) and `resets_at_unix`. The Daemon keeps the latest value **per (Agent, window name)** (`crates/orch-daemon/src/usage_windows.rs`, `note`), and the statusline drops a window after its reset (`docs/agent-adapters.md`). All pi Sessions are one Agent, so a user who switches between Claude and Codex in pi would mix two providers' `5h` windows unless the names differ.

| Provider (pi id) | Window `name` (suggested) | `label` | `used_percent` | `resets_at_unix` | Source event |
|---|---|---|---|---|---|
| `anthropic`, OAuth | `anthropic-five_hour` | `cl-5h` | `unified-5h-utilization × 100` | `unified-5h-reset` | `after_provider_response` |
| `anthropic`, OAuth | `anthropic-seven_day` | `cl-7d` | `unified-7d-utilization × 100` | `unified-7d-reset` | `after_provider_response` |
| `anthropic`, OAuth | `anthropic-seven_day_overage_included` | `cl-7d+` | `unified-7d_oi-utilization × 100` | `unified-7d_oi-reset` | `after_provider_response` |
| `openai-codex` | `openai-codex-<window>` (e.g. `-5h`, `-7d`) | `cx-5h`, `cx-7d` | `used_percent` | `reset_at` | `after_provider_response` (SSE) or `provider_stream_event` (`codex.rate_limits`) |
| `anthropic` / `openai` API key, `google` | none | — | — | — | — |

The prefixed names and labels are suggestions for the spec, not decisions. Label width matters to the statusline.

**How to tell an OAuth response from an API-key one:**
- Anthropic: key on the presence of `anthropic-ratelimit-unified-status` rather than on the credential. The extension never sees the token, and only subscription traffic carries the unified family.
- Codex: the `x-codex-*` family or a `codex.rate_limits` frame. Only the `openai-codex` provider produces those.

**Reporting:** the windows are sampled per response, but orch reports a UsageSample at `turn_end` (#100). The extension can keep the latest window values from the provider events and attach them to the next `turn_end` sample.

## Open questions

1. Does chatgpt.com send `codex.rate_limits` frames to pi's WebSocket client (`originator: pi`)? Settle it with a recorded live turn (header names and frame shape only, no tokens), or fall back to `transport: "sse"`. orch should not override the user's transport setting.
2. Does "Sign in with ChatGPT" on `api.openai.com` return any subscription-usage headers? Undocumented.
3. Do Anthropic OAuth responses also carry the standard `anthropic-ratelimit-*` bucket headers? This doesn't matter if API-key buckets are dropped anyway.
4. Should `after_provider_response` gain `provider` and `model` upstream? pi-ai already passes the model to `onResponse`, but `sdk.js` drops it. Using `ctx.model` works but is racy if the model changes mid-request.
