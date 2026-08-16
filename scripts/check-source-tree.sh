#!/usr/bin/env bash
set -Eeuo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

fail() {
    echo "source-tree check failed: $*" >&2
    exit 1
}

# The vendored third-party tree (vendor/cargo, vendor/u-boot-*) legitimately
# contains source paths with /build/ or /target/ segments (e.g. the `object`
# crate's src/build/, the `cc` crate's src/target/). Exclude it: only the
# repo's own generated build/target artifacts should trip this gate.
tracked_build_artifacts="$(git ls-files | while IFS= read -r path; do [[ -e "$path" ]] && printf '%s\n' "$path"; done | grep -E '(^|/)(build|target)/' | grep -v '^vendor/' || true)"
if [[ -n "${tracked_build_artifacts}" ]]; then
    printf '%s\n' "${tracked_build_artifacts}" >&2
    fail "generated build artifacts are tracked by git"
fi

tracked_macos_metadata="$(git ls-files | while IFS= read -r path; do [[ -e "$path" ]] && printf '%s\n' "$path"; done | grep -E '(^|/)(__MACOSX|\.DS_Store)(/|$)|(^|/)\._' || true)"
if [[ -n "${tracked_macos_metadata}" ]]; then
    printf '%s\n' "${tracked_macos_metadata}" >&2
    fail "macOS metadata is tracked by git"
fi

tracked_python_cache="$(git ls-files | while IFS= read -r path; do [[ -e "$path" ]] && printf '%s\n' "$path"; done | grep -E '(^|/)__pycache__/|\.(pyc|pyo)$|(^|/)\.pytest_cache/' || true)"
if [[ -n "${tracked_python_cache}" ]]; then
    printf '%s\n' "${tracked_python_cache}" >&2
    fail "generated Python cache files are tracked by git"
fi

tracked_editor_backups="$(git ls-files | while IFS= read -r path; do [[ -e "$path" ]] && printf '%s\n' "$path"; done | grep -E '(~|\.swp|\.swo)$' || true)"
if [[ -n "${tracked_editor_backups}" ]]; then
    printf '%s\n' "${tracked_editor_backups}" >&2
    fail "editor backup files are tracked by git"
fi

echo "source-tree check: clean"
