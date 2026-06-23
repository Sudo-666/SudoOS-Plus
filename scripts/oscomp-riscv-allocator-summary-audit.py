#!/usr/bin/env python3
from pathlib import Path
import sys
root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
mem = (root / "kernel/src/memory.rs").read_text()
checks = [
    ("riscv bounded pre-install allocator summary", '#[cfg(target_arch = "riscv64")]' in mem and 'expected_free_pages' in mem),
    ("non-riscv zone summary preserved", '#[cfg(not(target_arch = "riscv64"))]' in mem and 'zone_present_pages(ZoneKind::Dma32' in mem),
    ("allocator install remains after summary", 'crate::page_alloc::install(page_allocator).unwrap_or_else' in mem),
    ("no runtime OSCOMP trace strings", 'OSCOMP_' not in mem and 'P8R:' not in mem and 'B0:release-enter' not in mem),
]
fail = 0
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + ": " + name)
    fail += 0 if ok else 1
print(f"oscomp-riscv-allocator-summary-audit: PASS={len(checks)-fail} FAIL={fail}")
sys.exit(1 if fail else 0)
