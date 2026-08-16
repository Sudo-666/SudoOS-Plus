#!/usr/bin/env python3
"""Validate a generated contest ext4 fixture (scripts/make-contest-fixture.sh).

Checks the image is the expected raw ext4 size, opens it with the repo's
ext4 reader (which validates the superblock magic), and verifies the paths
the oscomp runners depend on are present with the expected markers.

Usage:
    check-contest-fixture.py --arch riscv64 --image build/fixtures/contest-rv.ext4
"""

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ext4_read import Ext4, Ext4Error  # noqa: E402

FIXTURE_MIB = 32


def read_str(fs, path):
    return fs.read_path(path).decode("utf-8", errors="replace")


def main():
    parser = argparse.ArgumentParser(description="validate a contest ext4 fixture")
    parser.add_argument("--arch", required=True, choices=("riscv64", "loongarch64"))
    parser.add_argument("--image", required=True)
    args = parser.parse_args()

    size = os.path.getsize(args.image)
    expected = FIXTURE_MIB * 1024 * 1024
    if size != expected:
        print(f"fixture check: FAIL size={size} expected={expected}")
        return 1

    try:
        fs = Ext4(args.image)
    except Ext4Error as error:
        print(f"fixture check: FAIL bad ext4 superblock: {error}")
        return 1

    checks = []

    def require(path, needle):
        try:
            content = read_str(fs, path)
            ok = needle in content
        except Ext4Error as error:
            ok = False
            content = f"(error: {error})"
        checks.append((path, needle, ok, content))

    require("/SUDOOS_CONTEST_FIXTURE", "SUDOOS_CONTEST_FIXTURE")
    require("/arch", args.arch)
    require("/glibc/cagent_testcode.sh", "FIXTURE_OSCOMP_PASS")
    require("/musl/cagent_testcode.sh", "FIXTURE_OSCOMP_PASS")
    require("/work/tgoskits/Cargo.toml", "[package]")

    failed = 0
    for path, needle, ok, content in checks:
        status = "ok" if ok else "MISSING"
        if not ok:
            failed += 1
        print(f"fixture check:   {path} -> {status} (need {needle!r})")
        if not ok:
            print(f"fixture check:   {content!r}")

    if failed:
        print(f"fixture check: FAIL ({failed} missing)")
        return 1

    print(f"fixture check: PASS arch={args.arch}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
