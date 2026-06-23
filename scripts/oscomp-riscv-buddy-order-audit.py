#!/usr/bin/env python3
from pathlib import Path
import re
ROOT = Path(__file__).resolve().parents[1]
text = (ROOT / "mm/src/buddy/allocator.rs").read_text(encoding="utf-8")
m = re.search(r"fn\s+largest_block_order\s*\([^)]*\)\s*->\s*usize\s*\{(?P<body>.*?)\n\}", text, re.S)
body = m.group("body") if m else ""
checks = []
def add(name, ok): checks.append((name, bool(ok)))
add("largest_block_order exists", bool(m))
add("zero guard", "remaining_pages == 0" in body and "return 0" in body)
add("MAX_ORDER bounded loop", "while order + 1 < MAX_ORDER" in body)
add("remaining_pages limit", "block_pages > remaining_pages" in body)
add("alignment mask", "pfn & (block_pages - 1)" in body or "pfn % block_pages" in body)
add("no trailing_zeros", "trailing_zeros" not in body)
add("no leading_zeros", "leading_zeros" not in body)
fail = 0
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
    fail += 0 if ok else 1
print(f"oscomp-riscv-buddy-order-audit: PASS={len(checks)-fail} FAIL={fail}")
raise SystemExit(1 if fail else 0)
