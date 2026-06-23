#!/bin/sh
set -eu
fail=0
check() {
  desc="$1"; shift
  if "$@"; then
    echo "PASS: $desc"
  else
    echo "FAIL: $desc"
    fail=$((fail+1))
  fi
}
check "R1: post-final marker" grep -q "OSCOMP_RISCV_POST_FINAL_TRACE_R1" kernel/src/main.rs
check "R2: before allocator marker" grep -q "OSCOMP_RISCV_POST_FINAL_TRACE_R2" kernel/src/main.rs
check "R3: allocator returned marker" grep -q "OSCOMP_RISCV_POST_FINAL_TRACE_R3" kernel/src/main.rs
check "no unsafe // trace comments" sh -c '! grep -q "// OSCOMP_RISCV_POST_FINAL_TRACE" kernel/src/main.rs'
check "trace helper installed" grep -q "fn oscomp_riscv_raw_trace" kernel/src/main.rs
check "P0: page allocator marker" grep -q "P0:enter-page-allocator" kernel/src/memory.rs
check "raw UART trace" grep -q "early_console::write_byte" kernel/src/main.rs
check "page allocator raw UART trace" grep -q "early_console::write_byte" kernel/src/memory.rs
if [ "$fail" -eq 0 ]; then
  echo "oscomp-riscv-post-final-trace-audit: PASS"
else
  echo "oscomp-riscv-post-final-trace-audit: FAIL=$fail"
  exit 1
fi
