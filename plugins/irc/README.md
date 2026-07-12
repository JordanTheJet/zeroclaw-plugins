# IRC channel plugin

This plugin mirrors `[channels.irc.<alias>]` through `provides = "irc"` and
implements the IRC text path over the host-mediated socket capability:

- verified TLS connection and IRC registration;
- optional server password, SASL PLAIN, and NickServ identification;
- nickname collision recovery and configured channel joins;
- PING/PONG, channel and direct `PRIVMSG` receive/send;
- mention-only filtering, line-length splitting, and command-injection checks.

It remains `registry = false` until ZeroClaw's `socket_client` host
capability reaches upstream. IRC media, DCC, STARTTLS, and insecure
`verify_tls = false` connections are not supported.

## Configuration

```toml
[channels.irc.default]
enabled = true
server = "irc.example.net"
port = 6697
nickname = "zeroclaw"
channels = ["#bots"]
mention_only = false
```

Optional `username`, `server_password`, `nickserv_password`, and
`sasl_password` fields match the native channel configuration.

## Validation

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
