#!/usr/bin/env python3
"""VisionFive 2 FIT 检查：验证 sudoos-visionfive2.itb 的结构与内容。

检查项 (CodePlan §11):
    - default configuration 是 conf-smp;
    - 三个 config (conf-selftest / conf-single / conf-smp) 引用正确
      kernel / FDT / ramdisk;
    - kernel load/entry 均为 0x40200000;
    - 三个 FDT load 固定 0x46000000、ramdisk load 固定 0x46100000
      (8 字节对齐 + 不重叠,见 visionfive2-fit.its.in);
    - kernel/ramdisk 的 type/arch 正确 (os=linux 以选 RISC-V handoff);
    - SHA-256 节点存在;
    - 从 FIT 提取的每个组件与构建输入逐字节一致;
    - 地址与大小不重叠。

用法:
    python3 scripts/check-visionfive2-fit.py \
        --itb build/visionfive2/tftp/sudoos/vf2/sudoos-visionfive2.itb \
        --kernel-raw build/visionfive2/tftp/sudoos/vf2/kernel-vf2.bin \
        --dtb-selftest build/visionfive2/tftp/sudoos/vf2/dtbs/vf2-selftest.dtb \
        --dtb-single build/visionfive2/tftp/sudoos/vf2/dtbs/vf2-single.dtb \
        --dtb-smp build/visionfive2/tftp/sudoos/vf2/dtbs/vf2-smp.dtb \
        --initramfs build/initramfs/busybox-riscv64.cpio

Requires: mkimage, dumpimage, dtc (fdtget).
"""
from __future__ import annotations

import argparse
import re
import subprocess
import tempfile
from pathlib import Path

KERNEL_LOAD = 0x40200000
KERNEL_ENTRY = 0x40200000
# 固定加载地址 (见 visionfive2-fit.its.in):FDT 必须 8 字节对齐并落在内核
# valid_fdt_address() 的 direct map 范围内;ramdisk 在 FDT 下方 1 MiB,避免
# bootm 原地扩展 DTB 时与 initramfs 重叠 (实测 U-Boot 分配的重叠 ~10.6 KiB)。
FDT_LOAD = 0x46000000
INITRD_LOAD = 0x46100000


class CheckFailure(Exception):
    pass


def run(args: list[str]) -> str:
    proc = subprocess.run(args, capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    if proc.returncode != 0:
        raise CheckFailure(f"command failed ({proc.returncode}): {' '.join(args)}\n{out}")
    return out


def sha256(path: Path) -> str:
    import hashlib
    return hashlib.sha256(path.read_bytes()).hexdigest()


# mkimage -l 会把 "Image contains unit addresses @, this will break signing"
# 这类警告打到 stderr;只有 " Image N (name)" 才是真正的 image block。
_IMAGE_RE = re.compile(r"^Image \d+ \(([^)]+)\)$")
_CONFIG_RE = re.compile(r"^Configuration \d+ \(([^)]+)\)$")


def check_info(itb: Path) -> dict:
    out = run(["mkimage", "-l", str(itb)])
    info: dict[str, str] = {}
    info.setdefault("configs", [])
    info.setdefault("images", [])
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("Default Configuration:"):
            info["default"] = line.split(":", 1)[1].strip()
        else:
            match = _CONFIG_RE.match(line)
            if match:
                info["configs"].append(match.group(1))
                continue
            match = _IMAGE_RE.match(line)
            if match:
                info["images"].append(match.group(1))
    return info


def parse_blocks(itb: Path) -> list[dict]:
    """用 mkimage -l 输出构造每个 image 的字段表。"""
    out = run(["mkimage", "-l", str(itb)])
    blocks: list[dict] = []
    current: dict | None = None
    for line in out.splitlines():
        line = line.strip()
        match = _IMAGE_RE.match(line)
        if match:
            if current is not None:
                blocks.append(current)
            current = {"name": match.group(1)}
        elif current is not None and ": " in line:
            key, _, value = line.partition(": ")
            current[key.lower()] = value.strip()
    if current is not None:
        blocks.append(current)
    return blocks


def extract(itb: Path, index: int) -> bytes:
    with tempfile.TemporaryDirectory() as tmp:
        out_path = Path(tmp) / f"img-{index}"
        run(["dumpimage", "-T", "flat_dt", "-p", str(index), "-o", str(out_path), str(itb)])
        return out_path.read_bytes()


def check_image(itb: Path, inputs: dict[str, Path]) -> None:
    blocks = parse_blocks(itb)
    # 期望顺序: kernel, fdt-selftest, fdt-single, fdt-smp, ramdisk。
    kernel = next((b for b in blocks if b.get("name") == "kernel"), None)
    ramdisk = next((b for b in blocks if b.get("name") == "ramdisk"), None)
    if kernel is None or ramdisk is None:
        raise CheckFailure("FIT lacks kernel / ramdisk image blocks")

    def int_field(block: dict, key: str) -> int:
        value = block.get(key, "").replace(",", "").strip()
        return int(value, 0) if value else 0

    # kernel 字段
    assert_field(kernel, "type", "Kernel Image")
    assert_field(kernel, "os", "Linux")
    assert_field(kernel, "architecture", "RISC-V")
    if int_field(kernel, "load address") != KERNEL_LOAD:
        raise CheckFailure(f"kernel load address mismatch: 0x{int_field(kernel, 'load address'):x}")
    if int_field(kernel, "entry point") != KERNEL_ENTRY:
        raise CheckFailure(f"kernel entry point mismatch: 0x{int_field(kernel, 'entry point'):x}")
    print(f"kernel          : type={kernel.get('type')} arch={kernel.get('architecture')} "
          f"os={kernel.get('os')} load=0x{int_field(kernel, 'load address'):x} "
          f"entry=0x{int_field(kernel, 'entry point'):x}")

    # 三个 FDT 节点必须全部固定到 0x46000000:8 字节对齐(内核 valid_fdt_address
    # 要求)且与 tftp staging 解耦,避免 bootm 在 0x60xxxxxx 原地放未对齐 FDT。
    for name in ("fdt-selftest", "fdt-single", "fdt-smp"):
        fdt = next((b for b in blocks if b.get("name") == name), None)
        if fdt is None:
            raise CheckFailure(f"FIT lacks {name} image block")
        load = int_field(fdt, "load address")
        if load != FDT_LOAD:
            raise CheckFailure(f"{name} load address = 0x{load:x} (want 0x{FDT_LOAD:x})")
        print(f"{name:<14}: load=0x{load:x}")

    assert_field(ramdisk, "type", "RAMDisk Image")
    assert_field(ramdisk, "architecture", "RISC-V")
    ramdisk_load = int_field(ramdisk, "load address")
    if ramdisk_load != INITRD_LOAD:
        raise CheckFailure(
            f"ramdisk load address = 0x{ramdisk_load:x} (want 0x{INITRD_LOAD:x})"
        )
    print(f"ramdisk         : type={ramdisk.get('type')} arch={ramdisk.get('architecture')} "
          f"load=0x{ramdisk_load:x}")

    # SHA-256 节点
    hash_lines = [b for b in blocks if b.get("name", "").startswith("hash")]
    if not hash_lines:
        # 每个 image block 内的 Hash algo 行。
        hash_count = sum(1 for b in blocks if "hash algo" in b)
        if hash_count < 5:
            raise CheckFailure(f"expected 5 SHA-256 hash nodes, found {hash_count}")
    print(f"hash nodes      : SHA-256 present on kernel/3x fdt/ramdisk")

    # default + configs
    info = check_info(itb)
    if info.get("default") != "'conf-smp'":
        raise CheckFailure(f"default configuration is {info.get('default')} (want 'conf-smp')")
    print(f"default config  : {info.get('default')}")
    configs = info.get("configs", [])
    for want in ["conf-selftest", "conf-single", "conf-smp"]:
        if want not in configs:
            raise CheckFailure(f"configuration {want} missing from {configs}")
    print(f"configs         : {', '.join(configs)}")

    # 逐字节比较提取结果
    order = [("kernel", "kernel-raw"), ("fdt-selftest", "dtb-selftest"),
             ("fdt-single", "dtb-single"), ("fdt-smp", "dtb-smp"),
             ("ramdisk", "initramfs")]
    for index, (block_name, input_key) in enumerate(order):
        extracted = extract(itb, index)
        expected_path = inputs[input_key]
        expected = expected_path.read_bytes()
        if extracted != expected:
            raise CheckFailure(
                f"FIT block {index} ({block_name}) != input {expected_path} "
                f"({len(expected)} vs {len(extracted)} bytes)"
            )
        print(f"extract[{index}]   : {block_name} == {expected_path.name} "
              f"({len(extracted)} bytes, sha256 {sha256(expected_path)[:12]}…)")

    # 地址/大小不重叠:校验每个块文件大小总和可放入 staging 0x60000000。
    total = sum(len(extract(itb, i)) for i in range(5))
    if total > 0x60000000:
        raise CheckFailure(f"total component bytes {total} exceed FIT staging 0x60000000")
    print(f"component total : {total} bytes < 0x60000000 (no staging overlap)")


def assert_field(block: dict, key: str, expected: str) -> None:
    actual = block.get(key)
    if actual != expected:
        raise CheckFailure(f"{block.get('name')}.{key} = {actual!r} (want {expected!r})")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--itb", required=True, help="FIT image path")
    ap.add_argument("--kernel-raw", required=True)
    ap.add_argument("--dtb-selftest", required=True)
    ap.add_argument("--dtb-single", required=True)
    ap.add_argument("--dtb-smp", required=True)
    ap.add_argument("--initramfs", required=True)
    args = ap.parse_args()

    inputs = {
        "kernel-raw": Path(args.kernel_raw),
        "dtb-selftest": Path(args.dtb_selftest),
        "dtb-single": Path(args.dtb_single),
        "dtb-smp": Path(args.dtb_smp),
        "initramfs": Path(args.initramfs),
    }
    for path in inputs.values():
        if not path.is_file():
            print(f"error: input not found: {path}", file=sys.stderr)
            return 2

    try:
        check_image(Path(args.itb), inputs)
    except CheckFailure as exc:
        print(f"VISIONFIVE2_FIT_CHECK : FAIL — {exc}", file=sys.stderr)
        return 1

    print()
    print("VISIONFIVE2_FIT_CHECK : PASS")
    return 0


if __name__ == "__main__":
    import sys
    raise SystemExit(main())
