# webhook — ZeroClaw channel plugin

A WASM (`wasm32-wasip2`) channel plugin mirroring the built-in generic
**webhook** channel — the "universal adapter" for any system that speaks HTTP.
`provides = "webhook"`, so it reads the existing `[channels.webhook.<alias>]`
config as the single source of truth and honors native-wins.

## How it works

The host serves `POST /plugin/webhook`. On each request the plugin:

1. **Verifies** the signature when a `secret` is configured: the request must
   carry `X-Webhook-Signature: [sha256=]<hex(HMAC-SHA256(secret, body))>`. A bad
   or missing signature is rejected with `401`. When no secret is set, all inbound
   is accepted (matching the native channel).
2. **Decodes** the JSON body `{sender, content, thread_id?}` into an inbound
   message. Empty `content` (or invalid JSON) is a `400`. `reply_target` is the
   `thread_id` when present, else the `sender`.

Replies are sent to `send_url` as `{content, thread_id?, recipient?}` via `POST`
(or `PUT` when `send_method = "PUT"`), with the optional `auth_header` value sent
as `Authorization`. When `send_url` is unset, outbound is a no-op.

## Config (`[channels.webhook.<alias>]`)

- `send_url` — outbound reply endpoint (optional; no-op when unset).
- `send_method` — `POST` (default) or `PUT`.
- `auth_header` — optional `Authorization` header value for outbound.
- `secret` — optional HMAC-SHA256 signing secret for inbound verification.

The native `port` / `listen_path` fields are **ignored**: as a plugin, inbound
arrives on the host's `/plugin/webhook` route (the host owns the listener), not a
private port. They are accepted for config compatibility.

## Scope / deferrals

- **Text only.** No attachments.
- No sender allowlist in the plugin; the host gates senders via `peer_groups`.
- **Outbound retry/backoff is deferred.** The native channel retries transient
  failures (429/5xx, `Retry-After`, exponential backoff with jitter). This plugin
  performs a **single** outbound attempt and returns `Err` on a non-2xx, letting
  the host's own send-error handling take over. Signature verification and the
  inbound/outbound payload shapes are fully faithful.

## Build

```bash
cargo test --lib
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```
