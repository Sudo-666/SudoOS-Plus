#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parents[1]
mem = (root / "kernel/src/memory.rs").read_text()

def ok(name, cond):
    print(("PASS" if cond else "FAIL") + f" {name}")
    return bool(cond)

checks = []
checks.append(ok("RISC-V final clean has no runtime trace strings", all(s not in mem for s in [
    "OSCOMP_RISCV_POST_FINAL_TRACE",
    "P0:enter-page-allocator",
    "P8R:release-chunk-begin",
    "B0:release-enter",
    "oscomp_riscv_raw_trace",
    "oscomp_riscv_page_alloc_trace",
    "release_range_with_trace",
])))
checks.append(ok("chunked buddy handoff helper exists", "release_early_ranges_to_buddy_chunked" in mem and "release_early_range_to_buddy_chunked" in mem))
checks.append(ok("RISC-V summary is gated before verbose zone prints", '#[cfg(target_arch = "riscv64")]' in mem and '#[cfg(not(target_arch = "riscv64"))]' in mem))
checks.append(ok("RISC-V preinstall summary avoids DMA32 zone access", re.search(r'#\[cfg\(target_arch = "riscv64"\)\]\s*\{(?:(?!zone_present_pages|zone_free_pages).)*early handoff: complete', mem, re.S) is not None))
checks.append(ok("non-RISC-V keeps verbose zone summary", re.search(r'#\[cfg\(not\(target_arch = "riscv64"\)\)\]\s*\{(?:(?!\n\s*\}\s*crate::page_alloc::install).)*zone_present_pages\(ZoneKind::Dma32', mem, re.S) is not None))
checks.append(ok("allocator install still follows summary", "crate::page_alloc::install(page_allocator)" in mem))

if not all(checks):
    sys.exit(1)
