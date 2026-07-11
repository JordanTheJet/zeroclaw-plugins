# WeCom Bot Webhook channel plugin

This plugin mirrors `[channels.wecom.<alias>]` through `provides = "wecom"` and
implements the same send-only Bot Webhook mode as ZeroClaw's native channel.
Text is posted through the host's `wasi:http` implementation, and both HTTP and
WeCom JSON error responses are surfaced to the caller.

## Configuration

```toml
[channels.wecom.default]
enabled = true
webhook_key = "<encrypted bot webhook key>"
```

The recipient field is ignored because a WeCom webhook key is bound to one bot
conversation. For inbound messages and active-session replies, use the separate
`wecom-ws` channel, which remains source-only until WebSocket host support lands.

## Validation

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
