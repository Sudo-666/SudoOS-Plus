#!/usr/bin/env bash
set -eu
root="${1:-.}"
mem="$root/kernel/src/memory.rs"
fail=0
pass=0

check_present() {
  pattern="$1"; msg="$2"
  if grep -q -- "$pattern" "$mem"; then
    echo "[riscv-linuxlike] PASS: $msg"; pass=$((pass+1))
  else
    echo "[riscv-linuxlike] FAIL: $msg"; fail=$((fail+1))
  fi
}

check_absent() {
  pattern="$1"; msg="$2"
  if grep -q -- "$pattern" "$mem"; then
    echo "[riscv-linuxlike] FAIL: $msg"; fail=$((fail+1))
  else
    echo "[riscv-linuxlike] PASS: $msg"; pass=$((pass+1))
  fi
}

check_present 'rebase_riscv_boot_stack_to_kernel_alias();' 'final-root install rebases low boot stack immediately after runtime UART mapping'
check_present 'fn rebase_riscv_boot_stack_to_kernel_alias' 'Linux-like RISC-V stack alias handoff helper exists'
check_present 'phys_to_direct(physical)' 'stack handoff uses layout::phys_to_direct, not a hard-coded direct-map base'
check_present 'low boot mapping: removed' 'final root reports low boot mapping removed'
check_present 'final page table still maps the low boot image' 'final root keeps strict low-image unmapped invariant'
check_absent 'contest boot-stack safety' 'no retained-low-map contest workaround remains'
check_absent 'oscomp_riscv_switch_stack_to' 'old return-through stack switcher removed'
check_absent 'oscomp_rebase_boot_stack_to_direct_map_once' 'old hard-coded direct-map helper removed'

if [ "$fail" -ne 0 ]; then
  echo "[riscv-linuxlike] SUMMARY: PASS=$pass FAIL=$fail"
  exit 1
fi

echo "[riscv-linuxlike] SUMMARY: PASS=$pass FAIL=0"
