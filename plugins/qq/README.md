# QQ Official Bot channel plugin

This plugin mirrors `[channels.qq.<alias>]` through `provides = "qq"` and
implements the QQ Official Bot text path:

- OAuth app-access-token acquisition and expiry-aware refresh;
- gateway discovery over HTTPS;
- host-mediated WebSocket Identify/Resume and heartbeat frames;
- C2C and group-at text dispatch with message deduplication;
- markdown text sends to `user:<openid>` and `group:<openid>` recipients.

The implementation is real but remains `registry = false` until ZeroClaw's
`websocket_client` host capability lands upstream. Media upload/download,
voice transcription, attachments, and the native per-channel `proxy_url`
override remain explicit follow-up work.

## Configuration

```toml
[channels.qq.default]
enabled = true
app_id = "<QQ Bot App ID>"
app_secret = "<encrypted App Secret>"
```

## Validation

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
