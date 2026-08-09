#!/usr/bin/env python3
"""Decode LoongArch pcaddu18i+jirl call targets. Verify against known case
__rust_alloc_error_handler(0x90201000): pcaddu18i $ra,4 -> 0x90300000, jirl -> __rg_oom 0x902e7e80."""
import re

def decode(raw_hex, addr):
    v = int(raw_hex, 16)
    op = (v >> 26) & 0x3F
    name = {0x13: "jirl", 0x14: "b", 0x07: "pcaddu18i"}.get(op, f"op{op:#x}")
    return name, v

def pcaddu18i_base(addr, imm20_signed):
    # rd = (pc & ~0x3FFFF) + (si20 << 18)
    return (addr & ~0x3FFFF) + (imm20_signed << 18)

def sign(n, bits):
    if n & (1 << (bits-1)):
        return n - (1 << bits)
    return n

def dec(pcaddu18i_imm, jirl_off, base):
    return base + jirl_off

# known verification case
a1 = 0x9000000090201000
r1 = 0x1e000081  # pcaddu18i $ra, 4
r2 = 0x4c737c21  # jirl $ra, $ra, ...
# decode pcaddu18i imm: bits[25:5]
imm = (r1 >> 5) & 0xFFFFF
imm = sign(imm, 20)
base = pcaddu18i_base(a1, imm)
off = sign((r2 >> 10) & 0xFFFF, 16)
print(f"verify: pcaddu18i imm={imm} base={base:#x} jirl_off={off} target={base+off:#x}")

# Now decode the two calls of interest from objdump disassembly
# (addr, pcaddu_raw, jirl_raw)
CALLS = [
    ("callA", 0x90000000902da89c, 0x1fffffa1, 0x4f598421),
    ("callB", 0x90000000902e17cc, 0x1fffff81, 0x4dfc2821),
    ("callC(handler->oom)", 0x90000000902e7e8c, 0x1e000001, 0x4ff8f421),
]
for label, a, r1, r2 in CALLS:
    op1 = (r1 >> 26) & 0x3F
    op2 = (r2 >> 26) & 0x3F
    imm = sign((r1 >> 5) & 0xFFFFF, 20)
    base = pcaddu18i_base(a, imm)
    off = sign((r2 >> 10) & 0xFFFF, 16)
    tgt = base + off
    rd2 = (r2 >> 5) & 0x1F
    rj2 = r2 & 0x1F
    print(f"{label}: pcaddu18i_op={op1:#x} imm={imm} base={base:#x}; jirl_op={op2:#x} rd=r{rd2} rj=r{rj2} off={off} TARGET={tgt:#x}")
