#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FAIL=0
check() {
  local path="$1"
  if [[ -f "$ROOT/$path" ]]; then
    echo "[oscomp-rust-src-repair-audit] PASS: $path"
  else
    echo "[oscomp-rust-src-repair-audit] FAIL: missing $path"
    FAIL=1
  fi
}
check vendor/rust-src/library/Cargo.toml
check vendor/rust-src/library/Cargo.lock
check vendor/rust-src/library/core/Cargo.toml
check vendor/rust-src/library/alloc/Cargo.toml
check vendor/rust-src/library/std/Cargo.toml
check scripts/oscomp-prepare-rust-src.sh
if grep -R "oscomp-prepare-rust-src.sh" "$ROOT/scripts" "$ROOT/Makefile" "$ROOT/Makefile.project" >/dev/null 2>&1; then
  echo "[oscomp-rust-src-repair-audit] PASS: build path references oscomp-prepare-rust-src.sh"
else
  echo "[oscomp-rust-src-repair-audit] WARN: did not find prepare script reference in common build files"
fi
exit "$FAIL"
