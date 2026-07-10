# reddit — ZeroClaw channel plugin

A [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) **channel** plugin that
connects your agent to Reddit as an OAuth2 bot. It long-polls the inbox for
unread mentions, DMs, and comment replies and posts the agent's replies back —
all from a sandboxed `wasm32-wasip2` WIT component, with no native build.

```bash
zeroclaw plugin install reddit \
  --registry https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json
```

## Configuration

The OAuth2 credentials and settings come from the plugin's config section
(requires the `config_read` permission). Fields (they mirror the built-in
`reddit` channel's snake_case keys):

- `client_id` (required) — Reddit OAuth2 app client ID.
- `client_secret` (required) — Reddit OAuth2 app client secret. Sent with
  `client_id` as HTTP Basic auth to the token endpoint.
- `refresh_token` (required) — OAuth2 refresh token for persistent access. It is
  exchanged for a short-lived access token via the `refresh_token` grant, which
  is cached and re-fetched automatically on a `401`.
- `username` (required) — the bot's Reddit username (without the `u/` prefix).
  Used for the self-loop guard and as the bot's self-handle.
- `subreddits` — optional allow-list (without the `r/` prefix). Empty accepts
  items from any subreddit the bot can see; DMs (which carry no subreddit) are
  always accepted. Matching is case-insensitive.

On a host with the `provides` feature this plugin **mirrors** the built-in
`reddit` channel and reads `[channels.reddit.<alias>]`; on older hosts it loads
as a novel channel configured from `[[plugins.entries.reddit]].config`.

## How it works

- **Auth** — `POST https://www.reddit.com/api/v1/access_token` with HTTP Basic
  (`client_id:client_secret`) and `grant_type=refresh_token`. The access token is
  cached and re-fetched on a `401`.
- **Inbound** — `GET https://oauth.reddit.com/message/unread?limit=25`. Each
  unread item is mapped to an inbound message (`id = reddit_<fullname>`,
  `sender = author`, `content = body`); the batch is then acknowledged with
  `POST /api/read_message` so items are never re-delivered.
- **Outbound** — a fullname recipient (`t1_`/`t3_`/`t4_`) is a threaded comment
  reply via `POST /api/comment` (`thing_id`, `text`); any other recipient is a DM
  via `POST /api/compose` (`to`, `subject`, `text`).

Every request carries the required `User-Agent`. A single short poll per
`poll_message` keeps the client within Reddit's 60 requests/minute cap.

## Permissions

- `http_client` — outbound calls to `oauth.reddit.com` and `www.reddit.com` (TLS
  is performed host-side).
- `config_read` — read the credentials + settings above.

## What's covered

`src/reddit.rs` holds the pure OAuth2/REST logic (inbox item → message mapping,
subreddit allow-list, recipient classification, base64/HTTP-Basic header, token
parsing) with host `cargo test` coverage in `tests/`. `src/lib.rs` is the thin
component shim that does the HTTP via the blocking
[`waki`](https://crates.io/crates/waki) `wasi:http` client.

Text comments and DMs are supported today; media and rich formatting are future
work.

## Build

```bash
rustup target add wasm32-wasip2
cargo test                                   # pure core, on the host
cargo build --release --target wasm32-wasip2 # the component
```
