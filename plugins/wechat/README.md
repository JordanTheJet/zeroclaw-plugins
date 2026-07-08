# wechat — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to a personal WeChat account through the **iLink Bot** API
(`ilinkai.weixin.qq.com`). It long-polls `getupdates` for incoming messages and
sends the agent's replies with `sendmessage` — all from a sandboxed
`wasm32-wasip2` WIT component, with no native build.

```bash
zeroclaw plugin install wechat \
  --registry https://raw.githubusercontent.com/JordanTheJet/zeroclaw-plugins/main/registry.json
```

## Session / login

The iLink Bot API is authorized with a `bot_token` that is obtained by scanning
a **QR code** with the WeChat phone app. That login is interactive (render a QR
to a terminal, then long-poll for the scan), which cannot run inside the wasm
sandbox. So this plugin does **not** perform the QR login itself — you establish
the session once with the native `zeroclaw` `wechat` channel (which renders the
QR and persists the token to `~/.zeroclaw/wechat/account.json`), then copy that
`bot_token` into this plugin's config.

Without a `bot_token` the plugin is inert: `poll` returns nothing and `send`
returns a clear error telling you the session is missing. If the token later
expires (iLink `errcode -14`), the plugin drops it and goes inert again until a
fresh token is supplied.

## Configuration

Settings come from the plugin's config section (requires the `config_read`
permission). Field names mirror the native `WeChatConfig`, so a
`[channels.wechat.<alias>]` section is fed to the plugin verbatim.

- `bot_token` — the iLink session token from a one-time native QR login (see
  above). Required for the plugin to do anything. This is the one key the native
  config does not itself store in `config.toml` (it lives in the state dir); set
  it here explicitly.
- `api_base_url` — iLink API origin; defaults to `https://ilinkai.weixin.qq.com`.
  Override for a test mock.
- `cdn_base_url` — iLink CDN origin; defaults to
  `https://novac2c.cdn.weixin.qq.com/c2c`. Accepted for parity (media transfer
  is future work in this plugin).
- `state_dir` / `enabled` / `excluded_tools` — accepted for native-section
  parity; the sandboxed plugin does not act on them.

On a host with the `provides` feature this plugin **mirrors** the built-in
`wechat` channel and reads `[channels.wechat.<alias>]`; on older hosts it loads
as a novel channel configured from `[[plugins.entries.wechat]].config`.

## Permissions

- `http_client` — outbound calls to the iLink Bot API (TLS is performed
  host-side).
- `config_read` — read the settings + session token above.

## What's covered

`src/wechat.rs` holds the pure iLink logic (message → inbound mapping,
`getupdates` / `sendmessage` / `getconfig` body building, error/session-expiry
classification, cursor + context-token handling, a Markdown→plain-text pass, and
the `X-WECHAT-UIN` Base64 helper) with host `cargo test` coverage in `tests/`.
`src/lib.rs` is the thin component shim that does the HTTP via the blocking
[`waki`](https://crates.io/crates/waki) `wasi:http` client, sending the iLink
auth headers (`Authorization: Bearer …`, `AuthorizationType: ilink_bot_token`,
`X-WECHAT-UIN`).

Inbound text and voice-transcription messages are delivered today; the poll caps
the server hold (`longpolling_timeout_ms: 0`) so it never stalls `send`. WeChat
conversations are 1:1, so a reply's recipient is the sender's `from_user_id`, and
the per-sender `context_token` harvested on poll is reused to thread the reply.
Media (images, files, voice notes) and the `/bind` pairing flow are handled by
the native channel and are future work here.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
