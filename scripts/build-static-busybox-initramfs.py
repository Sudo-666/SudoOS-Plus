#!/usr/bin/env python3
"""Build a deterministic rootless newc initramfs for a static BusyBox.

The builder is intentionally self-contained for macOS hosts:
- no GNU cpio dependency
- no root or mknod dependency
- deterministic inode/mtime ordering
"""
from __future__ import annotations

import argparse
import os
import stat
import struct
import sys
from dataclasses import dataclass
from pathlib import Path

DEFAULT_APPLETS = [
    "sh", "ash", "cat", "echo", "ls", "mkdir", "mount", "umount", "dmesg",
    "pwd", "cd", "test", "true", "false", "sleep", "env", "printenv", "id",
    "uname", "date", "touch", "rm", "rmdir", "ln", "cp", "mv", "chmod",
    "chown", "sync", "dd", "hexdump", "more", "grep", "sed", "awk", "ps",
    "kill", "killall", "free", "df", "du", "hostname", "which", "readlink",
    "basename", "dirname", "expr", "printf", "wc", "head", "tail", "sort",
    "stty", # termios acceptance tool: `stty -a` verifies the console baud/etc.
    # PID 1 / admin applets: /init runs busybox with argv[0]=/init, and the
    # /sbin/{init,reboot,poweroff,halt} symlinks dispatch the same applets.
    "init", "reboot", "poweroff", "halt",
]

# BusyBox init(1) reads /etc/inittab. The sysinit action emits the readiness
# marker the board log greps for; askfirst attaches an interactive ash to the
# console; restart keeps init alive after a shell exits.
INITTAB = (
    b"::sysinit:/bin/echo SUDOOS_INIT_READY\n"
    b"::askfirst:-/bin/sh\n"
    b"::restart:/sbin/init\n"
)

# /etc/profile is sourced by ash login shells. PATH must cover every applet
# symlink the kernel installs; PS1 identifies the sudoos root shell.
PROFILE = (
    b"export PATH=/bin:/sbin:/usr/bin:/usr/sbin\n"
    b"export HOME=/root\n"
    b"export TERM=vt100\n"
    b"export PS1='sudoos:/# '\n"
)

DYNAMIC_MARKERS = [
    b"/lib/ld-musl-", b"/lib64/ld-linux", b"/lib/ld-linux", b"ld.so.1", b"PT_INTERP",
]


def align4(data: bytes) -> bytes:
    return data + (b"\0" * ((4 - (len(data) % 4)) % 4))


@dataclass(frozen=True)
class Entry:
    name: str
    mode: int
    data: bytes = b""
    link: str = ""
    uid: int = 0
    gid: int = 0
    nlink: int = 1
    rdevmajor: int = 0
    rdevminor: int = 0

    @property
    def filesize(self) -> int:
        if stat.S_ISLNK(self.mode):
            return len(self.link.encode())
        return len(self.data)

    @property
    def payload(self) -> bytes:
        if stat.S_ISLNK(self.mode):
            return self.link.encode()
        return self.data


def hex8(value: int) -> bytes:
    if value < 0 or value > 0xFFFFFFFF:
        raise ValueError(f"newc field out of range: {value}")
    return f"{value:08x}".encode()


def emit_entry(entry: Entry, ino: int) -> bytes:
    name = entry.name.lstrip("/")
    if not name:
        raise ValueError("empty cpio path")
    name_b = name.encode() + b"\0"
    fields = [
        b"070701",
        hex8(ino),
        hex8(entry.mode),
        hex8(entry.uid),
        hex8(entry.gid),
        hex8(entry.nlink),
        hex8(0),  # deterministic mtime
        hex8(entry.filesize),
        hex8(0),  # devmajor
        hex8(0),  # devminor
        hex8(entry.rdevmajor),
        hex8(entry.rdevminor),
        hex8(len(name_b)),
        hex8(0),  # check
    ]
    header = b"".join(fields)
    assert len(header) == 110
    return align4(header + name_b) + align4(entry.payload)


def emit_archive(entries: list[Entry]) -> bytes:
    seen: set[str] = set()
    out = bytearray()
    for ino, entry in enumerate(entries, start=1):
        key = entry.name.strip("/")
        if key in seen:
            raise ValueError(f"duplicate cpio path: {entry.name}")
        seen.add(key)
        out.extend(emit_entry(entry, ino))
    out.extend(emit_entry(Entry("TRAILER!!!", stat.S_IFREG | 0o644), len(entries) + 1))
    return bytes(out)


def dir_entry(path: str, mode: int = 0o755) -> Entry:
    return Entry(path, stat.S_IFDIR | mode, nlink=2)


def file_entry(path: str, data: bytes, mode: int = 0o644) -> Entry:
    return Entry(path, stat.S_IFREG | mode, data=data)


def symlink_entry(path: str, target: str) -> Entry:
    return Entry(path, stat.S_IFLNK | 0o777, link=target)


def likely_dynamic(binary: bytes) -> bool:
    return any(marker in binary for marker in DYNAMIC_MARKERS)


def build_entries(busybox: Path, applets: list[str]) -> list[Entry]:
    data = busybox.read_bytes()
    dirs = [
        "bin", "sbin", "usr", "usr/bin", "usr/sbin", "etc", "dev", "proc", "sys",
        "tmp", "mnt", "root", "var", "var/tmp", "run",
    ]
    entries = [dir_entry(d, 0o1777 if d in {"tmp", "var/tmp"} else 0o755) for d in dirs]
    entries.extend([
        file_entry("bin/busybox", data, 0o755),
        symlink_entry("init", "bin/busybox"),
        file_entry("etc/passwd", b"root:x:0:0:root:/root:/bin/sh\n"),
        file_entry("etc/group", b"root:x:0:\n"),
        file_entry("etc/inittab", INITTAB, 0o644),
        file_entry("etc/profile", PROFILE, 0o644),
        # /sbin admin applets used by init(1) reboot/poweroff paths. The link
        # target is relative so the archive is position-independent.
        symlink_entry("sbin/init", "../bin/busybox"),
        symlink_entry("sbin/reboot", "../bin/busybox"),
        symlink_entry("sbin/poweroff", "../bin/busybox"),
        symlink_entry("sbin/halt", "../bin/busybox"),
    ])

    seen = {"busybox"}
    for applet in applets:
        applet = applet.strip().strip("/")
        if not applet or applet in seen or "/" in applet:
            continue
        seen.add(applet)
        entries.append(symlink_entry(f"bin/{applet}", "busybox"))
    return entries


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--busybox", required=True, help="path to a static BusyBox binary")
    ap.add_argument("--out", default="build/initramfs/busybox.cpio")
    ap.add_argument("--applets", default="", help="optional comma-separated applet list")
    ap.add_argument("--allow-dynamic", action="store_true", help="do not reject binaries that look dynamically linked")
    args = ap.parse_args()

    busybox = Path(args.busybox).expanduser().resolve()
    if not busybox.exists() or not busybox.is_file():
        print(f"error: BUSYBOX is not a file: {busybox}", file=sys.stderr)
        return 2
    if not os.access(busybox, os.X_OK):
        print(f"error: BUSYBOX is not executable: {busybox}", file=sys.stderr)
        return 2

    binary = busybox.read_bytes()
    if not binary.startswith(b"\x7fELF"):
        print(f"error: BUSYBOX is not an ELF binary: {busybox}", file=sys.stderr)
        return 2
    if likely_dynamic(binary) and not args.allow_dynamic:
        print("error: BUSYBOX appears dynamically linked; use a static busybox for M14/M16 smoke", file=sys.stderr)
        print("       pass --allow-dynamic only for packaging experiments, not for kernel smoke", file=sys.stderr)
        return 2

    applets = DEFAULT_APPLETS
    if args.applets:
        applets = [x.strip() for x in args.applets.split(",") if x.strip()]

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    archive = emit_archive(build_entries(busybox, applets))
    out.write_bytes(archive)
    print(f"busybox initramfs: {out} ({len(archive)} bytes)")
    print("entry: /init -> /bin/busybox")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
