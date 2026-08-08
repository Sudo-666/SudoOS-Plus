#!/usr/bin/env python3
"""
将 MyOS 内核 ELF 转换为 U-Boot 可启动的 uImage 格式。

用法:
    python3 scripts/elf-to-uimage.py kernel-ls2k1000 [output.uImage]

依赖: Python 3.6+ (仅标准库，无需 mkimage)

生成的 uImage 可以直接在 LS2K1000 U-Boot 中使用:
    tftp 0x90000000 kernel.uImage
    bootm 0x90000000
"""

import struct
import sys
import os
import time
import zlib
from pathlib import Path

# ── uImage 常量 ─────────────────────────────────────────────
IH_MAGIC = 0x27051956          # uImage 魔数

# 架构代码（来自 LoongArch-patched U-Boot）
# U-Boot 2022.04 BSP: IH_ARCH_LOONGARCH = 27 (紧接 RISCV=26)
# U-Boot 主线:       IH_ARCH_LOONGARCH = 43
# U-Boot 2025.x:     枚举已重新编号
IH_ARCH_RISCV = 26            # U-Boot 2022.04: RISC-V = 26 (VisionFive 2)
IH_ARCH_LOONGARCH = 27        # U-Boot 2022.04 BSP 值
IH_ARCH_MIPS64 = 6            # U-Boot 2022.04: MIPS64 = 6
IH_ARCH_MIPS = 5              # U-Boot 2022.04: MIPS = 5

# OS 代码
IH_OS_LINUX = 17                # Linux 内核
IH_OS_U_BOOT = 20               # 固件

# 镜像类型
IH_TYPE_KERNEL = 2              # 内核
IH_TYPE_KERNEL_NOLOAD = 14      # 无需加载的内核 (u-boot 不复制)
IH_TYPE_STANDALONE = 1          # 独立程序

# 压缩类型
IH_COMP_NONE = 0                # 无压缩

# uImage 头部结构: 64 字节
# ih_magic(4) ih_hcrc(4) ih_time(4) ih_size(4)
# ih_load(4) ih_ep(4) ih_dcrc(4) ih_os(1) ih_arch(1)
# ih_type(1) ih_comp(1) ih_name(32)
HEADER_FMT = '>IIIII I I BB BB 32s'  # big-endian, 64 bytes total


def read_elf_info(elf_path):
    """从 ELF 文件中提取物理入口地址和链接地址。"""
    with open(elf_path, 'rb') as f:
        ident = f.read(16)

        if ident[:4] != b'\x7fELF':
            raise ValueError(f"{elf_path}: not a valid ELF file")

        # 判断 32/64 位
        elf_class = ident[4]   # 1=32bit, 2=64bit
        elf_endian = ident[5]  # 1=little, 2=big

        if elf_class != 2:
            raise ValueError("only 64-bit ELF is supported")
        if elf_endian != 1:
            raise ValueError("only little-endian ELF is supported")

        # 读取完整的 ELF 头 (64-bit = 64 字节)
        f.seek(0)
        elf_header = f.read(64)

        # 解析 ELF header (64-bit)
        e_entry = struct.unpack_from('<Q', elf_header, 24)[0]  # 入口 VMA
        e_phoff = struct.unpack_from('<Q', elf_header, 32)[0]  # 程序头表偏移
        e_phentsize = struct.unpack_from('<H', elf_header, 54)[0]  # 程序头大小
        e_phnum = struct.unpack_from('<H', elf_header, 56)[0]  # 程序头数量

        # 遍历程序头表，找到 LOAD 段的最低物理地址
        lowest_paddr = None
        segments = []

        f.seek(e_phoff)
        for i in range(e_phnum):
            phdr = f.read(e_phentsize)
            p_type = struct.unpack_from('<I', phdr, 0)[0]
            if p_type == 1:  # PT_LOAD
                p_offset = struct.unpack_from('<Q', phdr, 8)[0]
                p_vaddr = struct.unpack_from('<Q', phdr, 16)[0]
                p_paddr = struct.unpack_from('<Q', phdr, 24)[0]
                p_filesz = struct.unpack_from('<Q', phdr, 32)[0]
                p_memsz = struct.unpack_from('<Q', phdr, 40)[0]
                p_flags = struct.unpack_from('<I', phdr, 4)[0]  # PF_X=1, PF_W=2, PF_R=4

                # 物理地址（如果有 AT() 则等于 p_paddr，否则等于 p_vaddr）
                phys_addr = p_paddr if p_paddr != 0 else p_vaddr
                if lowest_paddr is None or phys_addr < lowest_paddr:
                    lowest_paddr = phys_addr

                segments.append({
                    'offset': p_offset,
                    'vaddr': p_vaddr,
                    'paddr': phys_addr,
                    'filesz': p_filesz,
                    'memsz': p_memsz,
                    'flags': p_flags,
                })

        if lowest_paddr is None:
            raise ValueError("no PT_LOAD segments found in ELF")

        # 按文件偏移排序
        segments.sort(key=lambda s: s['offset'])

    return e_entry, lowest_paddr, segments


def extract_raw_kernel(elf_path, segments):
    """从 ELF 提取内核原始二进制，去除段间空隙。"""
    # 计算需要的二进制大小
    min_offset = min(s['offset'] for s in segments)
    max_offset = max(s['offset'] + s['filesz'] for s in segments)

    with open(elf_path, 'rb') as f:
        f.seek(min_offset)
        raw = f.read(max_offset - min_offset)

    return raw, min_offset


def uimage_crc32(data):
    """计算 uImage 兼容的 CRC32 (与 zlib.crc32 一致)。"""
    return zlib.crc32(data) & 0xFFFFFFFF


# LoongArch cached DMW 窗口前缀与 48 位物理地址掩码。
CACHED_BASE = 0x9000_0000_0000_0000
PHYS_MASK = 0x0FFF_FFFF_FFFF_FFFF


def physical_entry_address(vma_entry, load_addr, lowest_paddr):
    """
    把 ELF e_entry（可能是高 DMW VMA 或低物理地址）映射到加载后的物理入口。

    AT() 链接脚本的 ELF 入口是低物理符号（如 __kernel_entry_phys = 0x9000_0000），
    段 VMA 却是高 DMW 地址（0x9000_0000_9000_0000），两者不能直接相减。

    - 高 DMW VMA（>= CACHED_BASE）：去掉窗口前缀得到物理地址
    - 低物理地址：直接使用

    再换算成相对 load_addr 的入口：load_addr + (entry_phys - lowest_paddr)。
    """
    entry_phys = (vma_entry & PHYS_MASK) if vma_entry >= CACHED_BASE else vma_entry
    return load_addr + (entry_phys - lowest_paddr)


def build_raw(elf_path, output_path, load_addr=None):
    """
    从 ELF 提取原始二进制，绕过 uImage 头部，直接用于 U-Boot 'go' 命令。

    用法:
      python3 scripts/elf-to-uimage.py --raw kernel-ls2k1000 kernel.bin

    U-Boot:
      => tftp 0x90000000 kernel.bin
      => go 0x90000000
    """
    print(f"[1/3] Reading ELF: {elf_path}")
    vma_entry, lowest_paddr, segments = read_elf_info(elf_path)

    if load_addr is None:
        load_addr = lowest_paddr

    entry_paddr = physical_entry_address(vma_entry, load_addr, lowest_paddr)

    print(f"    Load address : 0x{load_addr:016x}")
    print(f"    Entry point  : 0x{entry_paddr:016x}")

    print(f"[2/3] Extracting raw binary...")
    total_size = 0
    for s in segments:
        end = s['paddr'] - load_addr + s['memsz']
        if end > total_size:
            total_size = end

    raw = bytearray(total_size)
    with open(elf_path, 'rb') as f:
        for s in segments:
            offset = s['paddr'] - load_addr
            size = s['filesz']
            if size > 0:
                f.seek(s['offset'])
                data = f.read(size)
                raw[offset:offset + size] = data
                print(f"    segment: paddr=0x{s['paddr']:x} offset=0x{offset:x} size=0x{size:x}")

    print(f"    Binary size : {len(raw)} bytes ({len(raw) / 1024:.1f} KiB)")

    print(f"[3/3] Writing raw binary: {output_path}")
    with open(output_path, 'wb') as f:
        f.write(bytes(raw))

    print()
    print(f"✅ Raw binary created: {output_path}")
    print()
    print("U-Boot usage (go command):")
    print(f"  => tftp 0x{load_addr:08x} {os.path.basename(output_path)}")
    print(f"  => go 0x{entry_paddr:08x}")


def build_uimage(elf_path, output_path, load_addr=None, entry_addr=None, name="MyOS-2K1000", ih_arch=36):
    """
    将 LoongArch ELF 转换为 uImage。

    工作流程:
    1. 读取 ELF 获取物理地址和段布局
    2. 提取原始内核数据
    3. 构造 uImage 头部
    4. 写入 uImage 文件
    """
    # ── 1. 读取 ELF 信息 ──
    print(f"[1/4] Reading ELF: {elf_path}")
    vma_entry, lowest_paddr, segments = read_elf_info(elf_path)

    if load_addr is None:
        load_addr = lowest_paddr

    if entry_addr is None:
        entry_addr = physical_entry_address(vma_entry, load_addr, lowest_paddr)

    print(f"    Load address : 0x{load_addr:016x}")
    print(f"    Entry point  : 0x{entry_addr:016x}")
    print(f"    ELF VMA entry: 0x{vma_entry:016x}")

    # ── 2. 转换 ELF → 原始二进制 ──
    print(f"[2/4] Converting ELF → raw binary...")

    total_size = 0
    for s in segments:
        end = s['paddr'] - load_addr + s['memsz']
        if end > total_size:
            total_size = end

    raw = bytearray(total_size)
    with open(elf_path, 'rb') as f:
        for s in segments:
            offset = s['paddr'] - load_addr
            size = s['filesz']
            if size > 0:
                f.seek(s['offset'])
                data = f.read(size)
                raw[offset:offset + size] = data

    print(f"    Binary size : {total_size} bytes ({total_size / 1024:.1f} KiB)")

    # ── 3. 构造 uImage 头部 ──
    print(f"[3/4] Building uImage header (arch={ih_arch})...")

    image_data = bytes(raw)
    ih_size = len(image_data)
    ih_time = int(time.time())

    ih_os = IH_OS_LINUX
    ih_type = IH_TYPE_KERNEL
    ih_comp = IH_COMP_NONE

    # 先构建部分头部以计算 CRC
    header_partial = struct.pack(
        '>III I I I I BB BB',
        IH_MAGIC,
        0,  # ih_hcrc (稍后填充)
        ih_time,
        ih_size,
        load_addr,
        entry_addr,
        0,  # ih_dcrc (稍后填充)
        ih_os,
        ih_arch,
        ih_type,
        ih_comp,
    )
    name_bytes = name.encode('ascii', 'replace')[:31].ljust(32, b'\x00')

    # 数据 CRC32
    ih_dcrc = uimage_crc32(image_data)

    # 重新构建带有正确 CRC 的头部
    header = bytearray(struct.pack(
        '>III I I I I BB BB 32s',
        IH_MAGIC,
        0,  # ih_hcrc placeholder (填充后计算)
        ih_time,
        ih_size,
        load_addr,
        entry_addr,
        ih_dcrc,
        ih_os,
        ih_arch,
        ih_type,
        ih_comp,
        name_bytes,
    ))

    # 计算头部 CRC (ih_hcrc 字段先置零再计算)
    header[4:8] = b'\x00\x00\x00\x00'
    ih_hcrc = uimage_crc32(bytes(header))
    struct.pack_into('>I', header, 4, ih_hcrc)
    header = bytes(header)

    print(f"    Magic       : 0x{IH_MAGIC:08x}")
    arch_names = {26: 'RISC-V', 27: 'LoongArch (2022.04)', 6: 'MIPS64', 5: 'MIPS'}
    print(f"    Architecture: {ih_arch} ({arch_names.get(ih_arch, 'Unknown')})")
    print(f"    OS          : {ih_os} (Linux)")
    print(f"    Image type  : {ih_type} (Kernel)")
    print(f"    Compression : {ih_comp} (None)")
    print(f"    Name        : {name}")

    # ── 4. 写入 uImage ──
    print(f"[4/4] Writing uImage: {output_path}")

    with open(output_path, 'wb') as f:
        f.write(header)
        f.write(image_data)

    total_size = len(header) + len(image_data)
    print(f"    Total size  : {total_size} bytes ({total_size / 1024:.1f} KiB)")
    print()
    print(f"✅ uImage created: {output_path}")
    print()
    print("U-Boot usage:")
    print(f"  => tftp 0x{load_addr:08x} {os.path.basename(output_path)}")
    print(f"  => bootm 0x{load_addr:08x}")
    print()
    print("Note: if bootm reports 'Bad Linux LoongArch Image',")
    print("      retry with: python3 scripts/elf-to-uimage.py --arch mips64 kernel-ls2k1000")


def main():
    import argparse

    parser = argparse.ArgumentParser(
        description="Convert MyOS LoongArch ELF kernel to U-Boot uImage"
    )
    parser.add_argument('elf', help="Path to kernel ELF file")
    parser.add_argument('output', nargs='?', help="Output uImage path (default: <elf>.uImage)")
    parser.add_argument('-a', '--load-addr', help="Override load address (hex)")
    parser.add_argument('-e', '--entry', help="Override entry point (hex)")
    parser.add_argument('-n', '--name', default="MyOS-2K1000", help="Image name")
    parser.add_argument('--arch', default='loongarch',
                        choices=['riscv', 'loongarch', 'mips64', 'mips'],
                        help="Architecture: riscv (26), loongarch (27), mips64 (6), mips (5) [U-Boot 2022.04]")
    parser.add_argument('--raw', action='store_true',
                        help="Output raw binary instead of uImage (for U-Boot 'go' command)")

    args = parser.parse_args()

    elf_path = Path(args.elf)
    if not elf_path.is_file():
        print(f"Error: {elf_path} not found", file=sys.stderr)
        sys.exit(1)

    arch_map = {'riscv': 26, 'loongarch': 27, 'mips64': 6, 'mips': 5}
    ih_arch = arch_map[args.arch]

    load_addr = int(args.load_addr, 0) if args.load_addr else None
    entry_addr = int(args.entry, 0) if args.entry else None

    if args.raw:
        output_path = args.output or str(elf_path.with_suffix('.bin'))
        build_raw(str(elf_path), output_path, load_addr)
    else:
        output_path = args.output or str(elf_path.with_suffix('.uImage'))
        build_uimage(str(elf_path), output_path, load_addr, entry_addr, args.name, ih_arch)


if __name__ == '__main__':
    main()
