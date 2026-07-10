# nostr — ZeroClaw channel plugin

A ZeroClaw WIT **channel** plugin for [Nostr](https://github.com/nostr-protocol/nostr).
Nostr clients talk to relays over a persistent WebSocket, so this plugin drives
the relay protocol over the host-mediated **`ws-client`** import (the plugin
never opens a socket itself — the host dials the relay, performs TLS, and pumps
frames; the plugin owns the application protocol).

It mirrors the built-in `nostr` channel: it declares `provides = "nostr"` and
reads the same `[channels.nostr.<alias>]` config, so on a host with the
`provides` feature it is a drop-in for the native channel (native wins when
both are present).

## Scope (v0.1.0): receive-only, plaintext notes

This build proves the WebSocket round-trip: **subscribe to a relay and surface
each received plaintext note (kind 1) as an inbound message.** The pure core
does the protocol work — build the `["REQ", …]` frame, decode `["EVENT", …]` /
`EOSE` / `NOTICE` / `OK` / `CLOSED` / `AUTH` frames, and map a note onto the
host's inbound fields (`sender`/`reply_target` = author hex pubkey,
`content` = note text, `timestamp` = created_at in ms).

Two features are deliberately **deferred** — both need secp256k1 (schnorr) and
AES, which are too heavy for the dependency-free pure core:

- **Encrypted DMs are not read.** Kind-4 (NIP-04) and NIP-17 gift-wrapped DMs
  require secp256k1 ECDH + AES to decrypt, so they are filtered out. We
  subscribe to **kind 1** (public notes / mentions) instead.
- **Outbound `send` is not implemented.** Publishing a note means schnorr-signing
  an event with your private key (BIP-340 over secp256k1). Until that lands,
  `send` returns an explicit error rather than silently dropping the reply.

The follow-up is to pull in a pure-Rust schnorr/secp256k1 implementation
(e.g. `k256` with the `schnorr` feature) so the plugin can sign events (enabling
`send`) and do NIP-04/NIP-17 decrypt (enabling encrypted DMs), plus `bech32` to
accept `npub…`/`nsec…` keys.

## Config — `[channels.nostr.<alias>]`

Mirrors the native channel, plus a few plugin conveniences:

- `relays` (array of `wss://` URLs) — the plugin connects to the **first** one
  (single-relay in v0.1.0; fan-out is a follow-up). Defaults to the same public
  relay set as the native channel when omitted.
- `relay_url` / `relay` (string) — convenience aliases appended to `relays`.
- `pubkey` / `public_key` (64-char hex) — **optional but recommended.** When
  set, the subscription narrows to notes that `#p`-tag you (mentions/replies)
  and it is reported as the bot's self-handle so the runtime drops your own
  notes. When absent, the plugin samples the relay's recent kind-1 notes.
  (The native channel derives this from `private_key`; the pure core can't, so
  it is supplied explicitly. `npub…` is not yet accepted — use hex.)
- `private_key` / `secret_key` (hex or nsec) — read and retained for the future
  signed `send`; unused in this receive-only build.
- `allowed_pubkeys` (array of hex pubkeys, or `["*"]`) — sender allow-list.
  Empty = allow everyone; non-empty gates by author.
- `kinds` (array of ints) — event kinds to subscribe to (default `[1]`).
- `subscription_id` (string, default `"sub1"`) and `limit` (int, default `20`).

## Capabilities & permissions

- `get_channel_capabilities()` = `HEALTH_CHECK | SELF_HANDLE` (no
  `WEBHOOK_INGRESS` — this is a WebSocket, not a webhook, channel).
- Permissions: `websocket_client` (host-mediated relay socket) and `config_read`.

## Build & test

```sh
# pure core, host:
cargo test --lib

# wasm component:
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
# → target/wasm32-wasip2/release/nostr.wasm  (copy next to manifest.toml)
```
