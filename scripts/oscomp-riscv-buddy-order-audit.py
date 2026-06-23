#!/usr/bin/env python3
from pathlib import Path
import re
root = Path(__file__).resolve().parents[1]
text = (root/"mm/src/buddy/allocator.rs").read_text(encoding="utf-8")
m = re.search(r"fn\s+largest_block_order\s*\([^)]*\)\s*->\s*usize\s*\{(?P<body>.*?)\n\}", text, re.S)
body = m.group("body") if m else ""
checks = []
def add(name, ok): checks.append((name, bool(ok)))
add("largest_block_order exists", bool(m))
add("zero guard present", "remaining_pages == 0" in body)
add("MAX_ORDER bounded loop present", "MAX_ORDER" in body and "while" in body)
add("remaining_pages limit present", "remaining_pages < block_pages" in body)
add("alignment mask present", "alignment_mask" in body and "pfn & alignment_mask" in body)
add("bit-scan helpers absent", "trailing_zeros" not in body and "leading_zeros" not in body)
pass_count = sum(ok for _, ok in checks)
fail_count = len(checks) - pass_count
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + ": " + name)
print(f"oscomp-riscv-buddy-order-audit: PASS={pass_count} FAIL={fail_count}")
raise SystemExit(0 if fail_count == 0 else 1)
