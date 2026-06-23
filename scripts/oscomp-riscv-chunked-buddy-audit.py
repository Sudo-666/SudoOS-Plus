#!/usr/bin/env python3
from pathlib import Path
root = Path(__file__).resolve().parents[1]
mem = (root/"kernel/src/memory.rs").read_text(encoding="utf-8")
checks = []
def add(name, ok): checks.append((name, bool(ok)))
add("MAX_ORDER chunk size constant", "MAX_ORDER_NR_PAGES" in mem and "myos_mm::MAX_ORDER" in mem)
add("chunked ranges helper exists", "fn release_early_ranges_to_buddy_chunked" in mem)
add("chunked range helper exists", "fn release_early_range_to_buddy_chunked" in mem)
add("allocator uses chunked handoff", "release_early_ranges_to_buddy_chunked(&mut page_allocator, &early_allocator);" in mem)
add("runtime chunk trace absent", "P8R:" not in mem and "P8S:" not in mem and "oscomp_riscv_chunked_buddy_trace" not in mem)
add("direct release only inside chunk helper", mem.count("page_allocator.release_range") == 1)
pass_count = sum(ok for _, ok in checks)
fail_count = len(checks) - pass_count
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + ": " + name)
print(f"oscomp-riscv-chunked-buddy-audit: PASS={pass_count} FAIL={fail_count}")
raise SystemExit(0 if fail_count == 0 else 1)
