#!/usr/bin/env python3
"""
将 MyOS 内核 ELF 转换为 U-Boot 可启动的 uImage 格式。

用法:
    python3 scripts/elf-to-uimage.py kernel-ls2k1000 [output.uImage]
    python3 scripts/elf-to-uimage.py --platform ls2k1000 kernel-ls2k1000 kernel-ls2k1000.uImage
    python3 scripts/elf-to-uimage.py --raw kernel-ls2k1000 kernel.bin

两种生成路径:

1. 非 ls2k1000 平台（默认）: 纯 Python 构造 64 字节 legacy uImage 头。
   依赖: Python 3.6+ (仅标准库，无需 mkimage)。

2. ls2k1000 平台 (`--platform ls2k1000`): 交给厂商 U-Boot 的 mkimage 生成。
   LS2K1000 板载 U-Boot (2022.04-v2.1.0 BSP) 使用"位置枚举"，LoongArch 是
   IH_ARCH_LA = 27（紧跟 RISCV=26），只有厂商 mkimage 认识这个编号。
   脚本负责: ELF→kernel.bin → 校验(非空/32 位范围) → 调用厂商 mkimage
   → mkimage -l 回读校验。不再由 Python 手工拼 uImage 头。

   厂商 mkimage 定位优先级:
     LS2K1000_MKIMAGE 环境变量
     build/host-tools/ls2k1000/mkimage (由 scripts/build-ls2k1000-mkimage.sh 生成)

生成的 uImage 可以直接在 LS2K1000 U-Boot 中使用:
    fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
    iminfo  0x9000000002000000
    bootm   0x9000000002000000 - ${fdt_addr}
"""

import struct
import sys
import os
import time
import zlib
import tempfile
import subprocess
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

# legacy uImage 头的 load/entry 字段是 32 位
UINT32_MAX = 0xFFFFFFFF

# 厂商 mkimage 默认构建产物（scripts/build-ls2k1000-mkimage.sh）
DEFAULT_VENDOR_MKIMAGE = Path(__file__).resolve().parent.parent / \
    'build' / 'host-tools' / 'ls2k1000' / 'mkimage'


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

                # LS2K1000 `.nocache_ram` uncached DMW 窗口（0x8000_0000_0000_0000）：
                # 该段是 NOLOAD 内核保留 DMA 内存（VMA 在 uncached 窗口，LLD 还会为它
                # 造一个 ELF-header 孤儿 LOAD），都不是可加载内核内容，跳过。
                # 其余平台地址全部 < 0x8000_0000_0000_0000，此过滤是无操作。
                if phys_addr >= 0x8000_0000_0000_0000:
                    continue

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


def extract_raw_bytes(elf_path, load_addr=None):
    """
    从 ELF 提取内核原始二进制（去除段间空隙、按加载地址铺平）。

    返回 (raw, load_addr, entry_paddr)：
      raw         : 铺平后的二进制字节串
      load_addr   : 载荷应加载到的物理地址
      entry_paddr : 加载后的物理入口地址

    AT() 链接脚本下：最低 LOAD 段 AT 地址即 load；ELF e_entry 是低物理符号，
    physical_entry_address() 把它换算成相对 load 的物理入口。
    """
    vma_entry, lowest_paddr, segments = read_elf_info(elf_path)

    if load_addr is None:
        load_addr = lowest_paddr

    entry_paddr = physical_entry_address(vma_entry, load_addr, lowest_paddr)

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

    return bytes(raw), load_addr, entry_paddr


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
    raw, load_addr, entry_paddr = extract_raw_bytes(elf_path, load_addr)

    print(f"    Load address : 0x{load_addr:016x}")
    print(f"    Entry point  : 0x{entry_paddr:016x}")

    print(f"[2/3] Extracting raw binary...")
    print(f"    Binary size : {len(raw)} bytes ({len(raw) / 1024:.1f} KiB)")

    print(f"[3/3] Writing raw binary: {output_path}")
    with open(output_path, 'wb') as f:
        f.write(raw)

    print()
    print(f"✅ Raw binary created: {output_path}")
    print()
    print("U-Boot usage (go command):")
    print(f"  => tftp 0x{load_addr:08x} {os.path.basename(output_path)}")
    print(f"  => go 0x{entry_paddr:08x}")


def find_vendor_mkimage():
    """
    定位厂商 mkimage。优先级:
      1. LS2K1000_MKIMAGE 环境变量
      2. build/host-tools/ls2k1000/mkimage (构建脚本默认产物)
    返回绝对路径；找不到返回 None。
    """
    candidates = []
    env_mkimage = os.environ.get('LS2K1000_MKIMAGE')
    if env_mkimage:
        candidates.append(env_mkimage)
    candidates.append(str(DEFAULT_VENDOR_MKIMAGE))

    for c in candidates:
        p = Path(c)
        if p.is_file() and os.access(p, os.X_OK):
            return str(p)
    return None


def check_32bit(label, addr):
    """校验地址能放进 legacy uImage 的 32 位 load/entry 字段。"""
    if addr < 0 or addr > UINT32_MAX:
        raise ValueError(
            f"{label} address 0x{addr:x} does not fit in the 32-bit "
            f"legacy uImage header (IH_LOAD/IH_EP are uint32)"
        )


def check_vendor_image(mkimage, uimage_path, load_addr, entry_addr):
    """
    用同一个厂商 mkimage -l 回读校验产物。
    mkimage -l 会验证 magic / header CRC / data CRC，并打印内容。
    """
    proc = subprocess.run([mkimage, '-l', uimage_path],
                          capture_output=True, text=True)
    out = proc.stdout + proc.stderr

    checks = {
        'LoongArch in Image Type': 'LoongArch' in out,
        f'load == 0x{load_addr:08x}': f'Load Address: {load_addr:08x}' in out,
        f'entry == 0x{entry_addr:08x}': f'Entry Point: {entry_addr:08x}',
    }
    failures = [k for k, ok in checks.items() if not ok]

    if proc.returncode != 0 or failures:
        print("    vendor mkimage -l output:")
        for line in out.splitlines():
            print(f"      {line}")
        raise RuntimeError(
            "vendor mkimage readback check failed: "
            + (", ".join(failures) if failures else f"exit {proc.returncode}")
        )

    # mkimage -l 已校验两个 CRC；打印内容供人核对
    for line in out.splitlines():
        if line.startswith('Image Type') or line.startswith('Load Address') \
           or line.startswith('Entry Point') or line.startswith('Data Size'):
            print(f"    {line}")


def build_uimage_vendor(elf_path, output_path, load_addr=None, entry_addr=None,
                        name="SudoOS-LS2K1000"):
    """
    LS2K1000: 用厂商 mkimage 生成 uImage，不再手工拼头。

    流程: ELF → kernel.bin（临时文件） → 厂商 mkimage → mkimage -l 回读。
    """
    # ── 1. 定位厂商 mkimage ──
    mkimage = find_vendor_mkimage()
    if mkimage is None:
        raise RuntimeError(
            "vendor mkimage not found. Build it first with:\n"
            "    ./scripts/build-ls2k1000-mkimage.sh\n"
            "or point LS2K1000_MKIMAGE=<path> at an existing vendor mkimage."
        )
    print(f"    vendor mkimage: {mkimage}")

    # ── 2. ELF → raw 二进制 ──
    print(f"[1/4] Reading ELF: {elf_path}")
    raw, load_addr, entry_paddr = extract_raw_bytes(elf_path, load_addr)
    if entry_addr is None:
        entry_addr = entry_paddr
    print(f"    Load address : 0x{load_addr:016x}")
    print(f"    Entry point  : 0x{entry_addr:016x}")
    print(f"    Binary size  : {len(raw)} bytes ({len(raw) / 1024:.1f} KiB)")

    # ── 3. 校验 ──
    print(f"[2/4] Validating inputs...")
    if len(raw) == 0:
        raise ValueError("kernel payload is empty")
    check_32bit('load', load_addr)
    check_32bit('entry', entry_addr)
    print("    32-bit load/entry: OK")

    # ── 4. 调用厂商 mkimage ──
    print(f"[3/4] Running vendor mkimage (-A loongarch ...)...")
    with tempfile.NamedTemporaryFile(prefix='kernel-ls2k1000-',
                                     suffix='.bin', delete=False) as tf:
        tf.write(raw)
        data_path = tf.name
    try:
        cmd = [
            mkimage,
            '-A', 'loongarch',
            '-O', 'linux',
            '-T', 'kernel',
            '-C', 'none',
            '-a', f'0x{load_addr:x}',
            '-e', f'0x{entry_addr:x}',
            '-n', name,
            '-d', data_path,
            output_path,
        ]
        print(f"    cmd: {' '.join(cmd)}")
        proc = subprocess.run(cmd, capture_output=True, text=True)
        if proc.returncode != 0:
            # 失败时完整输出错误
            if proc.stdout:
                print(proc.stdout)
            if proc.stderr:
                print(proc.stderr, file=sys.stderr)
            raise RuntimeError(
                f"vendor mkimage failed with exit code {proc.returncode}"
            )
        # 打印 mkimage 的确认信息（包含校验和）
        if proc.stdout.strip():
            for line in proc.stdout.splitlines():
                print(f"    {line}")
    finally:
        os.unlink(data_path)

    # ── 5. 回读校验 ──
    print(f"[4/4] Verifying with vendor mkimage -l ...")
    check_vendor_image(mkimage, output_path, load_addr, entry_addr)

    total_size = os.path.getsize(output_path)
    print(f"    Total size  : {total_size} bytes ({total_size / 1024:.1f} KiB)")
    print()
    print(f"✅ uImage created: {output_path}")
    print()
    print("U-Boot usage (bootm, stash at low memory to avoid overlap):")
    print(f"  => fatload usb 0:1 0x9000000002000000 {os.path.basename(output_path)}")
    print(f"  => iminfo  0x9000000002000000")
    print(f"  => bootm   0x9000000002000000 - ${{fdt_addr}}")


def build_uimage(elf_path, output_path, load_addr=None, entry_addr=None, name="MyOS-2K1000", ih_arch=36):
    """
    将 LoongArch ELF 转换为 uImage（纯 Python 路径，供非 ls2k1000 平台使用）。

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
    parser.add_argument('--platform', default='auto', choices=['auto', 'ls2k1000'],
                        help="ls2k1000: use the vendor U-Boot mkimage (loongarch, arch=27); "
                             "auto: pure-Python header (default)")
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

    load_addr = int(args.load_addr, 0) if args.load_addr else None
    entry_addr = int(args.entry, 0) if args.entry else None

    if args.raw:
        output_path = args.output or str(elf_path.with_suffix('.bin'))
        build_raw(str(elf_path), output_path, load_addr)
        return

    output_path = args.output or str(elf_path.with_suffix('.uImage'))

    if args.platform == 'ls2k1000':
        # 厂商 mkimage 路径：loongarch 架构名由厂商工具自己解析，不用 arch_map
        build_uimage_vendor(str(elf_path), output_path, load_addr, entry_addr,
                            args.name)
    else:
        arch_map = {'riscv': 26, 'loongarch': 27, 'mips64': 6, 'mips': 5}
        ih_arch = arch_map[args.arch]
        build_uimage(str(elf_path), output_path, load_addr, entry_addr,
                     args.name, ih_arch)


if __name__ == '__main__':
    main()
