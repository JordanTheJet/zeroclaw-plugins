# ZeroClaw Plugin Registry

The official catalog for [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
WASM plugins. This is what `zeroclaw plugin search` and
`zeroclaw plugin install <name>` read by default.

```bash
zeroclaw plugin search music
zeroclaw plugin install image-gen-fal
zeroclaw plugin install image-gen-fal@0.1.0      # pin a version
```

## How it works

- [`registry.json`](./registry.json) is a single index of available plugins.
  The ZeroClaw CLI fetches it from
  `https://raw.githubusercontent.com/zeroclaw-labs/zeroclaw-plugins/main/registry.json`.
- Each entry's `url` points to a zipped plugin directory published as a
  **GitHub Release asset** on this repo. Binaries are never committed to git —
  only the small text index is.
- On install, the CLI downloads the zip, **verifies the `sha256`** (transport
  integrity), then hands it to the host, which enforces the configured
  **Ed25519 `signature_mode`** (authenticity).

### Index format

```json
{
  "plugins": [
    {
      "name": "image-gen-fal",
      "version": "0.1.0",
      "description": "Generate images from text prompts using fal.ai Flux models",
      "author": "ZeroClaw Labs",
      "capabilities": ["tool"],
      "url": "https://github.com/zeroclaw-labs/zeroclaw-plugins/releases/download/plugins/image-gen-fal-0.1.0.zip",
      "sha256": "<hex digest of the zip>"
    }
  ]
}
```

Each zip contains the plugin directory (`manifest.toml`, the `.wasm`, and an
optional `skills/` subtree) under a top-level `<name>/` folder.

## Run your own registry

Point the CLI at any host:

```bash
zeroclaw plugin install <name> --registry https://my-host/registry.json
# or, globally:
export ZEROCLAW_PLUGIN_REGISTRY_URL=https://my-host/registry.json
```

## Maintenance

Plugins live in the [`plugins/`](https://github.com/zeroclaw-labs/zeroclaw/tree/master/plugins)
directory of the main repo. The [publish workflow](./.github/workflows/publish.yml)
checks out the main repo, builds every `plugins/*` to `wasm32-unknown-unknown`,
runs [`tools/build-registry.py`](./tools/build-registry.py) to package the zips
and regenerate `registry.json`, uploads the zips to the `plugins` release, and
commits the refreshed index. The catalog grows automatically as plugin PRs merge.
