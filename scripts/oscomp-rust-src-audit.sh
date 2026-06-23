#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/vendor/rust-src/library"

fail=0
check() {
    if [ -e "$1" ]; then
        echo "[oscomp-rust-src-audit] PASS: ${1#$ROOT/}"
    else
        echo "[oscomp-rust-src-audit] FAIL: missing ${1#$ROOT/}" >&2
        fail=1
    fi
}

check "$SRC/Cargo.toml"
check "$SRC/Cargo.lock"
check "$SRC/core/Cargo.toml"
check "$SRC/alloc/Cargo.toml"

if grep -R --include='*.sh' -n 'oscomp-prepare-rust-src.sh' "$ROOT/scripts" "$ROOT/Makefile" "$ROOT/Makefile.project" >/dev/null 2>&1; then
    echo "[oscomp-rust-src-audit] PASS: build path invokes oscomp-prepare-rust-src.sh"
else
    echo "[oscomp-rust-src-audit] FAIL: build path does not invoke oscomp-prepare-rust-src.sh" >&2
    fail=1
fi

exit "$fail"
