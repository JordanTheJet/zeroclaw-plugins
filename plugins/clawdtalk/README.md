# clawdtalk - ZeroClaw channel plugin source

This directory is the Phase 4 migration landing point for the built-in
`clawdtalk` channel. The manifest declares `provides = "clawdtalk"`, so
when it becomes publishable it will read the existing `[channels.clawdtalk.*]`
configuration as the single source of truth and honor the native-wins policy.

Current status: **source-only / host-gated**. The plugin exports the channel WIT
surface, parses configuration, reports identity metadata, and can drain messages
that a future host-managed listener queues for it. Direct send/poll transport is
not published yet:

ClawdTalk protocol I/O remains host-gated until the native transport is exposed to channel plugins.

Because `registry = false`, CI keeps this source in the repo but does not build,
package, or advertise it in `registry.json`. Remove that guard only when protocol
parity has tests and the required host capability is available to stock hosts.

## Build

```bash
cargo test
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```
