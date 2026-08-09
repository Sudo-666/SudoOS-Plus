#!/usr/bin/env python3
"""Find code that loads a target address via pcalau12i+addi.d (or +ori)."""
import re

DIS = "/tmp/kernel_full.dis"
# target string VAs to find
TARGETS = {
    0x9000000090302710: "TASK00",
    0x9000000090302780: "TASK01",
    0x9000000090302794: "TASK02",
    0x90000000903027d3: "TASK19",
    0x90000000903027f0: "TASK20",
    0x9000000090302828: "TASK03",
}

addr_re = re.compile(r"^([0-9a-f]+):\s+([0-9a-f]{8})\s+([a-z0-9_.]+)\s+(.*)$")
insns = []
for line in open(DIS, encoding="utf-8", errors="replace"):
    m = addr_re.match(line.strip())
    if not m:
        continue
    insns.append((int(m.group(1), 16), m.group(3), m.group(4)))

# pcalau12i base: (addr & ~0xfff) + (si20 << 12)
pending = {}
for i, (addr, mn, ops) in enumerate(insns):
    if mn == "pcalau12i":
        mm = re.match(r"\$([a-z0-9]+),\s*(-?[0-9]+)", ops)
        if mm:
            reg, imm = mm.group(1), int(mm.group(2))
            if imm >= (1 << 19):
                imm -= (1 << 20)
            base = (addr & ~0xFFF) + (imm << 12)
            pending[reg] = (base, addr)
    elif mn == "addi.d":
        mm = re.match(r"\$([a-z0-9]+),\s*\$([a-z0-9]+),\s*(-?[0-9]+)", ops)
        if mm and mm.group(2) in pending:
            reg2, src, off = mm.group(1), mm.group(2), int(mm.group(3))
            b, pc = pending[src]
            if (b + off) in TARGETS:
                print(f"TASK str {TARGETS[b+off]} loaded at {addr:#x} (pcaddu at {pc:#x})")

# also catch lu12i.w+ori+lu32i.d long loads
pending2 = {}
for i, (addr, mn, ops) in enumerate(insns):
    if mn == "lu12i.w":
        mm = re.match(r"\$([a-z0-9]+),\s*(-?[0-9]+)", ops)
        if mm:
            reg, imm = mm.group(1), int(mm.group(2))
            val = ((imm & 0xFFFFF) << 12)
            pending2.setdefault(reg, {}).update({"hi": val, "pc": addr})
    elif mn == "ori":
        mm = re.match(r"\$([a-z0-9]+),\s*\$([a-z0-9]+),\s*0x([0-9a-f]+)", ops)
        if mm and mm.group(2) in pending2:
            r = mm.group(1); s = mm.group(2)
            lo = int(mm.group(3), 16)
            val = pending2[s]["hi"] | lo
            if (val >> 12) in [0x9000000090302 >> 12, 0x9000000090302 & 0xFFFFF]:
                pass
            pending2.setdefault(r, {}).update({"hi": val, "pc": pending2[s]["pc"]})
    elif mn == "lu32i.d":
        mm = re.match(r"\$([a-z0-9]+),\s*(-?[0-9]+)", ops)
        if mm and mm.group(1) in pending2:
            r = mm.group(1); imm = int(mm.group(2))
            v = pending2[r]["hi"]
            v = (v & 0xFFFFFFFF) | ((imm & 0xFFFFFFFF) << 32)
            if v in TARGETS:
                print(f"TASK str {TARGETS[v]} long-load at {addr:#x} (lu12i at {pending2[r].get('pc')})")
