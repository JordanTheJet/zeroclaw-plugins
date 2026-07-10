# bluesky — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to [Bluesky](https://bsky.app) over the AT Protocol. It
polls the account's notifications for incoming **mentions and replies** and
sends the agent's responses back as **threaded posts** — all from a sandboxed
`wasm32-wasip2` WIT component, with no native build.

```bash
zeroclaw plugin install bluesky \
  --registry https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json
```

## Configuration

Credentials and settings come from the plugin's config section (requires the
`config_read` permission). Fields:

- `handle` (required) — the account handle / identifier, e.g.
  `mybot.bsky.social`. Sent as the `identifier` to `createSession`.
- `app_password` (required) — an **app password** from Bluesky
  Settings → App Passwords (not your account password).
- `service` — PDS / service origin; defaults to `https://bsky.social`. Override
  for a self-hosted PDS or a test mock.

On a host with the `provides` feature this plugin **mirrors** the built-in
`bluesky` channel and reads `[channels.bluesky.<alias>]`; on older hosts it
loads as a novel channel configured from `[[plugins.entries.bluesky]].config`.
Native-only fields (`enabled`, `excluded_tools`) are ignored by the plugin.

## Permissions

- `http_client` — outbound XRPC calls to the PDS (TLS is performed host-side).
- `config_read` — read the handle + app password above.

## What's covered

`src/bluesky.rs` holds the pure AT-Protocol logic — notification → message
mapping (mentions/replies only, unread, self-loop guard), the `createSession` /
`createRecord` / `updateSeen` payload builders, the reply-threading strong-ref
encode/decode, the 300-char post clamp, and dependency-free RFC-3339 ⇄
Unix-millis conversion — with host `cargo test` coverage in `tests/`.
`src/lib.rs` is the thin component shim that does the HTTP via the blocking
[`waki`](https://crates.io/crates/waki) `wasi:http` client, caches the session
JWT (re-authenticating on a 401), and tracks the notification cursor.

Text posts are supported today; media embeds, rich facets (link/mention
cards), and interactive approvals are future work.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
