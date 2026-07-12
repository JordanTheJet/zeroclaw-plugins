# wecom-ws - ZeroClaw WeCom AI Bot channel plugin

This plugin implements the WeCom AI Bot long-connection text protocol through
ZeroClaw's host-mediated `ws-client` WIT import. The host owns the WebSocket and
TLS; the component owns subscription, application-level ping frames, callback
decoding, access control, reply encoding, and reconnect state.

The plugin mirrors the native `wecom_ws` channel with `provides = "wecom_ws"`.
Its only configuration source is the JSON section the host injects from
`[channels.wecom_ws.<alias>]`:

```toml
[channels.wecom_ws.primary]
enabled = true
bot_id = "your-wecom-bot-id"
secret = "your-wecom-bot-secret"
allowed_users = ["operator-userid"]
allowed_groups = ["approved-chatid"]
bot_name = "assistant"
stream_mode = "partial"
```

Empty `allowed_users` and `allowed_groups` deny all inbound callbacks. A `"*"`
entry allows every identifier in that list; all other identifiers are matched
exactly. `bot_name` supplies the self handle and `@mention` metadata.

ZeroClaw's generic plugin-channel authorizer also checks the callback sender
user ID against the alias's live `peer_groups` external peers. Consequently,
an `allowed_groups` entry does not bypass the host sender gate; group users must
also be present there (or the peer group must contain `"*"`).

## Supported behavior

- Connect to `wss://openws.work.weixin.qq.com` and send `aibot_subscribe`.
- Correlate the subscribe acknowledgement by `headers.req_id` without waiting
  or sleeping inside `poll-message`.
- Drain text frames in bounded batches and return immediately on an idle socket.
- Decode direct and group `aibot_msg_callback` text messages, including quoted
  text, and preserve the callback request ID for active-session replies.
- Suppress replayed message IDs with a bounded 4096-entry session cache.
- Reply with final or progressive `aibot_respond_msg` stream frames.
- Send proactive markdown with `aibot_send_msg` to `user--<userid>` and
  `group--<chatid>` recipients, using UTF-8-safe chunks.
- Send WeCom application ping frames every 30 seconds and reconnect with bounded
  exponential backoff on closure, transport failure, subscribe timeout, or a
  `disconnected_event`.

## Current limits

Only the protocol text path is implemented. Voice, image, file, and mixed
callbacks are ignored; media download, AES decryption, reactions, welcome-card
events, and typing indicators are not available. `proxy_url`, file retention,
and file-size config are native media/transport concerns and are not consumed by
this component. WebSocket command acknowledgements arrive asynchronously, so a
successful `send` confirms host-buffer acceptance rather than remote delivery.

`registry = false` remains required because the `plugins-wit-v0-websocket` host
capability is not on upstream master. A live test additionally requires WeCom
AI Bot credentials and a ZeroClaw host built with that capability; the focused
tests use protocol fixtures and do not contact WeCom.

## Validation

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
