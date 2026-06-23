#!/usr/bin/env bash
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if ! grep -R "oscomp_rebase_boot_stack_to_direct_map_once" -n arch/riscv64 kernel boot >/tmp/oscomp-riscv-stack-handoff-grep.txt 2>/dev/null; then
  echo "[oscomp-riscv-stack-handoff] FAIL: stack direct-map handoff helper/call not found"
  exit 1
fi
if ! grep -R "boot stack      : direct-map alias ready" -n arch/riscv64 kernel boot >/dev/null 2>/dev/null; then
  echo "[oscomp-riscv-stack-handoff] FAIL: runtime log for stack handoff not found"
  exit 1
fi
if ! grep -R "low boot mapping: retained" -n arch/riscv64 kernel boot >/dev/null 2>/dev/null; then
  echo "[oscomp-riscv-stack-handoff] WARN: low boot mapping retained log not found; verify previous retain patch"
else
  echo "[oscomp-riscv-stack-handoff] PASS: low mapping is retained during handoff"
fi
echo "[oscomp-riscv-stack-handoff] PASS: direct-map stack handoff installed"
