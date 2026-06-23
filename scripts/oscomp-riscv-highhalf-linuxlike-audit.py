#!/usr/bin/env python3
from pathlib import Path
import re
import sys

root = Path.cwd()
path = root / 'kernel/src/memory.rs'
if not path.exists():
    print("[oscomp-riscv-highhalf-linuxlike] FAIL: patched handoff file missing", path)
    sys.exit(1)
text = path.read_text(errors="ignore")
fail = []
if "oscomp_riscv_switch_stack_to" not in text:
    fail.append("assembly stack switch symbol missing")
if "fn oscomp_rebase_boot_stack_to_direct_map_once()" not in text:
    fail.append("safe stack handoff helper missing")
if "oscomp_rebase_boot_stack_to_direct_map_once();" not in text:
    fail.append("handoff call missing")
if "boot stack      : direct-map alias ready" in text:
    fail.append("old bare println handoff log still present")
if "low boot mapping: removed" in text:
    fail.append("low boot mapping removal status remains")
if re.search(r"unsafe\s+fn\s+oscomp_rebase_boot_stack_to_direct_map_once", text):
    fail.append("helper is unsafe fn; should be safe fn with internal unsafe blocks")
# Look specifically inside helper for unwrapped asm.  This is a textual guard,
# not a full Rust parser, but it catches the previous compile failure.
m = re.search(r"fn\s+oscomp_rebase_boot_stack_to_direct_map_once\s*\(\)\s*\{(?P<body>.*?)\n\}", text, re.S)
if not m:
    fail.append("could not parse helper body")
else:
    body = m.group("body")
    if "core::arch::asm!" in body and "unsafe {" not in body:
        fail.append("helper asm appears not to be wrapped in unsafe block")

if fail:
    print("[oscomp-riscv-highhalf-linuxlike] FAIL")
    for f in fail:
        print("  -", f)
    sys.exit(1)
print("[oscomp-riscv-highhalf-linuxlike] PASS: Linux-like RISC-V stack handoff installed")
print("  file:", path.relative_to(root))
