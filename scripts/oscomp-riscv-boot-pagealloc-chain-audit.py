#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
mem = (root / "kernel/src/memory.rs").read_text()
pa = (root / "kernel/src/page_alloc.rs").read_text()
irq = (root / "kernel/src/irq_lock.rs").read_text()
spin = (root / "sync/src/spin_lock.rs").read_text()
checks = []

def check(name, cond):
    checks.append((name, bool(cond)))

fn = mem[mem.find("pub fn initialize_page_allocator("):]
start = fn.find('#[cfg(target_arch = "riscv64")]')
end = fn.find('#[cfg(not(target_arch = "riscv64"))]', start)
riscv = fn[start:end] if start >= 0 and end >= 0 else ""
check("SpinLock exposes boot-only direct mutable access", "get_mut_unchecked" in spin)
check("IrqSpinLock forwards boot-only direct mutable access", "get_mut_unchecked" in irq)
check("page allocator has boot install API", "install_boot" in pa)
check("page allocator boot install uses direct lock storage", "PAGE_ALLOCATOR.get_mut_unchecked" in pa)
check("RISC-V memory path calls install_boot", "install_boot(page_allocator)" in riscv)
check("RISC-V memory path avoids runtime install lock", "page_alloc::install(page_allocator)" not in riscv)
check("RISC-V memory path avoids post-install boot reread", "is_initialized_boot" not in riscv)
check("temporary pagealloc trace removed", "riscv page_alloc install_boot:" not in mem + pa)
check("normal runtime allocator path still exists", "pub fn install(" in pa and "PAGE_ALLOCATOR.lock()" in pa)

passed = sum(v for _, v in checks)
failed = len(checks) - passed
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
print(f"oscomp-riscv-boot-pagealloc-chain-audit: PASS={passed} FAIL={failed}")
sys.exit(0 if failed == 0 else 1)
