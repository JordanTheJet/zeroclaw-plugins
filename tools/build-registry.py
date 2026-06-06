#!/usr/bin/env python3
"""Build the ZeroClaw plugin registry.

For each staged plugin directory (containing a `manifest.toml` and its built
`.wasm`), this:
  1. zips the directory under a top-level `<name>/` folder,
  2. computes the zip's SHA-256,
  3. emits a `registry.json` entry pointing at the release-asset URL.

The zips are uploaded as GitHub Release assets; only `registry.json` (small,
text) is committed to the repo. `zeroclaw plugin install <name>` reads
`registry.json`, downloads the zip, verifies the SHA-256, and installs it.

Usage:
  build-registry.py --staged <dir> --release-base <url> --out <dir>
"""
import argparse
import hashlib
import json
import zipfile
from pathlib import Path

try:
    import tomllib  # py3.11+
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None


def parse_manifest(path: Path) -> dict:
    if tomllib is not None:
        return tomllib.loads(path.read_text())
    # Minimal fallback: scalar `key = "value"` lines only.
    out: dict = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, val = line.partition("=")
        out[key.strip()] = val.strip().strip('"')
    return out


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
    for pdir in sorted(p for p in staged.iterdir() if p.is_dir()):
        manifest = pdir / "manifest.toml"
        if not manifest.exists():
            continue
        meta = parse_manifest(manifest)
        name = meta["name"]
        version = meta.get("version", "0.0.0")
        zip_name = f"{name}-{version}.zip"
        zip_path = out / zip_name

        with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
            for f in sorted(pdir.rglob("*")):
                if f.is_file():
                    z.write(f, f"{name}/{f.relative_to(pdir)}")

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

    (out / "registry.json").write_text(json.dumps({"plugins": entries}, indent=2) + "\n")
    print(f"wrote registry.json with {len(entries)} entr{'y' if len(entries) == 1 else 'ies'}")


if __name__ == "__main__":
    main()
