#!/usr/bin/env python3
from __future__ import annotations
import pathlib, re, sys

root = pathlib.Path(__file__).resolve().parents[1]
linker_rs = (root / "kernel/src/linker.rs").read_text(encoding="utf-8")
linker_ld = (root / "arch/riscv64/src/platform/qemu_virt/linker.ld").read_text(encoding="utf-8")

checks: list[tuple[str, bool]] = []

def has(pat: str, text: str, flags: int = 0) -> bool:
    return re.search(pat, text, flags) is not None

checks.append(("RISC-V linker script uses current compact rodata section", ".rodata : AT(" in linker_ld))
checks.append(("RISC-V rodata includes normal rodata", "*(.rodata .rodata.*)" in linker_ld))
checks.append(("RISC-V rodata absorbs small rodata orphans", "*(.srodata .srodata.*)" in linker_ld))
checks.append(("RISC-V rodata absorbs data.rel.ro orphans", "*(.data.rel.ro .data.rel.ro.*)" in linker_ld))
checks.append(("RISC-V rodata absorbs sdata2 constants", "*(.sdata2 .sdata2.*)" in linker_ld))
checks.append(("RISC-V rodata remains page aligned", "__rodata_end" in linker_ld and ". = ALIGN(4K); __rodata_end" in linker_ld))
checks.append(("RISC-V data start remains page aligned", "OSKernel2026: page-align first writable RISC-V PT_LOAD" in linker_ld))

checks.append((
    "RISC-V kernel layout maps rodata through data start",
    has(r"let\s+rodata\s*=\s*riscv_kernel_symbol_range\(\s*core::ptr::addr_of!\(__rodata_start\)\s*,\s*core::ptr::addr_of!\(__data_start\)\s*,\s*\"rodata\"\s*,\s*\)\s*;", linker_rs),
))
checks.append((
    "RISC-V kernel layout no longer ends rodata at __rodata_end",
    not has(r"let\s+rodata\s*=\s*riscv_kernel_symbol_range\(\s*core::ptr::addr_of!\(__rodata_start\)\s*,\s*core::ptr::addr_of!\(__rodata_end\)", linker_rs),
))
checks.append(("LoongArch cached layout path is still present", "cached_symbol_range" in linker_rs))
checks.append(("Kernel segment count remains three", "segments: [KernelSegment; 3]" in linker_rs))

fail = 0
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
    fail += 0 if ok else 1
print(f"oscomp-riscv-kernel-image-gap-audit: PASS={len(checks)-fail}, FAIL={fail}")
sys.exit(1 if fail else 0)
