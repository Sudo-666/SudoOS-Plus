#!/usr/bin/env bash
set -euo pipefail

fail=0
skip_re='(^|/)(target|build|vendor/cargo|\.git|\.oscomp_patch_backup)(/|$)'

files=$(find . -name '*.rs' -type f | grep -Ev "$skip_re" || true)

if echo "$files" | xargs grep -n '\.is_multiple_of(' >/tmp/oscomp-is-multiple.$$ 2>/dev/null; then
  if ! echo "$files" | xargs grep -n '^#!\[feature(unsigned_is_multiple_of)\]' >/dev/null 2>&1; then
    echo "[oscomp-rust2025-global-audit] WARN: is_multiple_of remains; ensure every containing crate root has #![feature(unsigned_is_multiple_of)]"
  else
    echo "[oscomp-rust2025-global-audit] PASS: unsigned_is_multiple_of feature gate present"
  fi
else
  echo "[oscomp-rust2025-global-audit] PASS: no .is_multiple_of calls remain"
fi
rm -f /tmp/oscomp-is-multiple.$$

if echo "$files" | xargs grep -nE '(&&|\|\|)[[:space:]]+let[[:space:]]+' >/tmp/oscomp-letchains.$$ 2>/dev/null; then
  if ! echo "$files" | xargs grep -n '^#!\[feature(let_chains)\]' >/dev/null 2>&1; then
    echo "[oscomp-rust2025-global-audit] FAIL: let-chains remain but no feature gate found"
    cat /tmp/oscomp-letchains.$$
    fail=1
  else
    echo "[oscomp-rust2025-global-audit] PASS: let_chains feature gate present"
  fi
else
  echo "[oscomp-rust2025-global-audit] PASS: no let-chains syntax found"
fi
rm -f /tmp/oscomp-letchains.$$

exit "$fail"
