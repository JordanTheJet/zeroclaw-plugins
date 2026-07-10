# linq — ZeroClaw channel plugin

A WASM (`wasm32-wasip2`) channel plugin mirroring the built-in **Linq** channel
(iMessage / RCS / SMS via the Linq Partner V3 API). `provides = "linq"`, so it
reads the existing `[channels.linq.<alias>]` config as the single source of truth
and honors native-wins.

## How it works

Linq delivers messages by POSTing webhooks, so this is a **webhook** channel, not
a poller. The host serves `POST /plugin/linq`. On each request the plugin:

1. **Verifies** the signature when `signing_secret` is configured: the request
   must carry `X-Webhook-Signature: [sha256=]<hex(HMAC-SHA256(secret,
   "{X-Webhook-Timestamp}.{body}"))>` and a fresh `X-Webhook-Timestamp` (within a
   300 s replay window). A bad signature is rejected with `401`. When no secret is
   set, inbound is accepted without verification (matching the native gateway).
2. **Decodes** the payload. Both webhook shapes are supported — the legacy
   (`chat_id` / `from` / `is_from_me` / `message.parts`) and the current
   `2026-02-03` shape (`chat.id` / `sender_handle` / `direction` / `parts`). Only
   `message.received` events yield messages; outgoing ones are skipped.

`reply_target` is the `chat_id` when present (so replies land in the same
conversation), else the sender's number. Replies are sent to the chat
(`POST /chats/<id>/messages`); on a `404` the plugin creates a new chat
(`POST /chats`) from `from_phone`.

## Config (`[channels.linq.<alias>]`)

- `api_token` — Linq Partner API token (Bearer auth), required to send.
- `from_phone` — E.164 number to send from (used when creating a new chat).
- `signing_secret` — optional; enables `X-Webhook-Signature` verification.

## Scope / deferrals

- **Text only.** Inbound **images** are surfaced as an `[IMAGE:<url>]` marker
  (matching the native channel); no media is downloaded. Other non-text parts are
  skipped.
- No sender allowlist in the plugin; the host gates senders via `peer_groups`.
- Inbound `timestamp` is left `0` (the native `created_at` is RFC 3339 and this
  I/O-free core carries no date parser). Typing indicators are no-ops.

## Build

```bash
cargo test --lib
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```
