#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-.}"
LIB="$ROOT/vendor/virtio-drivers/src/lib.rs"
CONSOLE="$ROOT/vendor/virtio-drivers/src/device/console.rs"

if [ ! -f "$LIB" ]; then
  echo "[oscomp-virtio-letchains-audit] FAIL: missing $LIB" >&2
  exit 1
fi

if [ ! -f "$CONSOLE" ]; then
  echo "[oscomp-virtio-letchains-audit] WARN: missing $CONSOLE"
  exit 0
fi

if grep -Eq '(^|[^[:alnum:]_])if[[:space:]]+let[\s\S]*(&&|\|\|)|&&[[:space:]]+let|\|\|[[:space:]]+let' "$CONSOLE" 2>/dev/null; then
  if ! grep -q '#!\[feature(let_chains)\]' "$LIB"; then
    echo "[oscomp-virtio-letchains-audit] FAIL: virtio-drivers uses let-chains but lib.rs lacks #![feature(let_chains)]" >&2
    exit 1
  fi
fi

echo "[oscomp-virtio-letchains-audit] PASS: virtio-drivers let_chains gate is present"
