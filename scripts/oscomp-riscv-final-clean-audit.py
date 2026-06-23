#!/usr/bin/env python3
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
checks = []

def add(name, ok, detail=""):
    checks.append((name, bool(ok), detail))

sources = [
    ROOT / "kernel/src/main.rs",
    ROOT / "kernel/src/memory.rs",
    ROOT / "mm/src/buddy/allocator.rs",
]
joined = "\n".join(p.read_text(encoding="utf-8") for p in sources if p.exists())
for token in [
    "oscomp_riscv_raw_trace",
    "oscomp_riscv_page_alloc_trace",
    "oscomp_riscv_chunked_buddy_trace",
    "release_range_with_trace",
    "OSCOMP_RISCV_POST_FINAL_TRACE",
    "P0:", "P1:", "P2:", "P3:", "P4:", "P5:", "P6:", "P7:", "P8:", "P8R:", "P8S:", "P9:", "P10:", "P11:",
    "B0:", "B1:", "B2:", "B3:", "B4:", "B5:", "B6:", "B7:", "B8:",
]:
    add(f"no runtime trace token {token}", token not in joined)
add("final low mapping remains removed", "low boot mapping: removed" in joined)
add("chunked handoff helper remains", "release_early_ranges_to_buddy_chunked" in joined)

fail = 0
for name, ok, detail in checks:
    print(("PASS" if ok else "FAIL") + f": {name}" + (f" -- {detail}" if detail else ""))
    fail += 0 if ok else 1
print(f"oscomp-riscv-final-clean-audit: PASS={len(checks)-fail} FAIL={fail}")
raise SystemExit(1 if fail else 0)
