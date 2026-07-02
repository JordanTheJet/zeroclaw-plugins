# redact-text

The canonical ZeroClaw **WIT component** tool plugin (adopted from
[zeroclaw-reference-plugin](https://github.com/singlerider/zeroclaw-reference-plugin),
renamed for what it does). It implements the `tool-plugin` world from `wit/v0`
and compiles to a `wasm32-wasip2` component. Copy it as the starting point for
a real tool plugin.

## What it does

A `redact` tool. It scrubs secrets and PII out of text before that text reaches
a log, a channel, or a model: email addresses, bearer/API tokens (`sk-`, `ghp_`,
`AKIA`, `xoxb-`, …), and any literal patterns the operator configures.

The redaction policy comes entirely from the plugin's own config section, which
makes this the reference for the three things every config-aware plugin must do:

1. **Own a config section.** The operator configures the plugin by name in
   `config.toml`; the host resolves that one section and hands the plugin a flat
   `string -> string` map.
2. **Deserialize that config.** `execute` reads the injected `__config` object
   out of its arguments and parses it into a typed `RedactConfig`.
3. **Stay jailed.** The host only injects the section when the manifest requests
   the `config_read` permission. Without it the plugin receives an empty map and
   falls back to defaults. A plugin can never read the global config or another
   plugin's section.

## Config keys

| Key | Default | Meaning |
|---|---|---|
| `replacement` | `[REDACTED]` | String substituted for each match. |
| `redact_emails` | `true` | Mask email-shaped substrings. |
| `patterns` | (empty) | Comma-separated literal patterns to also mask. |

## Layout (the reference format)

```
src/redact.rs   # pure logic, no wasm deps — host-testable with `cargo test`
src/lib.rs      # thin #[cfg(target_family = "wasm")] component shim
tests/          # host-run integration tests over the pure core
manifest.toml   # name, version, wasm_path, capabilities, permissions
```

## Build and test

```bash
cargo test                                        # host tests, no wasm needed
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release      # the component
cp target/wasm32-wasip2/release/redact_text.wasm redact_text.wasm
```

## Install

```bash
zeroclaw plugin install redact-text
```

or copy this directory (the `.wasm` next to its `manifest.toml`) into your
configured plugins dir, then enable plugins and (optionally) configure it:

```toml
[plugins]
enabled = true
```

Run the agent with a build that includes a compiler backend, e.g.
`--features plugins-wasm,plugins-wasm-cranelift`. For runtime-only hosts
(`--features plugins-wasm`), precompile with a matching wasmtime:
`wasmtime compile --target <triple> redact_text.wasm -o redact_text.cwasm` and
point `wasm_path` at the `.cwasm`.
