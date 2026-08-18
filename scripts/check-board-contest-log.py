#!/usr/bin/env python3
"""Validate a real-hardware contest boot log (CodePlan C10, hardened K2.2).

The kernel now carries an explicit result protocol:
- fixture mode: `FIXTURE_OSCOMP_PASS` only when
  `CONTEST_FIXTURE: paths-missing=0`
- final mode:   `CONTEST_RESULT mode=<mode> <pass|fail|timeout>` per runner,
  printed only when the judge script actually exited 0

Checks the common contest markers (in order), the board-specific markers,
the mode-appropriate result markers, and rejects known failure signatures.
A missing / non-zero / signal / timeout result is FAIL.

Usage:
    check-board-contest-log.py --board visionfive2 --image-type fixture --fixture vf2-contest.log
    check-board-contest-log.py --board ls2k1000 --image-type final --fixture ls2k-contest.log

Exit 0 = PASS, 1 = FAIL, 2 = usage error.
"""

import argparse
import re
import sys

COMMON_MARKERS = ("CONTEST00", "CONTEST01", "CONTEST02", "CONTEST03")

BOARD_REQUIRED = {
    "visionfive2": ("VF2-TF00", "VF2-TF01", "VF2-TF02", "VF2-TF03"),
    "ls2k1000": ("LS2K-RAMDISK00", "LS2K-RAMDISK01", "registered=/dev/ram0"),
}

# Final-image runner modes whose CONTEST_RESULT must be `pass`.
FINAL_RESULT_MODES = ("final-cagent", "final-buildstorm")

# Benign occurrences stripped before the generic timeout scan: the OS COMP
# SUMMARY's `timeout=N` counter and CONTEST_RESULT timeout verdicts (the
# latter are judged by the dedicated CONTEST_RESULT scan).
REAL_TIMEOUT_SUB = (
    (re.compile(r"timeout=\d+"), ""),
    (re.compile(r"CONTEST_RESULT mode=\S+ timeout"), ""),
)

REJECT_MARKERS = (
    "panicked",
    "panic",
    "timeout",
    "CRC error",
    "out of range",
    "filesystem corrupt",
    "unhandled trap",
    "OOM",
    "FIXTURE_OSCOMP_FAIL",
    # Early-boot heap use (allocation before BOOT06 heap-ready).  riscv64 trips
    # the default __rdl_oom "memory allocation of N bytes failed"; ls2k1000
    # spins in the HEAP_FATAL-* handler instead.
    "memory allocation of",
    "HEAP_FATAL",
)


def markers_in_order(text, markers):
    """Return True if every marker appears and they do so in the given order."""
    last = -1
    for marker in markers:
        pos = text.find(marker)
        if pos == -1 or pos < last:
            return False
        last = pos
    return True


def main():
    parser = argparse.ArgumentParser(description="validate a contest boot log")
    parser.add_argument("--board", required=True, choices=("visionfive2", "ls2k1000"))
    parser.add_argument("--fixture", required=True, help="serial log of the contest boot")
    parser.add_argument(
        "--image-type",
        default="fixture",
        choices=("fixture", "final"),
        help="fixture (generated ext4 probe) or final (official judge image)",
    )
    args = parser.parse_args()

    try:
        with open(args.fixture, "r", errors="replace") as handle:
            text = handle.read()
    except OSError as error:
        print(f"board contest check: FAIL cannot read log: {error}")
        return 1

    failed = 0

    # 1) Common markers, in CONTEST00 -> 01 -> 02 -> 03 order.
    if markers_in_order(text, COMMON_MARKERS):
        for marker in COMMON_MARKERS:
            print(f"board contest check:   {marker} -> ok")
    else:
        for marker in COMMON_MARKERS:
            if marker in text:
                print(f"board contest check:   {marker} -> out of order")
            else:
                print(f"board contest check:   {marker} -> MISSING")
            failed += 1

    # 2) Board-specific markers.
    for marker in BOARD_REQUIRED[args.board]:
        if marker in text:
            print(f"board contest check:   {marker} -> ok")
        else:
            print(f"board contest check:   {marker} -> MISSING")
            failed += 1

    # 3) Mode-appropriate result markers.
    if args.image_type == "fixture":
        if "FIXTURE_OSCOMP_PASS" in text:
            print("board contest check:   FIXTURE_OSCOMP_PASS -> ok")
        else:
            print("board contest check:   FIXTURE_OSCOMP_PASS -> MISSING")
            failed += 1
        match = re.search(r"CONTEST_FIXTURE: paths-missing=(\d+)", text)
        if match and int(match.group(1)) == 0:
            print("board contest check:   paths-missing=0 -> ok")
        else:
            value = match.group(1) if match else "missing"
            print(f"board contest check:   paths-missing=0 -> FAIL (value={value})")
            failed += 1
    else:  # final
        for mode in FINAL_RESULT_MODES:
            if f"CONTEST_RESULT mode={mode} pass" in text:
                print(f"board contest check:   CONTEST_RESULT {mode} pass -> ok")
            else:
                print(f"board contest check:   CONTEST_RESULT {mode} pass -> MISSING")
                failed += 1
        if "final-image-contract:" in text:
            missing_paths = re.findall(r"^\s+(\S+)\s*=\s*missing", text, re.MULTILINE)
            if missing_paths:
                print(
                    "board contest check:   final-image-contract missing paths: "
                    + ", ".join(missing_paths)
                )
                failed += 1
            else:
                print("board contest check:   final-image-contract all present -> ok")
        else:
            print("board contest check:   final-image-contract -> MISSING")
            failed += 1

    # 4) Boot-complete signal.
    if "SMOKE_TEST: PASS" in text:
        print("board contest check:   SMOKE_TEST: PASS -> ok")
    else:
        print("board contest check:   SMOKE_TEST: PASS -> MISSING")
        failed += 1

    # 5) Any non-pass CONTEST_RESULT verdict is a failure (both image types).
    for match in re.finditer(r"CONTEST_RESULT mode=(\S+) (fail|timeout)", text):
        mode, verdict = match.group(1), match.group(2)
        print(f"board contest check:   CONTEST_RESULT {mode} {verdict} -> FAIL")
        failed += 1

    # 6) Generic failure signatures (real timeouts only, not the timeout=N
    #    summary counter and not CONTEST_RESULT timeout verdicts).
    scanned = text
    for pattern, replacement in REAL_TIMEOUT_SUB:
        scanned = pattern.sub(replacement, scanned)
    scanned_lower = scanned.lower()
    for marker in REJECT_MARKERS:
        if marker.lower() in scanned_lower:
            print(f"board contest check: reject marker {marker!r} present")
            failed += 1

    if failed:
        print(
            f"board contest check: FAIL ({failed} problems) "
            f"board={args.board} image-type={args.image_type}"
        )
        return 1

    print(f"board contest check: PASS board={args.board} image-type={args.image_type}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
