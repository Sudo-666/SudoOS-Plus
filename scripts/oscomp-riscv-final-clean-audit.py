#!/usr/bin/env python3
from pathlib import Path
root = Path(__file__).resolve().parents[1]
checks = []

def add(name, ok, detail=""):
    checks.append((name, bool(ok), detail))

rust_text = "\n".join(p.read_text(encoding="utf-8", errors="ignore") for p in [root/"kernel/src/main.rs", root/"kernel/src/memory.rs", root/"mm/src/buddy/allocator.rs"] if p.exists())
forbidden = [
    "oscomp_riscv_raw_trace", "oscomp_riscv_page_alloc_trace", "oscomp_riscv_chunked_buddy_trace",
    "OSCOMP_RISCV_POST_FINAL_TRACE", "P0:", "P1:", "P2:", "P3:", "P4:", "P5:", "P6:", "P7:", "P8:", "P8R:", "P8S:", "P9:", "P10:", "P11:",
    "B0:", "B1:", "B2:", "B3:", "B4:", "B5:", "B6:", "B7:", "B8:", "retained-lowmap", "contest boot-stack safety",
]
add("no runtime trace or retained-lowmap residue", not any(s in rust_text for s in forbidden))
mem = (root/"kernel/src/memory.rs").read_text(encoding="utf-8")
add("final page table removes low boot mapping", "low boot mapping: removed" in mem and "final page table still maps the low boot image" in mem)
add("chunked early buddy handoff present", "release_early_ranges_to_buddy_chunked(&mut page_allocator, &early_allocator);" in mem)
add("no direct full-range early release loop", "for range in early_allocator.free_ranges()" not in mem.split("release_early_ranges_to_buddy_chunked", 1)[0])
pass_count = sum(ok for _, ok, _ in checks)
fail_count = len(checks) - pass_count
for name, ok, detail in checks:
    print(("PASS" if ok else "FAIL") + ": " + name + ((" — " + detail) if detail else ""))
print(f"oscomp-riscv-final-clean-audit: PASS={pass_count} FAIL={fail_count}")
raise SystemExit(0 if fail_count == 0 else 1)
