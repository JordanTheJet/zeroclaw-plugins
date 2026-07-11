# DingTalk channel plugin

This plugin mirrors `[channels.dingtalk.<alias>]` through `provides =
"dingtalk"`. It implements DingTalk Stream Mode rather than a webhook listener:

1. register `client_id` and `client_secret` with the gateway API;
2. connect to the returned WebSocket endpoint through the host `ws-client`;
3. acknowledge system and callback frames;
4. deliver inbound text with private/group reply routing;
5. reply through the `sessionWebhook` supplied with each inbound message.

The implementation is real but remains `registry = false` until ZeroClaw's
host-mediated WebSocket capability lands on upstream master. It builds and runs
against the `channel-to-plugin` host branch that provides `ws-client`.

## Configuration

```toml
[channels.dingtalk.default]
enabled = true
client_id = "<DingTalk AppKey>"
client_secret = "<encrypted AppSecret>"
```

Outbound messages require a prior inbound message for that chat because
DingTalk creates the session reply URL dynamically.

## Validation

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
