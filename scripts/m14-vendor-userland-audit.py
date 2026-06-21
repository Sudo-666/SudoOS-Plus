#!/usr/bin/env python3
"""Audit vendor/userland BusyBox ELF artifacts for SudoOS M14.

This is intentionally host-independent: it parses ELF headers directly instead of
requiring readelf/file, so it works on macOS runners as well as Linux.

Default policy:
  * riscv64 static BusyBox is required for the first QEMU userland artifact.
  * loongarch64 static BusyBox is reported as WARN when absent, because current
    QEMU LoongArch bring-up may not expose virtio-mmio/rootfs yet.
  * --strict requires both arches and is intended for the final dual-arch gate.
"""
from __future__ import annotations

import argparse
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

EM = {
    "riscv64": 243,       # EM_RISCV
    "loongarch64": 258,   # EM_LOONGARCH
}
REQUIRED_BY_DEFAULT = {"riscv64"}
PT_INTERP = 3


@dataclass
class ElfReport:
    arch: str
    path: Path
    exists: bool
    ok: bool
    status: str
    detail: str


def read_elf(path: Path) -> tuple[int, bool, bool, str]:
    data = path.read_bytes()
    if len(data) < 64 or data[:4] != b"\x7fELF":
        return 0, False, False, "not an ELF file"
    if data[4] != 2:
        return 0, False, False, "not ELF64"
    if data[5] != 1:
        return 0, False, False, "not little-endian ELF"
    e_machine = struct.unpack_from("<H", data, 18)[0]
    e_phoff = struct.unpack_from("<Q", data, 32)[0]
    e_phentsize = struct.unpack_from("<H", data, 54)[0]
    e_phnum = struct.unpack_from("<H", data, 56)[0]
    has_interp = False
    if e_phoff and e_phentsize >= 56:
        end = e_phoff + e_phentsize * e_phnum
        if end > len(data):
            return e_machine, True, False, "program header table exceeds file size"
        for index in range(e_phnum):
            off = e_phoff + index * e_phentsize
            p_type = struct.unpack_from("<I", data, off)[0]
            if p_type == PT_INTERP:
                has_interp = True
                break
    executable = bool(path.stat().st_mode & 0o111)
    return e_machine, executable, has_interp, "ok"


def audit_arch(root: Path, arch: str, strict: bool) -> ElfReport:
    path = root / "vendor" / "userland" / arch / "busybox-static"
    required = strict or arch in REQUIRED_BY_DEFAULT
    if not path.exists():
        status = "FAIL" if required else "WARN"
        return ElfReport(arch, path, False, not required, status, "missing vendor BusyBox artifact")
    try:
        machine, executable, has_interp, detail = read_elf(path)
    except OSError as exc:
        return ElfReport(arch, path, True, False, "FAIL", f"unable to read ELF: {exc}")
    expected = EM[arch]
    problems: list[str] = []
    if detail != "ok":
        problems.append(detail)
    if machine != expected:
        problems.append(f"wrong e_machine={machine}, expected {expected}")
    if has_interp:
        problems.append("has PT_INTERP; M14 requires static BusyBox")
    if not executable:
        problems.append("not executable; run chmod +x on the artifact")
    if problems:
        return ElfReport(arch, path, True, False, "FAIL", "; ".join(problems))
    return ElfReport(arch, path, True, True, "PASS", f"ELF64 static e_machine={machine}, executable")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--strict", action="store_true", help="require both riscv64 and loongarch64 artifacts")
    parser.add_argument("--root", default=".")
    args = parser.parse_args()
    root = Path(args.root).resolve()

    reports = [audit_arch(root, arch, args.strict) for arch in ("riscv64", "loongarch64")]
    counts = {"PASS": 0, "WARN": 0, "FAIL": 0}
    for report in reports:
        counts[report.status] += 1
        rel = report.path.relative_to(root) if report.path.is_absolute() else report.path
        print(f"{report.status:<5} {report.arch:<12} {rel} -- {report.detail}")

    print(
        "M14 vendor userland audit: "
        f"PASS={counts['PASS']} WARN={counts['WARN']} FAIL={counts['FAIL']}"
    )
    if counts["FAIL"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
