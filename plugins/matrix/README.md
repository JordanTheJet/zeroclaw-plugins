# Matrix channel plugin

This plugin mirrors the built-in `matrix` channel through `provides = "matrix"`
and reads the existing `[channels.matrix.<alias>]` configuration. It talks to
the Matrix Client-Server API through the host's `wasi:http` implementation.

## Supported

- access-token authentication and `/account/whoami` health checks
- startup backlog suppression followed by incremental `/sync` polling
- unencrypted `m.text` messages in joined rooms
- exact `allowed_rooms` filtering and `mention_only` handling
- room-ID or room-alias recipients
- top-level and threaded text replies

## Current limits

- encrypted rooms and recovery keys are not supported
- password login is not supported; configure `access_token`
- media, edits, reactions, typing indicators, and progressive drafts are not supported

Unsupported events are ignored rather than delivered with incomplete content.

## Configuration

```toml
[channels.matrix.default]
enabled = true
homeserver = "https://matrix.example.org"
access_token = "<encrypted secret>"
user_id = "@zeroclaw:example.org"
allowed_rooms = ["!room:example.org"]
mention_only = false
reply_in_thread = true
```

`user_id` is optional because the plugin resolves it with `/account/whoami`.

## Validation

```bash
cargo fmt --check
cargo test --lib
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
