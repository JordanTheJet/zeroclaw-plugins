# mochat — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to a self-hosted [Mochat](https://mochat.io) customer-service
server. It polls the Mochat REST API for incoming messages and sends the agent's
replies back — all from a sandboxed `wasm32-wasip2` WIT component, with no native
build.

```bash
zeroclaw plugin install mochat \
  --registry https://raw.githubusercontent.com/JordanTheJet/zeroclaw-plugins/main/registry.json
```

## Configuration

The API URL and token come from the plugin's config section (requires the
`config_read` permission). Field names match the built-in `mochat` channel:

- `api_url` (required) — base URL of your self-hosted Mochat server (e.g.
  `https://mochat.example.com`). A trailing slash is trimmed.
- `api_token` (required) — API token, sent as `Authorization: Bearer <token>`.
- `poll_interval_secs` — receive-poll interval in seconds; defaults to `5`.
- `enabled` / `excluded_tools` — carried for parity with the native config;
  interpreted host-side.

On a host with the `provides` feature this plugin **mirrors** the built-in
`mochat` channel and reads `[channels.mochat.<alias>]`; on older hosts it loads
as a novel channel configured from `[[plugins.entries.mochat]].config`.

## Endpoints

All relative to the configured `api_url`, matching the native channel:

- `GET  /api/message/receive[?since_id=<id>]` — poll for new messages.
- `POST /api/message/send` — send a text reply
  (`{ "toUserId", "msgType": "text", "content": { "text" } }`).
- `GET  /api/health` — reachability probe for `health_check`.

Inbound messages map `fromUserId`/`sender` → sender, `content.text` (or a bare
`content` string) → text, and `messageId`/`id` → the dedup + `since_id` cursor.

## Permissions

- `http_client` — outbound calls to your Mochat server (TLS is performed
  host-side).
- `config_read` — read the URL + token + settings above.

## What's covered

`src/mochat.rs` holds the pure REST logic (message → inbound mapping, send-body
building, response-code checks, URL building, dedup) with host `cargo test`
coverage in `tests/`. `src/lib.rs` is the thin component shim that does the HTTP
via the blocking [`waki`](https://crates.io/crates/waki) `wasi:http` client.

Text messages are supported today; media and typing indicators are future work
(the Mochat REST API exposes no typing endpoint).

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
