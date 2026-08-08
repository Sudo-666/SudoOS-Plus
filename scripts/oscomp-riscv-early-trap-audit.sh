#!/usr/bin/env bash
set -euo pipefail
fail=0

check() {
  if eval "$1"; then
    printf '[oscomp-riscv-early-trap] PASS: %s\n' "$2"
  else
    printf '[oscomp-riscv-early-trap] FAIL: %s\n' "$2"
    fail=1
  fi
}

check "grep -q 'call __riscv_early_trap_panic' arch/riscv64/src/platform/qemu_virt/entry.S" ".Lhigh_fault reports through Rust early trap reporter"
check "grep -q '__riscv_early_trap_panic' kernel/src/main.rs" "early trap reporter symbol exists"
check "grep -q 'csrr {old}, stvec' arch/riscv64/src/memory/paging/activate.rs" "switch_sv39_root saves caller stvec"
check "grep -q 'csrw stvec, {old}' arch/riscv64/src/memory/paging/activate.rs" "switch_sv39_root restores caller stvec"
check "grep -q 'low boot mapping: removed' kernel/src/memory.rs" "final root still removes low boot mapping"
check "! grep -R 'contest boot-stack safety\|retained (contest' -n kernel/src arch/riscv64/src >/tmp/oscomp-riscv-early-trap-retained.$$" "no retained-lowmap contest downgrade text remains"
rm -f /tmp/oscomp-riscv-early-trap-retained.$$

exit "$fail"
