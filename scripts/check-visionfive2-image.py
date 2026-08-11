#!/usr/bin/env python3
"""
VisionFive 2 (JH7110) 内核镜像检查：验证 ELF / raw 二进制与板级物理布局一致。

用法:
    python3 scripts/check-visionfive2-image.py \
        --elf kernel-visionfive2.elf \
        --bin kernel-vf2.bin

检查项:
    - ELF64 little-endian, e_machine = EM_RISCV (243)
    - entry 与最低 PT_LOAD 物理地址均为 0x40200000
    - SMP trampoline 落在 0x40300000 的 PT_LOAD 内
    - 高半 .text LMA 从 0x40400000 开始 (VMA 0xffffffff80000000)
    - raw 二进制与 ELF 按 PT_LOAD 铺平后的内容逐字节一致
    - raw 末端 < 0x46000000 (DTB/initramfs 预留)
    - 产物不是 QEMU 的 0x80200000 镜像
"""

import argparse
import struct
import sys
from pathlib import Path

# ── VisionFive 2 固定布局 (见 SudoOS-VisionFive2-TFTP-CodePlan §3) ─────────
BOOT_PHYS_BASE = 0x4020_0000
TRAMPOLINE = 0x4030_0000
KERNEL_PHYS_BASE = 0x4040_0000
KERNEL_VIRT_BASE = 0xFFFF_FFFF_8000_0000
DTB_RESERVE = 0x4600_0000          # DTB/initramfs 目标地址,内核不得触碰
QEMU_BOOT_BASE = 0x8020_0000       # QEMU virt 的启动基址,误用即失败

EM_RISCV = 243
PT_LOAD = 1


class CheckFailure(Exception):
    pass


def read_segments(elf_path):
    """读取 ELF: 返回 (e_entry, [segment dicts])。"""
    with open(elf_path, 'rb') as f:
        ident = f.read(16)
        if ident[:4] != b'\x7fELF':
            raise CheckFailure(f"not a valid ELF: {elf_path}")
        if ident[4] != 2:
            raise CheckFailure("only 64-bit ELF supported")
        if ident[5] != 1:
            raise CheckFailure("only little-endian ELF supported")

        f.seek(0)
        ehdr = f.read(64)
        e_machine = struct.unpack_from('<H', ehdr, 18)[0]
        e_entry = struct.unpack_from('<Q', ehdr, 24)[0]
        e_phoff = struct.unpack_from('<Q', ehdr, 32)[0]
        e_phentsize = struct.unpack_from('<H', ehdr, 54)[0]
        e_phnum = struct.unpack_from('<H', ehdr, 56)[0]

        segments = []
        f.seek(e_phoff)
        for _ in range(e_phnum):
            phdr = f.read(e_phentsize)
            p_type = struct.unpack_from('<I', phdr, 0)[0]
            if p_type != PT_LOAD:
                continue
            segments.append({
                'offset': struct.unpack_from('<Q', phdr, 8)[0],
                'vaddr': struct.unpack_from('<Q', phdr, 16)[0],
                'paddr': struct.unpack_from('<Q', phdr, 24)[0],
                'filesz': struct.unpack_from('<Q', phdr, 32)[0],
                'memsz': struct.unpack_from('<Q', phdr, 40)[0],
            })

    if not segments:
        raise CheckFailure("ELF has no PT_LOAD segments")
    return e_machine, e_entry, segments


def flatten_raw(elf_path, segments):
    """按 elf-to-uimage.py extract_raw_bytes 的同一规则把 ELF 铺平为 raw。"""
    load_addr = min(s['paddr'] for s in segments)
    total_size = 0
    for s in segments:
        end = s['paddr'] - load_addr + s['memsz']
        if end > total_size:
            total_size = end

    raw = bytearray(total_size)
    with open(elf_path, 'rb') as f:
        for s in segments:
            offset = s['paddr'] - load_addr
            if s['filesz'] > 0:
                f.seek(s['offset'])
                data = f.read(s['filesz'])
                raw[offset:offset + s['filesz']] = data
    return bytes(raw), load_addr


def check_image(elf_path, bin_path):
    e_machine, e_entry, segments = read_segments(elf_path)

    print(f"ELF machine    : {e_machine} ({'EM_RISCV' if e_machine == EM_RISCV else '??'})")
    if e_machine != EM_RISCV:
        raise CheckFailure(f"e_machine = {e_machine} (want EM_RISCV = {EM_RISCV})")
    print("ELF class/endian: 64-bit little-endian")

    # ── entry / 最低 PT_LOAD ──
    lowest = min(s['paddr'] for s in segments)
    print(f"ELF entry      : 0x{e_entry:016x}")
    print(f"lowest PT_LOAD : 0x{lowest:016x}")
    if e_entry != BOOT_PHYS_BASE:
        raise CheckFailure(f"e_entry = 0x{e_entry:x} (want 0x{BOOT_PHYS_BASE:x})")
    if lowest != BOOT_PHYS_BASE:
        raise CheckFailure(f"lowest PT_LOAD = 0x{lowest:x} (want 0x{BOOT_PHYS_BASE:x})")

    # ── 不是 QEMU 镜像 ──
    if e_entry == QEMU_BOOT_BASE or lowest == QEMU_BOOT_BASE:
        raise CheckFailure("this is a QEMU virt 0x80200000 image, not VisionFive 2")

    # ── trampoline 落在 0x40300000 ──
    in_tramp = [
        s for s in segments
        if s['paddr'] <= TRAMPOLINE < s['paddr'] + s['memsz']
    ]
    if not in_tramp:
        raise CheckFailure(f"no PT_LOAD covers SMP trampoline at 0x{TRAMPOLINE:x}")
    print(f"trampoline     : covered by PT_LOAD LMA "
          f"0x{in_tramp[0]['paddr']:x}+0x{in_tramp[0]['memsz']:x}")

    # ── 高半 .text LMA 从 0x40400000 开始 ──
    high_text = [
        s for s in segments
        if s['paddr'] == KERNEL_PHYS_BASE and s['vaddr'] == KERNEL_VIRT_BASE
    ]
    if not high_text:
        raise CheckFailure(
            f"no PT_LOAD starts at LMA 0x{KERNEL_PHYS_BASE:x} "
            f"with VMA 0x{KERNEL_VIRT_BASE:016x}"
        )
    print(f"high-half .text: LMA 0x{KERNEL_PHYS_BASE:x} VMA 0x{KERNEL_VIRT_BASE:016x}")

    # ── raw 与 ELF 铺平一致 ──
    raw, load_addr = flatten_raw(elf_path, segments)
    bin_data = Path(bin_path).read_bytes()
    print(f"raw size       : {len(raw)} bytes")
    if len(bin_data) != len(raw):
        raise CheckFailure(f"raw size {len(bin_data)} != ELF-flattened {len(raw)}")
    if bin_data != raw:
        raise CheckFailure("raw binary differs from ELF-flattened PT_LOAD content")
    print("raw content    : == ELF-flattened PT_LOADs")

    # ── raw 末端不碰 DTB ──
    max_end = max(s['paddr'] + s['memsz'] for s in segments)
    print(f"raw end        : 0x{max_end:x}")
    if max_end > DTB_RESERVE:
        raise CheckFailure(
            f"kernel end 0x{max_end:x} >= DTB reserve 0x{DTB_RESERVE:x}"
        )
    print(f"DTB separation : OK (kernel end < 0x{DTB_RESERVE:x})")

    print()
    print("VISIONFIVE2_IMAGE_CHECK : PASS")


def main():
    ap = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument('--elf', required=True, help="kernel ELF (kernel-visionfive2)")
    ap.add_argument('--bin', required=True, help="raw kernel binary (kernel-vf2.bin)")
    args = ap.parse_args()

    for label, p in (('--elf', args.elf), ('--bin', args.bin)):
        if not Path(p).is_file():
            print(f"error: {label} file not found: {p}", file=sys.stderr)
            sys.exit(1)

    try:
        check_image(args.elf, args.bin)
    except CheckFailure as exc:
        print(f"VISIONFIVE2_IMAGE_CHECK : FAIL — {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
