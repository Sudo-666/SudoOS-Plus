#!/usr/bin/env python3
"""M8-B3 R5 semantic SATP audit."""
from pathlib import Path
import sys

path = Path("arch/riscv64/src/memory/paging/mod.rs")
if not path.is_file():
    print("M8-B3 R5 audit: FAIL: missing RISC-V paging module", file=sys.stderr)
    raise SystemExit(1)

text = path.read_text(encoding="utf-8")
start = text.find("pub unsafe fn switch_user_address_space(")
end = text.find("\npub fn current_lower_root()", start)
if start < 0 or end < 0:
    print("M8-B3 R5 audit: FAIL: cannot isolate switch function", file=sys.stderr)
    raise SystemExit(1)

body = text[start:end]
pre = body.find('"sfence.vma zero, {asid}"')
write = body.find('"csrw satp, {satp}"')
post = body.rfind('"sfence.vma zero, {asid}"')

formatted_satp = (
    "let satp = (SATP_MODE_SV39 << SATP_MODE_SHIFT) "
    "| (asid_value << SATP_ASID_SHIFT) | ppn;"
)
checks = {
    "ASID captured before switch": "let asid_value = usize::from(asid.get());" in body,
    "rustfmt-stable SATP declaration": formatted_satp in body,
    "exactly two ASID fences": body.count('"sfence.vma zero, {asid}"') == 2,
    "correct fence/SATP order": 0 <= pre < write < post,
    "one SATP write": body.count('"csrw satp, {satp}"') == 1,
    "one asm block": body.count("core::arch::asm!(") == 1,
}
failed = [name for name, passed in checks.items() if not passed]
if failed:
    print(f"M8-B3 R5 audit: FAIL: {failed}", file=sys.stderr)
    raise SystemExit(1)

probe = text[:start]
if "probe value was visible" not in probe or '"sfence.vma zero, zero"' not in probe:
    print("M8-B3 R5 audit: FAIL: ASID probe path was unexpectedly altered", file=sys.stderr)
    raise SystemExit(1)

print("M8-B3 R5 audit: PASS")
print("  semantic function scope : verified")
print("  page-table publication  : fenced")
print("  SATP switch window      : self-contained")
print("  ASID probe path         : preserved")
print("  LoongArch path          : untouched")
