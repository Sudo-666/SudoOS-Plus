#!/usr/bin/env python3
"""Decode pcalau12i+addi.d pairs in the handler, print the referenced strings."""
import re

data = open("/mnt/d/oskernel2026-0xdeadbeef/kernel-ls2k1000", "rb").read()
base_va = 0x9000000090200000  # .text start; file offset of VA addr = addr - base_va + section_offset

# We need section mapping: .text file offset 0x20000 (from readelf earlier: .text at file 0x20000, VA 0x90200000)
# rodata/data offsets unknown — approximate via known string offsets.
# Instead: dump the disasm and compute targets, then locate in binary by matching a nearby known string.

DIS = "/mnt/d/oskernel2026-0xdeadbeef/.tmp_dis_hand"
import subprocess
subprocess.run(["loongarch64-linux-gnu-objdump", "-d",
                "--start-address=0x90000000902e7780",
                "--stop-address=0x90000000902e7b00",
                "/mnt/d/oskernel2026-0xdeadbeef/kernel-ls2k1000"],
               stdout=open(DIS, "w"))

addr_re = re.compile(r"^([0-9a-f]+):\s+[0-9a-f]{8}\s+([a-z0-9_.$<>]+)\s+(.*)$")
insns = []
for line in open(DIS, encoding="utf-8", errors="replace"):
    m = addr_re.match(line.strip())
    if m:
        insns.append((int(m.group(1), 16), m.group(2), m.group(3)))

def num(s):
    s = s.strip()
    return int(s, 16) if s.lower().startswith("0x") else int(s)

def pcalau12i_target(addr, imm):
    return (addr & ~0xFFF) + (imm << 12)

# find all pcalau12i and following addi.d in same "block"
res = []
i = 0
while i < len(insns):
    addr, mn, ops = insns[i]
    if mn == "pcalau12i":
        mm = re.match(r"\$([a-z0-9]+),\s*(-?0x[0-9a-f]+|-?[0-9]+)", ops)
        if mm:
            reg, imm = mm.group(1), num(mm.group(2))
            base = pcalau12i_target(addr, imm)
            # look ahead up to 4 insns for addi.d using same reg
            for j in range(i+1, min(i+5, len(insns))):
                a2, mn2, ops2 = insns[j]
                mm2 = re.match(r"\$([a-z0-9]+),\s*\$([a-z0-9]+),\s*(-?0x[0-9a-f]+|-?[0-9]+)", ops2)
                if mn2 == "addi.d" and mm2 and mm2.group(2) == reg:
                    tgt = base + num(mm2.group(3))
                    res.append((addr, tgt))
                    break
    i += 1

# print targets; try to read bytes at those VAs (assume file offset = va - 0x90200000 + 0x20000)
for src, tgt in res:
    off = tgt - 0x9000000090200000 + 0x20000
    if 0 <= off < len(data):
        chunk = data[off:off+64]
        text = chunk.split(b"\x00")[0]
        print(f"ref@{src:#x} -> {tgt:#x}: {text[:48]}")
    else:
        print(f"ref@{src:#x} -> {tgt:#x}: (out of .text)")
