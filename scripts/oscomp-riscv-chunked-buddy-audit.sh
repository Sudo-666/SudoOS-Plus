#!/usr/bin/env bash
set -euo pipefail
fail=0
pass=0
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
check "chunked handoff helper" "release_early_ranges_to_buddy_chunked" kernel/src/memory.rs
check "release chunk begin marker" "P8R:release-chunk-begin" kernel/src/memory.rs
check "release chunk done marker" "P8S:release-chunk-done" kernel/src/memory.rs
check "monolithic release loop replaced" "release_early_ranges_to_buddy_chunked(&mut page_allocator, &early_allocator)" kernel/src/memory.rs
check "MAX_ORDER chunk constant exported" "MAX_ORDER_NR_PAGES" mm/src/buddy/zone.rs
check "MAX_ORDER chunk visible from myos_mm" "MAX_ORDER_NR_PAGES" mm/src/lib.rs
if grep -q "for range in early_allocator.free_ranges().*release_range(range)" kernel/src/memory.rs; then
  echo "FAIL: old monolithic release loop still present"
  fail=$((fail+1))
else
  echo "PASS: old monolithic release loop absent"
  pass=$((pass+1))
fi
echo "oscomp-riscv-chunked-buddy-audit: PASS=$pass FAIL=$fail"
exit "$fail"
