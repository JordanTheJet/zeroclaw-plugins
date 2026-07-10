#!/usr/bin/env python3
"""Build the ZeroClaw plugin registry.

For each staged plugin directory (containing a `manifest.toml` and its built
`.wasm`), this:
  1. validates the staged plugin against the host's install contract,
  2. zips the directory under a top-level `<name>/` folder — reproducibly:
     fixed timestamps and permissions, so identical content always produces
     an identical zip and sha256 across CI runs (the refreshed registry.json
     only changes when plugin content actually changes),
  3. computes the zip's SHA-256,
  4. emits a `registry.json` entry pointing at the release-asset URL.

The zips are uploaded as GitHub Release assets; only `registry.json` (small,
text) is committed to the repo. `zeroclaw plugin install <name>` reads
`registry.json`, downloads the zip, verifies the SHA-256, and installs it.

Entries are sorted by (name, version). The host resolves an unpinned install
to the LAST matching entry in file order, so within a name the newest version
must sort last — the version key below handles numeric dotted versions.

Requires Python 3.11+ (tomllib). Honors SOURCE_DATE_EPOCH for the embedded
zip timestamps (zip cannot represent dates before 1980-01-01).

Usage:
  build-registry.py --staged <dir> --release-base <url> --out <dir>
"""
import argparse
import hashlib
import json
import os
import re
import sys
import time
import zipfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    sys.exit("error: build-registry.py requires Python 3.11+ (tomllib)")

# Mirror the host's install caps (src/plugin_registry.rs MAX_PLUGIN_ZIP_BYTES /
# MAX_PLUGIN_EXTRACTED_BYTES) so an oversized plugin fails the publish run
# instead of every user's `zeroclaw plugin install`.
MAX_ZIP_BYTES = 50 * 1024 * 1024
MAX_EXTRACTED_BYTES = 50 * 1024 * 1024

# The host's manifest schema (crates/zeroclaw-plugins/src/lib.rs). An unknown
# value here makes the host reject the whole manifest at parse time, so catch
# it at publish time.
KNOWN_CAPABILITIES = {"tool", "channel", "memory", "observer", "skill"}
KNOWN_PERMISSIONS = {
    "http_client",
    "file_read",
    "file_write",
    "config_read",
    "env_read",  # serde alias for config_read
    "memory_read",
    "memory_write",
    "websocket_client",
}

NAME_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")


def zip_date_time() -> tuple:
    """Fixed timestamp for reproducible zips (SOURCE_DATE_EPOCH or 1980-01-01)."""
    dos_min = 315532800  # 1980-01-01, the earliest zip can represent
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", dos_min))
    t = time.gmtime(max(epoch, dos_min))
    return (t.tm_year, t.tm_mon, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec)


def version_key(version: str) -> tuple:
    """Sort key making the newest numeric dotted version sort last."""
    parts = []
    for chunk in version.split("."):
        m = re.match(r"^(\d+)", chunk)
        parts.append((int(m.group(1)) if m else -1, chunk))
    return tuple(parts)


def validate(pdir: Path, meta: dict) -> list:
    """Check a staged plugin against the host's install contract."""
    errors = []
    name = meta.get("name")
    if not isinstance(name, str) or not NAME_RE.match(name or ""):
        errors.append(f"name {name!r} must be lowercase kebab-case")
    elif name != pdir.name:
        errors.append(f"name {name!r} does not match staged directory {pdir.name!r}")

    caps = meta.get("capabilities")
    if not isinstance(caps, list):
        caps = []
    if not caps:
        errors.append("capabilities must be a non-empty array")
    for cap in sorted(set(caps) - KNOWN_CAPABILITIES):
        errors.append(f"unknown capability {cap!r} (host rejects the manifest)")

    perms = meta.get("permissions", [])
    if not isinstance(perms, list):
        errors.append("permissions must be an array")
        perms = []
    for perm in sorted(set(perms) - KNOWN_PERMISSIONS):
        errors.append(f"unknown permission {perm!r} (host rejects the manifest)")

    wasm_path = meta.get("wasm_path")
    needs_wasm = bool(set(caps) - {"skill"})
    if needs_wasm:
        if not wasm_path:
            errors.append("wasm_path is required for non-skill-only plugins")
        else:
            wasm = pdir / wasm_path
            if not wasm.is_file():
                errors.append(f"wasm_path {wasm_path!r} not found in staged dir")
            elif wasm.stat().st_size == 0:
                errors.append(f"wasm_path {wasm_path!r} is empty")

    extracted = sum(f.stat().st_size for f in pdir.rglob("*") if f.is_file())
    if extracted > MAX_EXTRACTED_BYTES:
        errors.append(
            f"extracted size {extracted} exceeds the host's "
            f"{MAX_EXTRACTED_BYTES}-byte install cap"
        )
    return errors


def write_zip(zip_path: Path, pdir: Path, name: str) -> None:
    """Zip `pdir` under a top-level `<name>/`, reproducibly."""
    date_time = zip_date_time()
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted(pdir.rglob("*")):
            if not f.is_file():
                continue
            info = zipfile.ZipInfo(
                f"{name}/{f.relative_to(pdir).as_posix()}", date_time=date_time
            )
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, f.read_bytes())


def main() -> None:
    ap = argparse.ArgumentParser(description="Build the ZeroClaw plugin registry")
    ap.add_argument("--staged", required=True, help="dir of <plugin>/{manifest.toml,*.wasm}")
    ap.add_argument("--release-base", required=True, help="base URL for release assets")
    ap.add_argument("--out", required=True, help="output dir for zips + registry.json")
    args = ap.parse_args()

    staged = Path(args.staged)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    entries = []
    failures = []
    seen = set()
    for pdir in sorted(p for p in staged.iterdir() if p.is_dir()):
        manifest = pdir / "manifest.toml"
        if not manifest.exists():
            continue
        try:
            meta = tomllib.loads(manifest.read_text())
        except tomllib.TOMLDecodeError as e:
            failures.append(f"{pdir.name}: invalid manifest.toml: {e}")
            continue

        errors = validate(pdir, meta)
        if errors:
            failures.extend(f"{pdir.name}: {e}" for e in errors)
            continue

        name = meta["name"]
        version = meta.get("version", "0.0.0")
        if (name, version) in seen:
            failures.append(f"{pdir.name}: duplicate entry {name}@{version}")
            continue
        seen.add((name, version))

        zip_name = f"{name}-{version}.zip"
        zip_path = out / zip_name
        write_zip(zip_path, pdir, name)

        if zip_path.stat().st_size > MAX_ZIP_BYTES:
            failures.append(
                f"{pdir.name}: zip size {zip_path.stat().st_size} exceeds the "
                f"host's {MAX_ZIP_BYTES}-byte download cap"
            )
            continue

        sha = hashlib.sha256(zip_path.read_bytes()).hexdigest()
        entry = {
            "name": name,
            "version": version,
            "description": meta.get("description"),
            "author": meta.get("author"),
            "capabilities": meta.get("capabilities", []),
            "url": f"{args.release_base.rstrip('/')}/{zip_name}",
            "sha256": sha,
        }
        entries.append({k: v for k, v in entry.items() if v is not None})
        print(f"  packaged {name} v{version}  sha256={sha[:12]}…")

    if failures:
        for f in failures:
            print(f"error: {f}", file=sys.stderr)
        sys.exit(1)

    # Host install-by-name takes the last matching entry: newest must sort last.
    entries.sort(key=lambda e: (e["name"], version_key(e["version"])))

    tmp = out / "registry.json.tmp"
    tmp.write_text(json.dumps({"plugins": entries}, indent=2) + "\n")
    os.replace(tmp, out / "registry.json")
    print(f"wrote registry.json with {len(entries)} entr{'y' if len(entries) == 1 else 'ies'}")


if __name__ == "__main__":
    main()
