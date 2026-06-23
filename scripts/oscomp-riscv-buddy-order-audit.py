#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[1]
alloc = root / "mm/src/buddy/allocator.rs"
text = alloc.read_text()

m = re.search(r"fn\s+largest_block_order\s*\([^)]*\)\s*->\s*usize\s*\{(?P<body>.*?)\n\}", text, re.S)
if not m:
    print("FAIL: largest_block_order() missing")
    sys.exit(1)
body = m.group("body")
checks = [
    ("no trailing_zeros in early buddy order", "trailing_zeros" not in body),
    ("no leading_zeros in early buddy order", "leading_zeros" not in body),
    ("bounded by MAX_ORDER loop", "while order + 1 < MAX_ORDER" in body),
    ("bounded by remaining_pages", "block_pages > remaining_pages" in body),
    ("alignment mask check", "pfn & (block_pages - 1)" in body),
    ("zero remaining defensive guard", "remaining_pages == 0" in body),
]
fail = 0
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
    if not ok:
        fail += 1
if fail:
    print(f"oscomp-riscv-buddy-order-audit: FAIL={fail}")
    sys.exit(1)
print(f"oscomp-riscv-buddy-order-audit: PASS={len(checks)} FAIL=0")
