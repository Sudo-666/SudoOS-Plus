#!/usr/bin/env bash
set -euo pipefail
pass=0
fail=0
check() {
  local name="$1" pattern="$2" file="$3"
  if grep -q -- "$pattern" "$file"; then
    echo "PASS: $name"
    pass=$((pass+1))
  else
    echo "FAIL: $name"
    fail=$((fail+1))
  fi
}
check "BuddyAllocator release_range_with_trace installed" "release_range_with_trace" mm/src/buddy/allocator.rs
check "release enter marker" "B0:release-enter" mm/src/buddy/allocator.rs
check "range ok marker" "B1:range-ok" mm/src/buddy/allocator.rs
check "reserved ok marker" "B2:reserved-ok" mm/src/buddy/allocator.rs
check "block begin marker" "B3:block-begin" mm/src/buddy/allocator.rs
check "block free ok marker" "B6:block-free-ok" mm/src/buddy/allocator.rs
check "RISC-V uses traced release" "release_range_with_trace(chunk, oscomp_riscv_chunked_buddy_trace)" kernel/src/memory.rs
check "non-RISC-V keeps normal release" "page_allocator.release_range(chunk)" kernel/src/memory.rs
if grep -q '// OSCOMP' kernel/src/main.rs kernel/src/memory.rs mm/src/buddy/allocator.rs 2>/dev/null; then
  echo "FAIL: unsafe line-comment OSCOMP marker remains in Rust source"
  fail=$((fail+1))
else
  echo "PASS: no unsafe // OSCOMP marker in Rust source"
  pass=$((pass+1))
fi
echo "oscomp-buddy-release-internal-trace-audit: PASS=$pass FAIL=$fail"
exit "$fail"
