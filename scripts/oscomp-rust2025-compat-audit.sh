#!/usr/bin/env bash
set -euo pipefail

fail=0

if grep -R --line-number --include='*.rs' '\.is_multiple_of(' mm vendor/fdt-reader 2>/dev/null; then
  echo '[oscomp-rust2025-compat-audit] FAIL: remaining .is_multiple_of(...) under mm/ or vendor/fdt-reader/' >&2
  fail=1
else
  echo '[oscomp-rust2025-compat-audit] PASS: no gated .is_multiple_of(...) remains in checked crates'
fi

if grep -q '#!\[feature(let_chains)\]' vfs/src/lib.rs; then
  echo '[oscomp-rust2025-compat-audit] PASS: vfs enables let_chains for contest nightly'
else
  if grep -R --line-number '&&[[:space:]]*let[[:space:]]' vfs/src/lib.rs >/dev/null 2>&1; then
    echo '[oscomp-rust2025-compat-audit] FAIL: vfs still uses let-chains without feature gate' >&2
    fail=1
  else
    echo '[oscomp-rust2025-compat-audit] PASS: no vfs let-chain gate needed'
  fi
fi

exit "$fail"
