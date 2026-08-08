#!/usr/bin/env python3
"""
LS2K1000 uImage 镜像检查：验证 ELF / kernel.bin / uImage 三者的一致性，
并确认 bootm 的暂存地址与 payload 目标地址不重叠。

用法:
    python3 scripts/check-ls2k1000-image.py \
        --elf kernel-ls2k1000.elf \
        --bin kernel-ls2k1000.bin \
        --image kernel-ls2k1000.uImage

检查项:
    - ELF e_entry 与 LOAD 段
    - uImage magic (0x27051956)
    - uImage header CRC (ih_hcrc)
    - uImage data CRC (ih_dcrc)
    - architecture == 27 (LoongArch, 厂商 2022.04 BSP 枚举)
    - load/entry 与厂商启动代码期望值一致 (默认 0x90000000)
    - payload == 原始 kernel.bin
    - bootm 暂存地址 (--stash) 与 payload 目标地址不重叠
    - DTB 地址 (--dtb) 与暂存范围分开

参考（厂商 U-Boot 源码）:
    include/image.h      : IH_ARCH_LA = 27, ih_load/ih_ep 为 uint32
    boot/bootm.c         : image_get_kernel 校验 magic/hcrc/dcrc/arch
    arch/loongarch/lib/bootm.c : map_to_sysmem(ep) -> cached 窗口跳转
"""

import argparse
import struct
import sys
import zlib
from pathlib import Path

# ── 常量 ────────────────────────────────────────────────────
IH_MAGIC = 0x27051956
IH_ARCH_LOONGARCH = 27          # 厂商 2022.04 BSP: LoongArch = 27
IH_ARCH_LOONGARCH_NAME = "LoongArch"
PHYS_MASK = 0x0FFF_FFFF_FFFF_FFFF   # LoongArch DMW 物理地址掩码 (48 位)
UINT32_MAX = 0xFFFFFFFF

# 默认期望值（与厂商启动代码一致: load=entry=0x90000000）
DEFAULT_LOAD = 0x90000000
DEFAULT_ENTRY = 0x90000000
# 默认 bootm 暂存地址（cached VA of 物理 0x02000000）与 DTB 地址 (env fdt_addr)
DEFAULT_STASH = 0x9000000002000000
DEFAULT_DTB = 0x900000000a000000

HEADER_FMT = '>IIIII I I BB BB 32s'   # 64 字节 legacy 头


class CheckFailure(Exception):
    pass


def read_elf(elf_path):
    """读取 64 位小端 ELF: 返回 (e_entry, load_segments)。"""
    with open(elf_path, 'rb') as f:
        ident = f.read(16)
        if ident[:4] != b'\x7fELF':
            raise CheckFailure(f"not a valid ELF: {elf_path}")
        if ident[4] != 2:      # 64-bit
            raise CheckFailure("only 64-bit ELF supported")
        if ident[5] != 1:      # little-endian
            raise CheckFailure("only little-endian ELF supported")

        f.seek(0)
        ehdr = f.read(64)
        e_entry = struct.unpack_from('<Q', ehdr, 24)[0]
        e_phoff = struct.unpack_from('<Q', ehdr, 32)[0]
        e_phentsize = struct.unpack_from('<H', ehdr, 54)[0]
        e_phnum = struct.unpack_from('<H', ehdr, 56)[0]

        segments = []
        f.seek(e_phoff)
        for _ in range(e_phnum):
            phdr = f.read(e_phentsize)
            p_type = struct.unpack_from('<I', phdr, 0)[0]
            if p_type == 1:  # PT_LOAD
                p_paddr = struct.unpack_from('<Q', phdr, 24)[0]
                p_filesz = struct.unpack_from('<Q', phdr, 32)[0]
                p_memsz = struct.unpack_from('<Q', phdr, 40)[0]
                segments.append((p_paddr, p_paddr + p_memsz))

    return e_entry, segments


def parse_uimage(path):
    """解析 legacy uImage 头，返回字段 dict。"""
    with open(path, 'rb') as f:
        data = f.read()
    if len(data) < 64:
        raise CheckFailure("uImage too small (no 64-byte header)")

    magic, hcrc, time, size, load, ep, dcrc, os, arch, itype, comp, name = \
        struct.unpack(HEADER_FMT, data[:64])
    name = name.rstrip(b'\x00').decode('ascii', 'replace')
    payload = data[64:64 + size]
    if len(payload) < size:
        raise CheckFailure(f"uImage truncated: header says {size} bytes data, "
                           f"only {len(data) - 64} present")

    return {
        'data': data,
        'payload': payload,
        'magic': magic,
        'hcrc': hcrc,
        'time': time,
        'size': size,
        'load': load,
        'ep': ep,
        'dcrc': dcrc,
        'os': os,
        'arch': arch,
        'type': itype,
        'comp': comp,
        'name': name,
    }


def uimage_crc32(data):
    return zlib.crc32(data) & 0xFFFFFFFF


def check_image(elf_path, bin_path, image_path, stash, dtb):
    """执行全部检查，逐行打印；任一失败抛 CheckFailure。"""
    e_entry, segments = read_elf(elf_path)
    img = parse_uimage(image_path)
    payload = img['payload']

    # ── ELF ──
    print(f"ELF entry       : 0x{e_entry:016x}")
    if not segments:
        raise CheckFailure("ELF has no PT_LOAD segments")
    segs = " ".join(f"0x{s:x}-0x{e:x}" for s, e in segments)
    print(f"ELF LOAD segs   : {segs}")

    # ── magic / CRC ──
    if img['magic'] != IH_MAGIC:
        raise CheckFailure(f"bad uImage magic 0x{img['magic']:08x} "
                           f"(want 0x{IH_MAGIC:08x})")
    print("uImage magic    : OK (0x27051956)")

    # header CRC: 置零 hcrc 字段后对整个头计算
    hdr = bytearray(img['data'][:64])
    hdr[4:8] = b'\x00\x00\x00\x00'
    hcrc_calc = uimage_crc32(bytes(hdr))
    if hcrc_calc != img['hcrc']:
        raise CheckFailure(f"bad header CRC: stored 0x{img['hcrc']:08x}, "
                           f"computed 0x{hcrc_calc:08x}")
    print("uImage head CRC : OK")

    dcrc_calc = uimage_crc32(payload)
    if dcrc_calc != img['dcrc']:
        raise CheckFailure(f"bad data CRC: stored 0x{img['dcrc']:08x}, "
                           f"computed 0x{dcrc_calc:08x}")
    print("uImage data CRC : OK")

    # ── arch ──
    if img['arch'] != IH_ARCH_LOONGARCH:
        raise CheckFailure(f"uImage arch = {img['arch']} (want "
                           f"{IH_ARCH_LOONGARCH} = {IH_ARCH_LOONGARCH_NAME})")
    print(f"uImage arch     : {IH_ARCH_LOONGARCH_NAME}")

    # ── load / entry ──
    if img['load'] != DEFAULT_LOAD:
        raise CheckFailure(f"uImage load = 0x{img['load']:x} "
                           f"(want 0x{DEFAULT_LOAD:x})")
    if img['ep'] != DEFAULT_ENTRY:
        raise CheckFailure(f"uImage entry = 0x{img['ep']:x} "
                           f"(want 0x{DEFAULT_ENTRY:x})")
    print(f"uImage load     : 0x{img['load']:08x}")
    print(f"uImage entry    : 0x{img['ep']:08x}")

    # ── payload == kernel.bin ──
    bin_data = Path(bin_path).read_bytes()
    if len(bin_data) != len(payload) or bin_data != payload:
        raise CheckFailure(
            f"payload != {bin_path}: bin {len(bin_data)} bytes, "
            f"payload {len(payload)} bytes"
        )
    print(f"uImage payload  : == {bin_path} ({len(payload)} bytes)")

    # ── 暂存地址 vs payload 目标地址 ──
    load = img['load']
    size = img['size']
    stash_phys = stash & PHYS_MASK
    stash_range = (stash_phys, stash_phys + 64 + size)   # 头 + payload
    target_range = (load, load + size)

    def ranges_overlap(a, b):
        return a[0] < b[1] and b[0] < a[1]

    if ranges_overlap(stash_range, target_range):
        raise CheckFailure(
            f"overlap: stash [0x{stash_range[0]:x}, 0x{stash_range[1]:x}) "
            f"overlaps payload target [0x{target_range[0]:x}, "
            f"0x{target_range[1]:x}) — pick a disjoint --stash"
        )
    print(f"image overlap   : NO (stash 0x{stash_phys:016x} vs "
          f"target 0x{load:016x})")

    # ── DTB 与暂存范围分开 ──
    dtb_phys = dtb & PHYS_MASK
    dtb_range = (dtb_phys, dtb_phys + 0x100000)   # 按 1 MiB 上限估算
    if ranges_overlap(stash_range, dtb_range):
        raise CheckFailure(
            f"overlap: DTB [0x{dtb_range[0]:x}, 0x{dtb_range[1]:x}) overlaps "
            f"uImage stash [0x{stash_range[0]:x}, 0x{stash_range[1]:x})"
        )
    print(f"dtb separation  : OK (dtb 0x{dtb_phys:016x})")

    print()
    print("IMAGE_CHECK     : PASS")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--elf', required=True, help="kernel ELF (e.g. kernel-ls2k1000.elf)")
    ap.add_argument('--bin', required=True, help="raw kernel binary (e.g. kernel-ls2k1000.bin)")
    ap.add_argument('--image', required=True, help="uImage to check (e.g. kernel-ls2k1000.uImage)")
    ap.add_argument('--stash', default=DEFAULT_STASH,
                    help=f"bootm 暂存地址 (hex, cached VA), default 0x{DEFAULT_STASH:x}")
    ap.add_argument('--dtb', default=DEFAULT_DTB,
                    help=f"DTB 地址 (hex), default 0x{DEFAULT_DTB:x}")
    args = ap.parse_args()

    stash = int(str(args.stash), 0)
    dtb = int(str(args.dtb), 0)

    for label, p in (('--elf', args.elf), ('--bin', args.bin), ('--image', args.image)):
        if not Path(p).is_file():
            print(f"error: {label} file not found: {p}", file=sys.stderr)
            sys.exit(1)

    try:
        check_image(args.elf, args.bin, args.image, stash, dtb)
    except CheckFailure as exc:
        print(f"IMAGE_CHECK     : FAIL — {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
