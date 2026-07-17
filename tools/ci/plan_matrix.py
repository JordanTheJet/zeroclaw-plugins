#!/usr/bin/env python3
"""Plan changed-plugin or full component-validation shards."""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from registry_contract import PLUGIN_NAME_RE

DEFAULT_SHARD_SIZE = 8
DEFAULT_MAX_SHARDS = 4
FULL_SWEEP_PREFIXES = (".github/", "tools/", "wit/")
FULL_SWEEP_FILES = {"registry.json"}
DOC_ONLY_PREFIXES = ("docs/",)
DOC_ONLY_FILES = {"README.md"}


class PlanError(ValueError):
    pass


def repository_plugins(repository: Path) -> list[str]:
    plugins_dir = repository / "plugins"
    if not plugins_dir.is_dir():
        raise PlanError(f"plugins directory does not exist: {plugins_dir}")
    plugins = []
    for path in plugins_dir.iterdir():
        if path.is_symlink():
            raise PlanError(f"plugin directory must not be a symbolic link: {path.name!r}")
        if not path.is_dir():
            continue
        if not PLUGIN_NAME_RE.fullmatch(path.name):
            raise PlanError(f"invalid plugin directory name: {path.name!r}")
        plugins.append(path.name)
    return sorted(plugins)


def git_changed_paths(repository: Path, base: str) -> list[str]:
    try:
        merge_base = subprocess.run(
            ["git", "merge-base", base, "HEAD"],
            cwd=repository,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        if not merge_base:
            raise PlanError(f"git merge-base {base} HEAD returned no object ID")
        raw = subprocess.run(
            [
                "git",
                "diff",
                "--name-only",
                "--diff-filter=ACDMRTUXB",
                "-z",
                merge_base,
                "HEAD",
                "--",
            ],
            cwd=repository,
            check=True,
            capture_output=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        stderr = getattr(error, "stderr", b"")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", errors="replace")
        detail = str(stderr).strip()
        raise PlanError(f"cannot compute changes from {base}: {detail or error}") from error
    try:
        return [part.decode("utf-8") for part in raw.split(b"\0") if part]
    except UnicodeDecodeError as error:
        raise PlanError("changed paths must be valid UTF-8") from error


def requires_full_sweep(changed_paths: list[str]) -> bool:
    for raw_path in changed_paths:
        path = PurePosixPath(raw_path).as_posix().removeprefix("./")
        if path in FULL_SWEEP_FILES or path.startswith(FULL_SWEEP_PREFIXES):
            return True
        if path in DOC_ONLY_FILES or path.startswith(DOC_ONLY_PREFIXES):
            continue
        parts = PurePosixPath(path).parts
        if (
            len(parts) >= 2
            and parts[0] == "plugins"
            and PLUGIN_NAME_RE.fullmatch(parts[1])
        ):
            continue
        # Unknown root-level or future build configuration is safety-relevant
        # until it is deliberately classified as documentation-only.
        return True
    return False


def changed_plugins(changed_paths: list[str], available: list[str]) -> list[str]:
    available_set = set(available)
    selected = set()
    for raw_path in changed_paths:
        parts = PurePosixPath(raw_path).parts
        if len(parts) < 2 or parts[0] != "plugins":
            continue
        plugin = parts[1]
        if not PLUGIN_NAME_RE.fullmatch(plugin):
            raise PlanError(f"changed path has invalid plugin name: {raw_path!r}")
        if plugin in available_set:
            selected.add(plugin)
    return sorted(selected)


def shard_plugins(
    plugins: list[str], shard_size: int = DEFAULT_SHARD_SIZE, max_shards: int = DEFAULT_MAX_SHARDS
) -> list[list[str]]:
    if shard_size <= 0:
        raise PlanError("shard size must be positive")
    if max_shards <= 0:
        raise PlanError("maximum shard count must be positive")
    if not plugins:
        return []
    desired = math.ceil(len(plugins) / shard_size)
    shard_count = min(desired, max_shards)
    if desired <= max_shards:
        return [plugins[index : index + shard_size] for index in range(0, len(plugins), shard_size)]

    minimum_size, larger_shards = divmod(len(plugins), shard_count)
    shards = []
    offset = 0
    for shard_index in range(shard_count):
        size = minimum_size + (1 if shard_index < larger_shards else 0)
        shards.append(plugins[offset : offset + size])
        offset += size
    return shards


def make_plan(
    repository: Path,
    event: str,
    base: str,
    shard_size: int = DEFAULT_SHARD_SIZE,
    max_shards: int = DEFAULT_MAX_SHARDS,
) -> dict[str, object]:
    available = repository_plugins(repository)
    paths = []
    if event == "pull_request":
        paths = git_changed_paths(repository, base)
        if requires_full_sweep(paths):
            mode = "full"
            selected = available
        else:
            mode = "changed"
            selected = changed_plugins(paths, available)
    else:
        # Reusable workflows retain the caller's event name. A publish call
        # originates from push/dispatch and therefore intentionally lands here.
        mode = "full"
        selected = available
        if event == "push" and base and base != "0" * 40:
            paths = git_changed_paths(repository, base)

    strict = set(changed_plugins(paths, available))

    shards = shard_plugins(selected, shard_size=shard_size, max_shards=max_shards)
    matrix = {
        "include": [
            {
                "id": shard_id,
                "plugins": shard,
                "strict_plugins": [plugin for plugin in shard if plugin in strict],
            }
            for shard_id, shard in enumerate(shards)
        ]
    }
    return {"matrix": matrix, "mode": mode, "count": len(selected)}


def write_github_outputs(path: Path, plan: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(f"matrix={json.dumps(plan['matrix'], separators=(',', ':'))}\n")
        handle.write(f"mode={plan['mode']}\n")
        handle.write(f"count={plan['count']}\n")


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, default=Path.cwd())
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch"))
    parser.add_argument(
        "--base",
        default=os.environ.get("GITHUB_EVENT_BEFORE") or "origin/main",
    )
    parser.add_argument("--shard-size", type=int, default=DEFAULT_SHARD_SIZE)
    parser.add_argument("--max-shards", type=int, default=DEFAULT_MAX_SHARDS)
    parser.add_argument("--output", type=Path, default=None)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    output = args.output
    if output is None and os.environ.get("GITHUB_OUTPUT"):
        output = Path(os.environ["GITHUB_OUTPUT"])
    try:
        plan = make_plan(
            args.repository.resolve(),
            args.event,
            args.base,
            shard_size=args.shard_size,
            max_shards=args.max_shards,
        )
        if output is not None:
            write_github_outputs(output, plan)
    except PlanError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(plan, separators=(",", ":"), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
