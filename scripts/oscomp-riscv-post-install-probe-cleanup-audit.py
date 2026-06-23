#!/usr/bin/env python3
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
mem = (root / 'kernel/src/memory.rs').read_text()
pa = (root / 'kernel/src/page_alloc.rs').read_text()
irq = (root / 'kernel/src/irq_lock.rs').read_text()
spin = (root / 'sync/src/spin_lock.rs').read_text()

checks = []

def check(name, ok):
    checks.append((name, bool(ok)))

trace_markers = [
    'riscv page_alloc install_boot:',
    'OSCOMP_RISCV_POST_FINAL_TRACE',
    'B0:release-enter',
]
check('diagnostic pagealloc trace removed', not any(m in mem + pa for m in trace_markers))
check('SpinLock exposes boot-only get_mut_unchecked', 'pub unsafe fn get_mut_unchecked(&self) -> &mut T' in spin)
check('IrqSpinLock forwards boot-only get_mut_unchecked', 'pub unsafe fn get_mut_unchecked(&self) -> &mut T' in irq and 'self.inner.get_mut_unchecked()' in irq)
check('page_alloc has boot installer', 'pub unsafe fn install_boot' in pa and 'PAGE_ALLOCATOR.get_mut_unchecked()' in pa and '*slot = Some(allocator)' in pa)
check('runtime installer still uses IRQ lock', 'pub fn install(allocator: BuddyAllocator)' in pa and 'PAGE_ALLOCATOR.lock()' in pa)

idx_rv = mem.find('#[cfg(target_arch = "riscv64")]')
idx_non = mem.find('#[cfg(not(target_arch = "riscv64"))]', idx_rv)
rv_block = mem[idx_rv:idx_non] if idx_rv >= 0 and idx_non > idx_rv else ''
non_block = mem[idx_non:] if idx_non >= 0 else ''
check('memory RISC-V path calls install_boot', 'install_boot(page_allocator)' in rv_block)
check('memory RISC-V path avoids post-install global reread', 'is_initialized_boot' not in rv_block and 'is_initialized()' not in rv_block)
check('memory RISC-V path has no verbose zone summary', 'zone_present_pages' not in rv_block and 'zone_free_pages' not in rv_block)
check('non-RISC-V keeps verbose zone summary', 'zone_present_pages' in non_block and 'zone_free_pages' in non_block)

passed = sum(ok for _, ok in checks)
failed = len(checks) - passed
for name, ok in checks:
    print(('PASS' if ok else 'FAIL') + ': ' + name)
print(f'oscomp-riscv-post-install-probe-cleanup-audit: PASS={passed}, FAIL={failed}')
sys.exit(0 if failed == 0 else 1)
