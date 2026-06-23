#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
text = (root / "kernel/src/memory.rs").read_text()
checks = []

def check(name, cond):
    checks.append((name, bool(cond)))

fn = text[text.find("pub fn initialize_page_allocator("):]
start = fn.find('#[cfg(target_arch = "riscv64")]')
end = fn.find('#[cfg(not(target_arch = "riscv64"))]', start)
riscv = fn[start:end] if start >= 0 and end >= 0 else ""
check("riscv allocator cfg block present", bool(riscv))
check("riscv uses boot allocator install", "install_boot(page_allocator)" in riscv)
check("riscv avoids runtime allocator install", "page_alloc::install(page_allocator)" not in riscv)
check("riscv removes boot post-install reread", "is_initialized_boot" not in riscv)
check("riscv prints total free before publish", riscv.find("total free") >= 0 and riscv.find("total free") < riscv.find("install_boot(page_allocator)"))
check("riscv prints early handoff after publish", riscv.find("install_boot(page_allocator)") >= 0 and riscv.find("early handoff") > riscv.find("install_boot(page_allocator)"))
check("temporary install_boot trace removed from memory", "riscv page_alloc install_boot:" not in text)
check("temporary install_boot trace removed from page_alloc", "riscv page_alloc install_boot:" not in (root / "kernel/src/page_alloc.rs").read_text())
check("verbose zone summary remains non-riscv only", '#[cfg(not(target_arch = "riscv64"))]' in fn and "zone_present_pages" in fn[fn.find('#[cfg(not(target_arch = "riscv64"))'):])

passed = sum(v for _, v in checks)
failed = len(checks) - passed
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
print(f"oscomp-riscv-post-install-probe-cleanup-audit: PASS={passed} FAIL={failed}")
sys.exit(0 if failed == 0 else 1)
