#!/usr/bin/env bash

set -Eeuo pipefail

ROOT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
    pwd
)"

cd "${ROOT_DIR}"

fail() {
    echo "source-tree check failed: $*" >&2
    exit 1
}

tracked_build_artifacts="$(
    git ls-files | grep -E '(^|/)(build|target)/' || true
)"

if [[ -n "${tracked_build_artifacts}" ]]; then
    printf '%s\n' "${tracked_build_artifacts}" >&2
    fail "generated build artifacts are tracked by git"
fi

tracked_macos_metadata="$(
    git ls-files | grep -E '(^|/)(__MACOSX|\.DS_Store)(/|$)|(^|/)\._' || true
)"

if [[ -n "${tracked_macos_metadata}" ]]; then
    printf '%s\n' "${tracked_macos_metadata}" >&2
    fail "macOS metadata is tracked by git"
fi

echo "source-tree check: clean"
