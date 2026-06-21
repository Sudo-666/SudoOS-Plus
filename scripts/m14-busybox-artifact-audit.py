#!/usr/bin/env python3
from __future__ import annotations

import stat
import sys
from pathlib import Path


def parse_newc(path: Path) -> dict[str, tuple[int, int, bytes]]:
    data = path.read_bytes()
    off = 0
    out: dict[str, tuple[int, int, bytes]] = {}
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
        out[name] = (mode, filesize, payload)
    return out


def check_source(root: Path) -> list[tuple[str, bool, str]]:
    py = root / "scripts/build-static-busybox-initramfs.py"
    sh = root / "scripts/build-static-busybox-initramfs.sh"
    make = root / "Makefile"
    checks = []
    py_text = py.read_text() if py.exists() else ""
    sh_text = sh.read_text() if sh.exists() else ""
    make_text = make.read_text() if make.exists() else ""
    checks.append(("python newc builder", py.exists() and "070701" in py_text and "TRAILER!!!" in py_text, "rootless deterministic cpio builder exists"))
    checks.append(("static busybox guard", "likely_dynamic" in py_text and "DYNAMIC_MARKERS" in py_text, "rejects likely dynamic BusyBox by default"))
    checks.append(("shell wrapper", sh.exists() and "BUSYBOX" in sh_text and "build-static-busybox-initramfs.py" in sh_text, "Make-friendly wrapper exists"))
    checks.append(("make target", "busybox-initramfs" in make_text and "m14-busybox-artifact-audit" in make_text, "targets wired"))
    return checks


def check_archive(path: Path) -> list[tuple[str, bool, str]]:
    try:
        entries = parse_newc(path)
    except Exception as exc:
        return [("cpio parse", False, str(exc))]
    checks = []
    checks.append(("cpio parse", True, f"{len(entries)} entries"))
    checks.append(("/init symlink", "init" in entries and stat.S_ISLNK(entries["init"][0]) and entries["init"][2] == b"bin/busybox", "init handoff exists"))
    checks.append(("/bin/busybox", "bin/busybox" in entries and stat.S_ISREG(entries["bin/busybox"][0]) and entries["bin/busybox"][1] > 0, "busybox binary present"))
    checks.append(("/bin/sh applet", "bin/sh" in entries and stat.S_ISLNK(entries["bin/sh"][0]), "shell applet symlink present"))
    checks.append(("basic dirs", all(d in entries and stat.S_ISDIR(entries[d][0]) for d in ["dev", "proc", "sys", "tmp", "etc", "bin"]), "runtime dirs present"))
    return checks


def main() -> int:
    root = Path.cwd()
    checks = check_source(root)
    if len(sys.argv) > 1:
        checks.extend(check_archive(Path(sys.argv[1])))
    passed = sum(1 for _, ok, _ in checks if ok)
    failed = len(checks) - passed
    for name, ok, detail in checks:
        print(f"{'PASS' if ok else 'FAIL'} {name:24} {detail}")
    print(f"M14 BusyBox artifact audit: PASS={passed} FAIL={failed}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
