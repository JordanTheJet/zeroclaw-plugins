# Incubating plugins

Plugins here are **not published to the registry** (the publish workflow only
builds `plugins/*`). They compile and are kept current, but they target a
**proposed** host capability interface — [`wit/v0/host.wit`](./wit/v0/host.wit)
(`http-request`, `secret-exists`, `workspace-read`, `tool-invoke`) — that the
ZeroClaw host does not implement yet.

The real, host-implemented contract lives at the repo root
[`../wit/v0`](../wit/v0), vendored from
[zeroclaw `wit/v0`](https://github.com/zeroclaw-labs/zeroclaw/tree/master/wit/v0):
its `tool-plugin` world is `import logging; export plugin-info; export tool;` —
no host imports. A component that imports `zeroclaw:plugin/host` fails to
instantiate on today's host, which is why these plugins would break for anyone
who installed them.

| Plugin | Blocked on |
|---|---|
| `wikipedia-summary` | `host.http-request` |
| `mastodon-post` | `host.http-request`, `host.secret-exists`, host-injected `[[credentials]]` |

The relevant upstream work is the wasm-first runtime RFC
([zeroclaw#8135](https://github.com/zeroclaw-labs/zeroclaw/issues/8135)) and the
plugin program tracker ([zeroclaw#7314](https://github.com/zeroclaw-labs/zeroclaw/issues/7314)).
When the host ships a capability interface, promote these back to `plugins/`:
align `wit/v0` here with what actually landed, point each plugin's
`wit_bindgen::generate!` path back at `../../wit/v0`, and the publish workflow
picks them up on merge.

Building (they build today, against the proposed contract):

```bash
cd <plugin> && cargo build --target wasm32-wasip2 --release
```
