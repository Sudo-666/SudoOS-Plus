#!/usr/bin/env python3
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
checks = []

def text(rel):
    p = ROOT / rel
    if not p.exists():
        print(f"FAIL missing {rel}")
        sys.exit(1)
    return p.read_text(encoding="utf-8")

def ok(name, cond):
    checks.append((name, bool(cond)))

spin = text("sync/src/spin_lock.rs")
irq = text("kernel/src/irq_lock.rs")
pa = text("kernel/src/page_alloc.rs")
mem = text("kernel/src/memory.rs")

ok("SpinLock exposes boot-only unchecked access", "get_mut_unchecked" in spin and "self.value.get()" in spin)
ok("IrqSpinLock forwards boot-only unchecked access", "get_mut_unchecked" in irq and "inner.get_mut_unchecked" in irq)
ok("page allocator has boot installer", "install_boot" in pa and "PAGE_ALLOCATOR.get_mut_unchecked" in pa)
ok("page allocator has boot init check", "is_initialized_boot" in pa and "PAGE_ALLOCATOR.get_mut_unchecked().is_some()" in pa)
install_fn = re.search(r"pub\s+fn\s+install\s*\([^{}]*\)\s*->[^{}]*\{(?P<body>.*?)\n\}", pa, re.S)
ok("runtime install still uses IRQ lock", install_fn and "PAGE_ALLOCATOR.lock()" in install_fn.group("body"))
start = mem.find('#[cfg(target_arch = "riscv64")]')
# choose the cfg block after physical page allocator summary, not earlier RISC-V helpers
phys = mem.find('physical page allocator')
start = mem.find('#[cfg(target_arch = "riscv64")]', phys)
end = mem.find('#[cfg(not(target_arch = "riscv64"))]', start)
riscv = mem[start:end] if start >= 0 and end > start else ""
non = mem[end:mem.find('KernelMemoryState', end)] if end >= 0 else ""
ok("RISC-V allocator path uses boot installer", "install_boot(page_allocator)" in riscv)
ok("RISC-V allocator path avoids runtime install lock", "crate::page_alloc::install(page_allocator)" not in riscv)
ok("RISC-V allocator init check avoids runtime lock", "is_initialized_boot" in riscv and "is_initialized()" not in riscv)
ok("non-RISC-V allocator path keeps runtime install", "crate::page_alloc::install(page_allocator)" in non)

passed = sum(1 for _, c in checks if c)
failed = len(checks) - passed
for name, cond in checks:
    print(("PASS" if cond else "FAIL") + f" {name}")
print(f"oscomp-riscv-boot-pagealloc-chain-audit: PASS={passed} FAIL={failed}")
if failed:
    sys.exit(1)
