#!/usr/bin/env python3
"""Validate a real-hardware contest fixture boot log (CodePlan C10).

Checks the common contest storage markers plus the board-specific markers,
and rejects known failure signatures.

Usage:
    check-board-contest-log.py --board visionfive2 --fixture vf2-contest.log
    check-board-contest-log.py --board ls2k1000 --fixture ls2k-contest.log

Exit 0 = PASS, 1 = FAIL, 2 = usage error.
"""

import argparse
import sys

COMMON_REQUIRED = (
    "CONTEST00",
    "CONTEST01",
    "CONTEST02",
    "CONTEST03",
    "FIXTURE_OSCOMP_PASS",
    "SMOKE_TEST: PASS",
)

BOARD_REQUIRED = {
    "visionfive2": (
        "VF2-TF00",
        "VF2-TF01",
        "VF2-TF02",
        "VF2-TF03",
    ),
    "ls2k1000": (
        "LS2K-RAMDISK00",
        "LS2K-RAMDISK01",
        "registered=/dev/ram0",
    ),
}

REJECT_MARKERS = (
    "panicked",
    "panic",
    "timeout",
    "CRC error",
    "out of range",
    "filesystem corrupt",
    "unhandled trap",
    "OOM",
)


def main():
    parser = argparse.ArgumentParser(description="validate a contest fixture boot log")
    parser.add_argument("--board", required=True, choices=("visionfive2", "ls2k1000"))
    parser.add_argument("--fixture", required=True, help="serial log of the fixture boot")
    args = parser.parse_args()

    try:
        with open(args.fixture, "r", errors="replace") as handle:
            text = handle.read()
    except OSError as error:
        print(f"board contest check: FAIL cannot read log: {error}")
        return 1

    failed = 0

    for marker in COMMON_REQUIRED + BOARD_REQUIRED[args.board]:
        if marker in text:
            print(f"board contest check:   {marker} -> ok")
        else:
            print(f"board contest check:   {marker} -> MISSING")
            failed += 1

    rejected = [marker for marker in REJECT_MARKERS if marker.lower() in text.lower()]
    for marker in rejected:
        print(f"board contest check: reject marker {marker!r} present")
        failed += 1

    if failed:
        print(f"board contest check: FAIL ({failed} problems) board={args.board}")
        return 1

    print(f"board contest check: PASS board={args.board}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
