# ZeroClaw Plugin Registry

The official catalog of [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
WASM plugins — **self-contained WIT components** the agent can fetch and install
on demand. This is what `zeroclaw plugin search` and
`zeroclaw plugin install <name>` read by default.

```bash
zeroclaw plugin search wikipedia
zeroclaw plugin install wikipedia-summary
zeroclaw plugin install wikipedia-summary@0.1.0    # pin a version
```

## What's in this repo

```
plugins/<name>/        # one wit-bindgen component per directory
  Cargo.toml           # cdylib + wit-bindgen, standalone [workspace]
  src/lib.rs           # implements the `tool-plugin` WIT world
  manifest.toml        # name, version, capabilities, permissions, [[credentials]]
  README.md
wit/v0/                # vendored ZeroClaw plugin WIT contract (the ABI plugins build against)
registry.json          # GENERATED index — published by CI, do not hand-edit
tools/build-registry.py
.github/workflows/publish.yml
```

Plugins are **WebAssembly components** built for `wasm32-wasip2` against the WIT
world `tool-plugin` (`wit/v0/`). They run sandboxed and deny-by-default: the
host grants only the capabilities a plugin's `manifest.toml` declares.

## How install works

- [`registry.json`](./registry.json) is a single index, fetched from
  `https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json`.
- Each entry's `url` points to a zipped plugin directory published as a
  **GitHub Release asset** — binaries are never committed to git, only the small
  text index.
- On install the CLI downloads the zip, **verifies the `sha256`** (transport
  integrity), then the host enforces the configured **Ed25519 `signature_mode`**
  (authenticity).

### Index format

```json
{
  "plugins": [
    {
      "name": "wikipedia-summary",
      "version": "0.1.0",
      "description": "Look up a short factual summary of a topic from Wikipedia",
      "author": "ZeroClaw Labs",
      "capabilities": ["tool"],
      "url": "https://github.com/zeroclaw-labs/zeroclaw-plugins/releases/download/plugins/wikipedia-summary-0.1.0.zip",
      "sha256": "<hex digest of the zip>"
    }
  ]
}
```

`registry.json` is **generated** — the [publish workflow](./.github/workflows/publish.yml)
builds every `plugins/*`, packages the zips, uploads them to the `plugins`
release, and commits a refreshed index. It starts empty and is populated on the
first publish run.

## Add a plugin

1. Create `plugins/<your-plugin>/` with `Cargo.toml` (cdylib + `wit-bindgen`),
   `src/lib.rs` implementing the `tool-plugin` world, and a `manifest.toml`. Use
   `plugins/wikipedia-summary` (no auth) and `plugins/mastodon-post`
   (host-injected credentials) as templates.
2. Build it:
   ```bash
   rustup target add wasm32-wasip2
   (cd plugins/<name> && cargo build --target wasm32-wasip2 --release)
   ```
3. Open a PR. On merge, the publish workflow packages and indexes it.

The contract in `wit/v0/` is vendored from
[zeroclaw `wit/v0`](https://github.com/zeroclaw-labs/zeroclaw/tree/master/wit/v0);
bump it together when the plugin ABI version changes.

## Secrets / credentials

A plugin never sees secret values. It declares `[[credentials]]` in its
`manifest.toml`, and the host injects the secret into matching outbound requests
(e.g. `Authorization: Bearer <token>`) at the HTTP boundary — see
`plugins/mastodon-post`. The plugin can only check existence via the
`secret_exists` permission.

## Run your own registry

```bash
zeroclaw plugin install <name> --registry https://my-host/registry.json
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://my-host/registry.json
```
