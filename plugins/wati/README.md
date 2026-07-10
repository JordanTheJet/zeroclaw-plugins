# wati — ZeroClaw channel plugin

A WASM (`wasm32-wasip2`) channel plugin mirroring the built-in **WATI**
(WhatsApp Business API) channel. `provides = "wati"`, so it reads the existing
`[channels.wati.<alias>]` config as the single source of truth and honors
native-wins.

## How it works

WATI delivers messages by POSTing webhooks, so this is a **webhook** channel, not
a poller. The host serves `GET` + `POST` on `/plugin/wati`:

- **GET** — WATI's verification handshake (`?hub.challenge=…`). The plugin echoes
  the `hub.challenge` back verbatim. WATI sends **no** `hub.verify_token`, so
  (matching the native gateway) there is no token check.
- **POST** — a message event. The plugin decodes WATI's variable-shape payload
  (`waId`/`wa_id`/`from`, top-level `text` or `message.text`/`message.body`,
  `fromMe`/`owner`) into inbound text messages.

Replies are sent via `POST <api_url>/api/ext/v3/conversations/messages/text`
(Bearer `api_token`) over the host's `wasi:http`.

## Config (`[channels.wati.<alias>]`)

- `api_token` — WATI API token (Bearer auth), required to send.
- `api_url` — API base (default `https://live-mt-server.wati.io`).
- `tenant_id` — optional; prefixes the send `target` as `<tenant>:<msisdn>`.

## Security

WATI does **not** sign its inbound webhooks (no HMAC/signature header), and the
native gateway performs no authenticity check on the inbound body — it relies on
the sender allowlist (`peer_groups`), which the host applies to plugin-returned
messages. This plugin therefore performs **no** signature verification. If you
need transport authenticity, place the `/plugin/wati` route behind a secret path
or a network ACL.

## Scope / deferrals

- **Text only** (send + receive). Inbound media and WATI's voice-note
  transcription are **not** ported (the native channel downloads media and runs a
  transcription provider host-side; that is out of scope for a text v0.1.0
  plugin). Non-text events yield no message.
- Typing indicators are no-ops (the WATI API has none), matching the native
  channel.

## Build

```bash
cargo test --lib
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```
