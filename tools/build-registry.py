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
  4. merges a `registry.json` entry pointing at the release-asset URL into the
     immutable existing registry history.

The zips are uploaded as GitHub Release assets; only `registry.json` (small,
text) is committed to the repo. `zeroclaw plugin install <name>` reads
`registry.json`, downloads the zip, verifies the SHA-256, and installs it.

Entries are sorted by (name, version). The host resolves an unpinned install
to the LAST matching entry in file order, so within a name the newest version
must sort last — the version key below handles numeric dotted versions.

Requires Python 3.11+ (tomllib). Honors SOURCE_DATE_EPOCH for the embedded
zip timestamps (zip cannot represent dates before 1980-01-01).

It can also synchronize or check the checked-in registry metadata against the
canonical source manifests without rebuilding artifacts.

Usage:
  build-registry.py --staged <dir> --release-base <url> --out <dir> \
    [--existing-registry registry.json]
  build-registry.py --source-plugins plugins --check-metadata registry.json
  build-registry.py --source-plugins plugins --sync-metadata registry.json
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
    "socket_client",
    "websocket_client",
}
KNOWN_SENDER_MATCH = {"exact", "case_insensitive", "handle", "email"}

NAME_RE = re.compile(r"^[a-z0-9][a-z0-9_-]*$")
VERSION_RE = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+"
    r"(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?"
    r"(?:\+[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?$"
)

# `manifest.toml` is canonical. These are the manifest-owned fields projected
# into registry entries; release-only `url` and `sha256` are added separately.
REGISTRY_METADATA_FIELDS = (
    "name",
    "version",
    "description",
    "author",
    "capabilities",
    "provides",
    "sender_match",
)


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


def validate(pdir: Path, meta: dict, *, require_wasm: bool = True) -> list:
    """Check a plugin manifest against the host's install contract."""
    errors = []
    name = meta.get("name")
    if not isinstance(name, str) or not NAME_RE.match(name or ""):
        errors.append(f"name {name!r} must be a lowercase slug")
    elif name != pdir.name:
        errors.append(f"name {name!r} does not match staged directory {pdir.name!r}")

    version = meta.get("version")
    if not isinstance(version, str) or not VERSION_RE.fullmatch(version):
        errors.append(f"version {version!r} must be a safe semantic version")

    # `manifest.toml` owns the release identity. When validating a source tree,
    # fail if Cargo's build metadata would export a different package version.
    # Staged release directories intentionally omit Cargo.toml.
    cargo_manifest = pdir / "Cargo.toml"
    if cargo_manifest.is_file():
        try:
            cargo_meta = tomllib.loads(cargo_manifest.read_text())
            cargo_version = cargo_meta.get("package", {}).get("version")
            if cargo_version != version:
                errors.append(
                    f"Cargo.toml package.version {cargo_version!r} does not match "
                    f"canonical manifest version {version!r}"
                )
        except (OSError, tomllib.TOMLDecodeError) as error:
            errors.append(f"cannot validate Cargo.toml package.version: {error}")

    for field in ("description", "author", "provides", "sender_match"):
        value = meta.get(field)
        if value is not None and (not isinstance(value, str) or not value.strip()):
            errors.append(f"{field} must be a non-empty string when present")
    sender_match = meta.get("sender_match")
    if isinstance(sender_match, str) and sender_match not in KNOWN_SENDER_MATCH:
        errors.append(
            f"unknown sender_match {sender_match!r}; expected one of "
            f"{sorted(KNOWN_SENDER_MATCH)!r}"
        )

    registry = meta.get("registry", True)
    if not isinstance(registry, bool):
        errors.append("registry must be a boolean when present")

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
    if needs_wasm and require_wasm:
        if not wasm_path:
            errors.append("wasm_path is required for non-skill-only plugins")
        else:
            wasm = pdir / wasm_path
            if not wasm.is_file():
                errors.append(f"wasm_path {wasm_path!r} not found in staged dir")
            elif wasm.stat().st_size == 0:
                errors.append(f"wasm_path {wasm_path!r} is empty")

    if require_wasm:
        extracted = sum(f.stat().st_size for f in pdir.rglob("*") if f.is_file())
        if extracted > MAX_EXTRACTED_BYTES:
            errors.append(
                f"extracted size {extracted} exceeds the host's "
                f"{MAX_EXTRACTED_BYTES}-byte install cap"
            )
    return errors


def manifest_registry_metadata(meta: dict) -> dict:
    """Project canonical manifest fields into one registry metadata view."""
    entry = {
        "name": meta["name"],
        "version": meta["version"],
        "description": meta.get("description"),
        "author": meta.get("author"),
        "capabilities": meta.get("capabilities", []),
        "provides": meta.get("provides"),
        "sender_match": meta.get("sender_match"),
    }
    return {key: value for key, value in entry.items() if value is not None}


def registry_metadata_view(entry: dict) -> dict:
    """Return only canonical-manifest metadata from a registry entry."""
    return {
        key: entry[key]
        for key in REGISTRY_METADATA_FIELDS
        if key in entry and entry[key] is not None
    }


def load_registry(path: Path) -> tuple[dict, dict]:
    """Load a registry and index entries by immutable `(name, version)`."""
    try:
        registry = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read registry {path}: {error}") from error
    entries = registry.get("plugins") if isinstance(registry, dict) else None
    if not isinstance(entries, list):
        raise ValueError(f"registry {path} must contain a plugins array")

    by_key = {}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise ValueError(f"registry entry {index} must be an object")
        name = entry.get("name")
        version = entry.get("version")
        if not isinstance(name, str) or not isinstance(version, str):
            raise ValueError(f"registry entry {index} must have string name and version")
        key = (name, version)
        if key in by_key:
            raise ValueError(f"registry {path} has duplicate entry {name}@{version}")
        by_key[key] = entry
    return registry, by_key


def source_manifest_metadata(source_plugins: Path) -> tuple[dict, set, list]:
    """Load current canonical source metadata, including registry opt-outs."""
    enabled = {}
    disabled = set()
    failures = []
    for pdir in sorted(path for path in source_plugins.iterdir() if path.is_dir()):
        manifest = pdir / "manifest.toml"
        if not manifest.exists():
            continue
        try:
            meta = tomllib.loads(manifest.read_text())
        except (OSError, tomllib.TOMLDecodeError) as error:
            failures.append(f"{pdir.name}: invalid manifest.toml: {error}")
            continue
        errors = validate(pdir, meta, require_wasm=False)
        if errors:
            failures.extend(f"{pdir.name}: {error}" for error in errors)
            continue
        key = (meta["name"], meta["version"])
        if key in enabled or key in disabled:
            failures.append(f"{pdir.name}: duplicate source entry {key[0]}@{key[1]}")
            continue
        if meta.get("registry") is False:
            disabled.add(key)
        else:
            enabled[key] = manifest_registry_metadata(meta)
    return enabled, disabled, failures


def sync_registry_metadata(source_plugins: Path, registry_path: Path, *, check: bool) -> None:
    """Synchronize current registry metadata from canonical source manifests."""
    try:
        registry, actual_by_key = load_registry(registry_path)
    except ValueError as error:
        sys.exit(f"error: {error}")
    expected, disabled, failures = source_manifest_metadata(source_plugins)
    changed = False
    matched = 0
    pending = []

    for key, metadata in expected.items():
        actual = actual_by_key.get(key)
        if actual is None:
            # A source manifest becomes indexable only after its immutable
            # artifact is built. Missing keys are pending new versions/plugins,
            # not metadata drift; packaging adds them with URL + digest.
            pending.append(key)
            continue
        matched += 1
        actual_metadata = registry_metadata_view(actual)
        if actual_metadata == metadata:
            continue
        if check:
            failures.append(
                f"registry metadata drift for {key[0]}@{key[1]}: "
                f"expected {metadata!r}, found {actual_metadata!r}"
            )
            continue
        release_fields = {
            key: value for key, value in actual.items() if key not in REGISTRY_METADATA_FIELDS
        }
        actual.clear()
        actual.update(metadata)
        actual.update(release_fields)
        changed = True

    disabled_present = disabled & actual_by_key.keys()
    for key in sorted(disabled_present):
        if check:
            failures.append(f"registry=false plugin is indexed: {key[0]}@{key[1]}")
        else:
            registry["plugins"].remove(actual_by_key[key])
            changed = True

    if failures:
        for failure in failures:
            print(f"error: {failure}", file=sys.stderr)
        sys.exit(1)

    if check:
        print(f"registry metadata matches {matched} indexed canonical manifest entries")
        for name, version in pending:
            print(f"  pending unpublished source: {name}@{version}")
        return
    if changed:
        tmp = registry_path.with_suffix(f"{registry_path.suffix}.tmp")
        tmp.write_text(json.dumps(registry, indent=2) + "\n")
        os.replace(tmp, registry_path)
        print(f"synchronized registry metadata for {matched} indexed entries")
    else:
        print("registry metadata already synchronized")
    for name, version in pending:
        print(f"  pending unpublished source: {name}@{version}")


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
    ap.add_argument("--staged", help="dir of <plugin>/{manifest.toml,*.wasm}")
    ap.add_argument("--release-base", help="base URL for release assets")
    ap.add_argument("--out", help="output dir for zips + registry.json")
    ap.add_argument(
        "--existing-registry",
        help="immutable registry history to merge; changed name/version pairs fail",
    )
    ap.add_argument("--source-plugins", help="canonical plugins source directory")
    metadata_mode = ap.add_mutually_exclusive_group()
    metadata_mode.add_argument("--check-metadata", help="fail if registry metadata has drifted")
    metadata_mode.add_argument("--sync-metadata", help="rewrite registry metadata from manifests")
    args = ap.parse_args()

    metadata_registry = args.check_metadata or args.sync_metadata
    if metadata_registry:
        if not args.source_plugins:
            ap.error("--source-plugins is required with metadata modes")
        sync_registry_metadata(
            Path(args.source_plugins),
            Path(metadata_registry),
            check=bool(args.check_metadata),
        )
        return
    if not args.staged or not args.release_base or not args.out:
        ap.error("--staged, --release-base, and --out are required for packaging")

    staged = Path(args.staged)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    entries = []
    existing_by_key = {}
    if args.existing_registry:
        try:
            existing, existing_by_key = load_registry(Path(args.existing_registry))
        except ValueError as error:
            sys.exit(f"error: {error}")
        entries.extend(existing["plugins"])
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

        # Host-gated / source-only plugins opt out of the install registry with
        # `registry = false`. Their source can land before the required host
        # capability is safe to publish, but stock hosts must not be offered an
        # entry they cannot run.
        if meta.get("registry") is False:
            print(f"  skipping {pdir.name} (registry = false: host-gated source)")
            continue

        errors = validate(pdir, meta)
        if errors:
            failures.extend(f"{pdir.name}: {e}" for e in errors)
            continue

        name = meta["name"]
        version = meta["version"]
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
            **manifest_registry_metadata(meta),
            "url": f"{args.release_base.rstrip('/')}/{zip_name}",
            "sha256": sha,
        }
        existing_entry = existing_by_key.get((name, version))
        if existing_entry is not None:
            existing_sha = str(existing_entry.get("sha256", "")).removeprefix("sha256:")
            zip_path.unlink(missing_ok=True)
            if not existing_sha or existing_sha.lower() != sha.lower():
                failures.append(
                    f"{pdir.name}: refusing to overwrite immutable package "
                    f"{name}@{version}: staged sha256 {sha} differs from registry "
                    f"sha256 {existing_sha or '<missing>'}; bump the manifest version "
                    "before publishing"
                )
                continue
            if registry_metadata_view(existing_entry) != manifest_registry_metadata(meta):
                failures.append(
                    f"{pdir.name}: registry metadata drift for existing package "
                    f"{name}@{version}; synchronize metadata before publishing"
                )
                continue
            print(f"  verified existing {name} v{version}  sha256={sha[:12]}…")
            continue

        entries.append(entry)
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
