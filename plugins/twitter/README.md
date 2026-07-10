# twitter — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to X/Twitter. It polls the API v2 mentions timeline for
incoming mentions and posts the agent's replies back — all from a sandboxed
`wasm32-wasip2` WIT component, with no native build.

```bash
zeroclaw plugin install twitter \
  --registry https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json
```

## Configuration

The bearer token and settings come from the plugin's config section (requires
the `config_read` permission). Fields:

- `bearer_token` (required) — the X API v2 OAuth 2.0 Bearer Token.
- `enabled` — mirrors the native flag; the host decides whether to load the
  channel, so this is inert inside the plugin.
- `api_base_url` — API origin; defaults to `https://api.x.com/2`. Override for a
  test mock.
- `allowed_users` — allow-list of author ids / usernames. `["*"]` allows anyone;
  empty means no plugin-level gating. Matching strips a leading `@` and is
  case-insensitive.
- `excluded_tools` — carried for native-config parity; host-side concern.

On a host with the `provides` feature this plugin **mirrors** the built-in
`twitter` channel and reads `[channels.twitter.<alias>]`; on older hosts it
loads as a novel channel configured from `[[plugins.entries.twitter]].config`.

## Permissions

- `http_client` — outbound calls to `api.x.com` (TLS is performed host-side).
- `config_read` — read the token + settings above.

## Endpoints

- `GET /2/users/me` — resolve + cache the bot's own user id and `@handle`.
- `GET /2/users/{id}/mentions?tweet.fields=author_id,created_at` — short poll for
  new mentions, paginating with `since_id` from the previous batch's
  `meta.newest_id`.
- `POST /2/tweets` — reply with `{ text, reply: { in_reply_to_tweet_id } }`;
  long replies thread as a self-reply chain (280-char tweets).

All requests carry `Authorization: Bearer {bearer_token}`.

## What's covered

`src/twitter.rs` holds the pure API-v2 logic (mention → message mapping, poll
URL + cursor advance, `POST /tweets` payload building, allow-list, 280-char
chunking) with host `cargo test` coverage in `tests/`. `src/lib.rs` is the thin
component shim that does the HTTP via the blocking
[`waki`](https://crates.io/crates/waki) `wasi:http` client.

Text mentions and reply tweets are supported today; DMs, media, and reactions
are future work. The mentions endpoint is heavily rate-limited, so each
`poll_message` issues a single short request rather than a long-poll.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
