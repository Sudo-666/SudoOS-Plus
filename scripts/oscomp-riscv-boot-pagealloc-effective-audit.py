#!/usr/bin/env python3
from pathlib import Path

root = Path(__file__).resolve().parents[1]
mem = (root / "kernel/src/memory.rs").read_text()
pa = (root / "kernel/src/page_alloc.rs").read_text()
irq = (root / "kernel/src/irq_lock.rs").read_text()
spin = (root / "sync/src/spin_lock.rs").read_text()

checks = []

def check(name, ok):
    checks.append((name, bool(ok)))

header = 'crate::println!("physical page allocator:");'
idx = mem.find(header)
check("allocator summary header present", idx >= 0)
if idx >= 0:
    cfg = mem.find('#[cfg(target_arch = "riscv64")]', idx)
    non = mem.find('#[cfg(not(target_arch = "riscv64"))]', idx)
    check("riscv allocator cfg block present", cfg >= 0)
    check("non-riscv allocator cfg block present", non >= 0)
    if cfg >= 0 and non >= 0:
        riscv_block = mem[cfg:non]
        check("riscv uses install_boot", "install_boot(page_allocator)" in riscv_block)
        check("riscv boot init check", "is_initialized_boot" in riscv_block)
        check("riscv block excludes zone_present_pages", "zone_present_pages" not in riscv_block)
        check("riscv block excludes zone_free_pages", "zone_free_pages" not in riscv_block)

check("page_alloc exposes install_boot", "pub unsafe fn install_boot" in pa)
check("page_alloc exposes is_initialized_boot", "pub unsafe fn is_initialized_boot" in pa)
check("irq lock exposes boot get_mut_unchecked", "pub unsafe fn get_mut_unchecked" in irq)
check("spin lock exposes boot get_mut_unchecked", "pub unsafe fn get_mut_unchecked" in spin)

passed = sum(ok for _, ok in checks)
failed = len(checks) - passed
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
print(f"oscomp-riscv-boot-pagealloc-effective-audit: PASS={passed}, FAIL={failed}")
raise SystemExit(0 if failed == 0 else 1)
