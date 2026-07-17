import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
BUILD_REGISTRY = REPOSITORY_ROOT / "tools" / "build-registry.py"
RELEASE_BASE = "https://example.invalid/releases/download/plugins"


def write_plugin(
    root: Path,
    *,
    version: str = "0.1.0",
    wasm: bytes = b"wasm-v1",
    provides: str | None = "telegram",
    sender_match: str | None = "exact",
) -> None:
    plugin = root / "bridge"
    plugin.mkdir(parents=True)
    lines = [
        'name = "bridge"',
        f'version = "{version}"',
        'description = "Bridge messages"',
        'author = "ZeroClaw Labs"',
        'wasm_path = "bridge.wasm"',
        'capabilities = ["channel"]',
        'permissions = ["config_read"]',
    ]
    if provides is not None:
        lines.append(f'provides = "{provides}"')
    if sender_match is not None:
        lines.append(f'sender_match = "{sender_match}"')
    (plugin / "manifest.toml").write_text("\n".join(lines) + "\n")
    (plugin / "bridge.wasm").write_bytes(wasm)


def run_registry(*arguments: object) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(BUILD_REGISTRY), *(str(arg) for arg in arguments)],
        cwd=REPOSITORY_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )


class BuildRegistryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def build(self, staged: Path, out: Path, existing: Path | None = None):
        arguments = [
            "--staged",
            staged,
            "--release-base",
            RELEASE_BASE,
            "--out",
            out,
        ]
        if existing is not None:
            arguments.extend(["--existing-registry", existing])
        return run_registry(*arguments)

    def test_metadata_is_synchronized_from_canonical_manifest(self) -> None:
        source = self.root / "plugins"
        write_plugin(source)
        registry = self.root / "registry.json"
        registry.write_text(
            json.dumps(
                {
                    "plugins": [
                        {
                            "name": "bridge",
                            "version": "0.1.0",
                            "description": "stale",
                            "author": "ZeroClaw Labs",
                            "capabilities": ["channel"],
                            "url": f"{RELEASE_BASE}/bridge-0.1.0.zip",
                            "sha256": "0" * 64,
                        }
                    ]
                }
            )
            + "\n"
        )

        drift = run_registry(
            "--source-plugins", source, "--check-metadata", registry
        )
        self.assertNotEqual(drift.returncode, 0)
        self.assertIn("registry metadata drift", drift.stderr)

        synced = run_registry(
            "--source-plugins", source, "--sync-metadata", registry
        )
        self.assertEqual(synced.returncode, 0, synced.stderr)
        entry = json.loads(registry.read_text())["plugins"][0]
        self.assertEqual(entry["provides"], "telegram")
        self.assertEqual(entry["sender_match"], "exact")
        self.assertEqual(entry["description"], "Bridge messages")

        checked = run_registry(
            "--source-plugins", source, "--check-metadata", registry
        )
        self.assertEqual(checked.returncode, 0, checked.stderr)

    def test_existing_package_is_reused_but_never_overwritten(self) -> None:
        staged = self.root / "staged"
        write_plugin(staged)
        initial = self.root / "initial"
        first = self.build(staged, initial)
        self.assertEqual(first.returncode, 0, first.stderr)

        unchanged = self.root / "unchanged"
        same = self.build(staged, unchanged, initial / "registry.json")
        self.assertEqual(same.returncode, 0, same.stderr)
        self.assertFalse((unchanged / "bridge-0.1.0.zip").exists())
        self.assertEqual(
            json.loads((unchanged / "registry.json").read_text()),
            json.loads((initial / "registry.json").read_text()),
        )

        (staged / "bridge" / "bridge.wasm").write_bytes(b"changed bytes")
        collision = self.root / "collision"
        changed = self.build(staged, collision, initial / "registry.json")
        self.assertNotEqual(changed.returncode, 0)
        self.assertIn("refusing to overwrite immutable package bridge@0.1.0", changed.stderr)
        self.assertIn("bump the manifest version", changed.stderr)
        self.assertFalse((collision / "bridge-0.1.0.zip").exists())

    def test_source_manifest_version_is_canonical_for_cargo(self) -> None:
        source = self.root / "plugins"
        write_plugin(source, version="0.2.0")
        (source / "bridge" / "Cargo.toml").write_text(
            '[package]\nname = "bridge"\nversion = "0.1.0"\n'
        )
        registry = self.root / "registry.json"
        registry.write_text('{"plugins": []}\n')

        checked = run_registry(
            "--source-plugins", source, "--check-metadata", registry
        )
        self.assertNotEqual(checked.returncode, 0)
        self.assertIn("does not match canonical manifest version", checked.stderr)

    def test_new_version_is_appended_to_registry_history(self) -> None:
        staged = self.root / "staged"
        write_plugin(staged)
        initial = self.root / "initial"
        first = self.build(staged, initial)
        self.assertEqual(first.returncode, 0, first.stderr)

        staged_v2 = self.root / "staged-v2"
        write_plugin(staged_v2, version="0.2.0", wasm=b"wasm-v2")
        metadata_check = run_registry(
            "--source-plugins",
            staged_v2,
            "--check-metadata",
            initial / "registry.json",
        )
        self.assertEqual(metadata_check.returncode, 0, metadata_check.stderr)
        self.assertIn("pending unpublished source: bridge@0.2.0", metadata_check.stdout)

        updated = self.root / "updated"
        second = self.build(staged_v2, updated, initial / "registry.json")
        self.assertEqual(second.returncode, 0, second.stderr)
        entries = json.loads((updated / "registry.json").read_text())["plugins"]
        self.assertEqual(
            [(entry["name"], entry["version"]) for entry in entries],
            [("bridge", "0.1.0"), ("bridge", "0.2.0")],
        )
        self.assertTrue((updated / "bridge-0.2.0.zip").is_file())


if __name__ == "__main__":
    unittest.main()
