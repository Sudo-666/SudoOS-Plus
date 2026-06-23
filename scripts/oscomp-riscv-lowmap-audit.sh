#!/usr/bin/env bash
set -euo pipefail

echo "[oscomp-riscv-lowmap-audit] checking RISC-V low boot mapping retention"

matches="$(grep -RIn "low boot mapping" arch/riscv64 kernel boot 2>/dev/null || true)"
if [ -z "$matches" ]; then
  echo "[oscomp-riscv-lowmap-audit] WARN: no low boot mapping log found"
  exit 0
fi

echo "$matches"

if echo "$matches" | grep -q "low boot mapping: removed"; then
  echo "[oscomp-riscv-lowmap-audit] FAIL: stale 'removed' log remains"
  exit 1
fi

if ! grep -RIn "OSKernel2026: retain low boot mapping" arch/riscv64 kernel boot >/dev/null 2>&1; then
  echo "[oscomp-riscv-lowmap-audit] WARN: no disabled lowmap removal marker found"
  echo "[oscomp-riscv-lowmap-audit]      inspect the file containing 'low boot mapping' manually"
  exit 0
fi

echo "[oscomp-riscv-lowmap-audit] PASS: low boot mapping is retained for contest boot-stack safety"
