#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
repository_root=$(cd -- "$script_dir/../.." && pwd -P)
image_file="$script_dir/packager-image.txt"

mapfile -t image_lines < <(
  sed -E 's/[[:space:]]*#.*$//' "$image_file" \
    | sed -E '/^[[:space:]]*$/d; s/^[[:space:]]+//; s/[[:space:]]+$//'
)
if [[ "${#image_lines[@]}" -ne 1 \
  || ! "${image_lines[0]}" =~ ^docker\.io/library/python@sha256:[0-9a-f]{64}$ ]]; then
  echo "error: $image_file must contain exactly one immutable official Python image digest" >&2
  exit 1
fi

docker_args=(
  run
  --platform linux/amd64
  --rm
  --network none
  --read-only
  --cap-drop ALL
  --security-opt no-new-privileges
  --user "$(id -u):$(id -g)"
  --env HOME=/tmp
  --env PYTHONDONTWRITEBYTECODE=1
  --tmpfs '/tmp:rw,noexec,nosuid,nodev'
)
if [[ -n "${RUNNER_TEMP:-}" ]]; then
  if [[ ! -d "$RUNNER_TEMP" ]]; then
    echo "error: RUNNER_TEMP is not a directory: $RUNNER_TEMP" >&2
    exit 1
  fi
  docker_args+=(--volume "$RUNNER_TEMP:$RUNNER_TEMP")
fi
docker_args+=(
  --volume "$repository_root:/workspace"
  --workdir /workspace
  "${image_lines[0]}"
  python3
)

exec docker "${docker_args[@]}" "$@"
