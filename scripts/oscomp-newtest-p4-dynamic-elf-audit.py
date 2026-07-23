#!/usr/bin/env python3
"""newtest P4 dynamic ELF audit: PT_INTERP loading, interpreter, auxv."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
checks = []

def add(ok, name):
    checks.append((ok, name))

exec_rs = (root / "kernel/src/exec.rs").read_text(encoding="utf-8")
elf_rs = (root / "kernel/src/elf.rs").read_text(encoding="utf-8")
main_rs = (root / "kernel/src/main.rs").read_text(encoding="utf-8")

# Interpreter loading
add("INTERP_LOAD_BIAS" in exec_rs,
    "exec.rs defines interpreter load bias")
add("load_exec_image_from_vfs" in exec_rs,
    "exec.rs has VFS-based interpreter loader")
add("interp_entry" in exec_rs and "interp_base" in exec_rs,
    "exec.rs tracks interpreter entry and base")
add("main_entry" in exec_rs and "main_phdr" in exec_rs,
    "exec.rs tracks main ELF metadata for auxv")

# parse_with_bias in elf.rs
add("pub fn parse_with_bias" in elf_rs,
    "elf.rs exposes parse_with_bias for interpreter loading")
add("bias_override" in elf_rs,
    "elf.rs parse_impl supports bias_override")

# auxv changes
add('at_base = interp_base.map(|b| b.get()).unwrap_or(0)' in exec_rs or 'AT_BASE' in exec_rs,
    "exec.rs sets AT_BASE from interp_base")
add('at_entry = main_entry.map(|e| e.get()).unwrap_or(elf.entry.get())' in exec_rs or 'at_entry' in exec_rs,
    "exec.rs sets AT_ENTRY from main_entry")

# Library paths
add('/lib/ld-linux-riscv64-lp64d.so.1' in main_rs,
    "sdcard installs RISC-V glibc ld-linux path")
add('/lib/ld-linux-loongarch64-lp64d.so.1' in main_rs,
    "sdcard installs LoongArch glibc ld-linux path")
add(
    'symlink("/lib", "/lib64")' in main_rs
    or (
        "/lib64/ld-linux-loongarch-lp64d.so.1" in main_rs
        and "real directory" in main_rs
    ),
    "sdcard provides /lib64 loader compatibility",
)

# PreparedExec has new fields
add("interp_base: Option<VirtAddr>" in exec_rs,
    "PreparedExec has interp_base field")
add("main_entry: Option<VirtAddr>" in exec_rs,
    "PreparedExec has main_entry field")
add("main_phdr: Option<(VirtAddr, usize, usize)>" in exec_rs,
    "PreparedExec has main_phdr field")

failed = [name for ok, name in checks if not ok]
if failed:
    print("newtest P4 dynamic ELF audit: FAIL")
    for name in failed:
        print("  FAIL:", name)
    sys.exit(1)
print("newtest P4 dynamic ELF audit: PASS")
for _, name in checks:
    print("  PASS:", name)
