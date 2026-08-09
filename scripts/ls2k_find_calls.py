#!/usr/bin/env python3
"""Scan LoongArch disassembly using objdump-decoded operands; compute call
targets for pcaddu18i+jirl; find calls to __rust_alloc_error_handler etc."""
import re

DIS = "/tmp/kernel_full.dis"
TARGETS = {
    0x9000000090201000: "__rust_alloc_error_handler",
    0x90000000902e7e80: "__rg_oom",
    0x90000000902e7774: "ls2k_alloc_error_handler(+tail)",
    0x90000000902e7780: "ls2k_alloc_error_handler",
    0x9000000090250220: "KernelGlobalAllocator::allocate",
    0x90000000902013f4: "handle_alloc_error",
}

def num(s):
    s = s.strip()
    if s.lower().startswith("0x"):
        return int(s, 16)
    return int(s)

addr_re = re.compile(r"^([0-9a-f]+):\s+[0-9a-f]{8}\s+([a-z0-9_.$<>]+)\s+(.*)$")
insns = []
for line in open(DIS, encoding="utf-8", errors="replace"):
    m = addr_re.match(line.strip())
    if not m:
        continue
    insns.append((int(m.group(1), 16), m.group(2), m.group(3)))

# pcaddu18i sets a register; base = (pc & ~0x3FFFF) + (imm<<18)
pc = {}
hits = []
n_ins = 0
n_jirl = 0
n_pc = 0
for i, (addr, mn, ops) in enumerate(insns):
    if mn == "pcaddu18i":
        mm = re.match(r"\$([a-z0-9]+),\s*(-?0x[0-9a-f]+|-?[0-9]+)", ops)
        if mm:
            reg = mm.group(1)
            imm = num(mm.group(2))
            pc[reg] = (addr & ~0x3FFFF) + (imm << 18)
            n_pc += 1
    elif mn == "jirl":
        n_jirl += 1
        mm = re.match(r"\$([a-z0-9]+),\s*\$([a-z0-9]+),\s*(-?0x[0-9a-f]+|-?[0-9]+)", ops)
        if mm:
            rd, rj, off = mm.group(1), mm.group(2), num(mm.group(3))
            if rj == "zero":  # bl: target = pc + off
                tgt = addr + off
            elif rj in pc:
                tgt = pc[rj] + off
            else:
                continue
            if tgt in TARGETS:
                hits.append((addr, TARGETS[tgt]))
    else:
        n_ins += 1

print(f"parsed: total={len(insns)} non-pc/jirl={n_ins} pcaddu={n_pc} jirl={n_jirl}")
print(f"sample pc: ra={pc.get('ra', None)} t8={pc.get('t8', None)}")
print(f"=== {len(hits)} calls to OOM/alloc targets ===")
for addr, name in sorted(hits):
    print(f"{addr:#018x}  {name}")
