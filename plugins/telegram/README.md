# telegram — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to Telegram. It long-polls the Bot API for incoming
messages and sends the agent's replies back — all from a sandboxed
`wasm32-wasip2` WIT component, with no native build.

```bash
zeroclaw plugin install telegram \
  --registry https://raw.githubusercontent.com/JordanTheJet/zeroclaw-plugins/main/registry.json
```

## Configuration

The bot token and settings come from the plugin's config section (requires the
`config_read` permission). Fields:

- `bot_token` (required) — the token from [@BotFather](https://t.me/BotFather).
- `api_base_url` — API origin; defaults to `https://api.telegram.org`. Override
  for a self-hosted Bot API server or a test mock.
- `parse_mode` — optional Telegram `parse_mode` (`HTML`, `MarkdownV2`, …).
  Unset means messages are sent as plain text (the safe default).
- `allowed_users` — allow-list of usernames / numeric ids. `["*"]` allows
  anyone; empty means no plugin-level gating. Matching strips a leading `@` and
  is case-insensitive.

On a host with the `provides` feature this plugin **mirrors** the built-in
`telegram` channel and reads `[channels.telegram.<alias>]`; on older hosts it
loads as a novel channel configured from `[[plugins.entries.telegram]].config`.

## Permissions

- `http_client` — outbound calls to the Bot API (TLS is performed host-side).
- `config_read` — read the token + settings above.

## What's covered

`src/telegram.rs` holds the pure Bot-API logic (update → message mapping,
`sendMessage` payload building, allow-list, chunking) with host `cargo test`
coverage in `tests/`. `src/lib.rs` is the thin component shim that does the
HTTP via the blocking [`waki`](https://crates.io/crates/waki) `wasi:http`
client.

Text messages are supported today; rich formatting, media, and inline-keyboard
approvals are future work.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
