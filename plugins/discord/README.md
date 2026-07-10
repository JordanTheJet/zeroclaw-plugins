# discord — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to Discord. It runs the full Discord **Gateway** protocol —
connect, IDENTIFY, heartbeats, RESUME, and `MESSAGE_CREATE` dispatch — over the
host's WebSocket capability, and sends replies over the REST API, all from a
sandboxed `wasm32-wasip2` WIT component with no native build.

```bash
zeroclaw plugin install discord \
  --registry https://raw.githubusercontent.com/JordanTheJet/zeroclaw-plugins/main/registry.json
```

> **Host requirement.** Discord needs a persistent duplex socket, which a
> sandboxed plugin cannot open. This plugin imports the host `ws-client`
> capability, so it runs only on a ZeroClaw built with the WebSocket capability
> (`plugins-wit-v0-websocket`) and granted the `websocket_client` permission.
> On a host without it the plugin fails closed at load rather than running
> without a socket.

## Configuration

The bot token and settings come from the plugin's config section (requires the
`config_read` permission). Fields:

- `bot_token` (required) — the token from the
  [Discord Developer Portal](https://discord.com/developers/applications). Sent
  as `Authorization: Bot <token>`.
- `guild_ids` — allow-list of guild (server) IDs. Empty = all invited guilds.
- `channel_ids` — allow-list of channel IDs. Empty = every visible channel.
  (Direct-match only; thread→parent resolution is done host-side.)
- `listen_to_bots` — process messages from other bots (webhook messages carry
  `author.bot = true`, so they are also gated by this). Default `false`. The
  bot's own messages are always dropped.
- `mention_only` — in guilds, only respond to messages that @-mention the bot.
  DMs are always answered. Default `false`.
- `intents_mask` — raw gateway IDENTIFY intent bitmask override (including `0`).
  Unset uses the baseline `37377` (guilds + guild/DM messages + MESSAGE_CONTENT).
- `api_base` — REST + gateway-discovery origin; defaults to
  `https://discord.com/api/v10`. Override for a self-hosted proxy or a test mock.
- `gateway_url` — connect to this Gateway WSS base directly instead of
  discovering it via `GET /gateway/bot`. For self-hosted gateways and tests.

On a host with the `provides` feature this plugin **mirrors** the built-in
`discord` channel and reads `[channels.discord.<alias>]`; on older hosts it
loads as a novel channel configured from `[[plugins.entries.discord]].config`.
Ownership and peer allow-listing (`[peer_groups]`) are enforced host-side and do
not reach the plugin's config.

## Permissions

- `websocket_client` — the Discord Gateway (a persistent WebSocket the host
  dials + pumps; TLS host-side).
- `http_client` — REST sends (`POST /channels/{id}/messages`) and gateway
  discovery (`GET /gateway/bot`).
- `config_read` — read the token + settings above.

## What's covered

`src/discord.rs` holds the pure protocol logic — config parsing, gateway frame
parsing, IDENTIFY/RESUME/HEARTBEAT payload builders, the `MESSAGE_CREATE` filter
pipeline, message chunking, and the [`Session`] state machine
([`Session::on_frame`]) — with full host `cargo test` coverage (every opcode
transition). `src/lib.rs` is the component shim: it owns the `ws-client` handle,
the heartbeat/backoff timers, and reconnect/resume, and translates the machine's
actions into WebSocket + `wasi:http` (blocking [`waki`](https://crates.io/crates/waki))
I/O, driving the whole protocol synchronously from `poll-message`.

Text messages (send + receive, guild + DM) are supported today. Attachments,
reactions, threads, slash commands, and draft/streamed replies are future work.
Fatal Gateway close codes (bad token/intents) stop reconnection when the host
surfaces the numeric close code; otherwise a close is treated as transient and
retried with capped exponential backoff.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core + state machine, on the host
cargo build --release --target wasm32-wasip2 # the component
```
