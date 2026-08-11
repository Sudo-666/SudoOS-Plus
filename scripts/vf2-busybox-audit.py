#!/usr/bin/env python3
"""VisionFive 2 BusyBox initramfs 审计。

在 m14 通用审计之外补充 RISC-V / VF2 特有的检查:

- BusyBox 是 ELF64 little-endian, e_machine = EM_RISCV (243);
- 静态链接,无 PT_INTERP;
- cpio 为 deterministic newc(070701 magic,所有 mtime = 0);
- Gate 所需 applet(cat/sh/sleep/ps/stty/kill)存在。

用法:
    python3 scripts/vf2-busybox-audit.py \
        --busybox vendor/userland/riscv64/busybox-static \
        --cpio build/initramfs/busybox-riscv64.cpio
"""
from __future__ import annotations

import argparse
import stat
import struct
import sys
from pathlib import Path

EM_RISCV = 243
PT_INTERP = 3
REQUIRED_APPLETS = ["cat", "sh", "sleep", "ps", "stty", "kill"]


class AuditFailure(Exception):
    pass


def check_elf(path: Path) -> None:
    data = path.read_bytes()
    if data[:4] != b"\x7fELF":
        raise AuditFailure(f"{path}: not an ELF binary")
    if data[4] != 2:
        raise AuditFailure(f"{path}: not ELF64 (class={data[4]})")
    if data[5] != 1:
        raise AuditFailure(f"{path}: not little-endian (data={data[5]})")

    e_machine = struct.unpack_from("<H", data, 18)[0]
    if e_machine != EM_RISCV:
        raise AuditFailure(
            f"{path}: e_machine={e_machine} (want EM_RISCV={EM_RISCV})"
        )
    print(f"busybox machine : EM_RISCV ({EM_RISCV})")

    e_phoff = struct.unpack_from("<Q", data, 32)[0]
    e_phentsize = struct.unpack_from("<H", data, 54)[0]
    e_phnum = struct.unpack_from("<H", data, 56)[0]

    interp = False
    for index in range(e_phnum):
        phoff = e_phoff + index * e_phentsize
        if phoff + e_phentsize > len(data):
            raise AuditFailure(f"{path}: program header {index} out of range")
        p_type = struct.unpack_from("<I", data, phoff)[0]
        if p_type == PT_INTERP:
            interp = True
    if interp:
        raise AuditFailure(f"{path}: has a PT_INTERP (dynamically linked)")
    print("busybox linkage : static (no PT_INTERP)")


def parse_newc(data: bytes) -> dict[str, tuple[int, int, bytes]]:
    out: dict[str, tuple[int, int, bytes]] = {}
    off = 0
    while True:
        if off + 110 > len(data):
            raise ValueError("truncated cpio header")
        header = data[off:off + 110]
        if header[:6] != b"070701":
            raise ValueError(f"bad cpio magic at offset {off}")
        off += 110

        def field(i: int) -> int:
            start = 6 + i * 8
            return int(header[start:start + 8], 16)

        mode = field(1)
        mtime = field(3)
        filesize = field(6)
        namesize = field(11)

        if off + namesize > len(data):
            raise ValueError("truncated cpio name")
        name = data[off:off + namesize - 1].decode()
        off += namesize
        off = (off + 3) & ~3

        if off + filesize > len(data):
            raise ValueError("truncated cpio payload")
        payload = data[off:off + filesize]
        off += filesize
        off = (off + 3) & ~3

        if name == "TRAILER!!!":
            break
        out[name] = (mode, mtime, payload)
    return out


def check_cpio(path: Path) -> None:
    data = path.read_bytes()
    try:
        entries = parse_newc(data)
    except Exception as exc:
        raise AuditFailure(f"cpio parse failed: {exc}") from exc

    print(f"cpio entries    : {len(entries)}")

    # deterministic newc: 所有 mtime 必须为 0。
    if any(mtime != 0 for _, mtime, _ in entries.values()):
        raise AuditFailure("cpio is not deterministic (non-zero mtime)")
    print("cpio determinism: newc, all mtime=0")

    def symlink_to_busybox(name: str) -> bool:
        entry = entries.get(name)
        return (
            entry is not None
            and stat.S_ISLNK(entry[0])
            and entry[2] in (b"busybox", b"bin/busybox", b"../bin/busybox")
        )

    if not symlink_to_busybox("init"):
        raise AuditFailure("/init is not a symlink to busybox")
    print("cpio /init      : -> bin/busybox")

    inittab = entries.get("etc/inittab")
    if inittab is None or b"SUDOOS_INIT_READY" not in inittab[2] or b"askfirst" not in inittab[2]:
        raise AuditFailure("/etc/inittab lacks SUDOOS_INIT_READY / askfirst")
    print("cpio inittab    : SUDOOS_INIT_READY + askfirst")

    profile = entries.get("etc/profile")
    if profile is None or b"PS1=" not in profile[2] or b"${PWD}" not in profile[2]:
        raise AuditFailure("/etc/profile lacks dynamic ${PWD} PS1")
    print("cpio profile    : dynamic ${PWD} PS1")

    missing = [a for a in REQUIRED_APPLETS if not symlink_to_busybox(f"bin/{a}")]
    if missing:
        raise AuditFailure(f"missing Gate applet symlinks: {missing}")
    print(f"cpio applets    : {', '.join(REQUIRED_APPLETS)} present")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--busybox", required=True, help="static BusyBox binary path")
    ap.add_argument("--cpio", required=True, help="built initramfs cpio path")
    args = ap.parse_args()

    busybox = Path(args.busybox)
    cpio = Path(args.cpio)
    if not busybox.is_file():
        print(f"error: busybox not found: {busybox}", file=sys.stderr)
        return 2
    if not cpio.is_file():
        print(f"error: cpio not found: {cpio}", file=sys.stderr)
        return 2

    try:
        check_elf(busybox)
        check_cpio(cpio)
    except AuditFailure as exc:
        print(f"VF2_BUSYBOX_AUDIT : FAIL — {exc}", file=sys.stderr)
        return 1

    print()
    print("VF2_BUSYBOX_AUDIT : PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
