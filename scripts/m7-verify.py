#!/usr/bin/env python3
"""Run reproducible M7 quick/full/soak/release gates."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def utc_stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def git(*args: str, required: bool = True) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if required and result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"git {' '.join(args)} failed")
    return result.stdout.strip()


def worktree_fingerprint() -> str:
    status = git("status", "--porcelain=v1", "--untracked-files=all")
    return hashlib.sha256(status.encode()).hexdigest()


def run(command: list[str], log: Path) -> dict[str, object]:
    started = time.monotonic()
    print("$", shlex.join(command), flush=True)
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w", encoding="utf-8") as output:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=1,
        )
        assert process.stdout is not None
        for line in process.stdout:
            sys.stdout.write(line)
            sys.stdout.flush()
            output.write(line)
            output.flush()
        code = process.wait()
    return {
        "command": command,
        "returncode": code,
        "seconds": round(time.monotonic() - started, 6),
        "log": str(log),
    }


def stress_command(
    report_dir: Path,
    name: str,
    *,
    smps: tuple[str, ...],
    mems: tuple[str, ...],
    profiles: tuple[str, ...],
    loops: str,
) -> list[str]:
    return [
        sys.executable,
        str(ROOT / "scripts/stress-smp.py"),
        "--arches",
        "riscv64",
        "loongarch64",
        "--smps",
        *smps,
        "--mems",
        *mems,
        "--profiles",
        *profiles,
        "--loops",
        loops,
        "--keep-going",
        "--fail-on-flaky",
        "--log-root",
        str(report_dir / name),
    ]


def append_step(
    steps: list[dict[str, object]],
    command: list[str],
    log: Path,
) -> bool:
    if steps and steps[-1]["returncode"] != 0:
        return False
    steps.append(run(command, log))
    return steps[-1]["returncode"] == 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--level",
        choices=("quick", "full", "soak", "release"),
        default="quick",
    )
    parser.add_argument("--require-clean", action="store_true")
    args = parser.parse_args()

    head = git("rev-parse", "HEAD")
    branch = git("branch", "--show-current", required=False) or "detached"
    clean = not git("status", "--porcelain=v1", "--untracked-files=all")
    require_clean = args.require_clean or args.level == "release"
    if require_clean and not clean:
        raise RuntimeError("M7 release verification requires a clean worktree")

    report_dir = ROOT / "build/m7" / f"{utc_stamp()}-{head[:12]}-{args.level}"
    report_dir.mkdir(parents=True, exist_ok=False)

    started = time.monotonic()
    steps: list[dict[str, object]] = []

    append_step(
        steps,
        [
            sys.executable,
            str(ROOT / "scripts/m7-audit.py"),
            "--json",
            str(report_dir / "audit.json"),
        ],
        report_dir / "audit.log",
    )
    append_step(steps, ["make", "check"], report_dir / "check.log")

    if args.level == "quick":
        append_step(
            steps,
            stress_command(
                report_dir,
                "matrix-quick",
                smps=("1", "4"),
                mems=("256M",),
                profiles=("debug", "release"),
                loops="1",
            ),
            report_dir / "matrix-quick.log",
        )

    if args.level in {"full", "release"}:
        append_step(
            steps,
            stress_command(
                report_dir,
                "matrix-full",
                smps=("1", "2", "4", "8"),
                mems=("64M", "256M", "1G"),
                profiles=("debug", "release"),
                loops="1",
            ),
            report_dir / "matrix-full.log",
        )

    if args.level in {"soak", "release"}:
        append_step(
            steps,
            stress_command(
                report_dir,
                "soak-debug",
                smps=("4",),
                mems=("256M",),
                profiles=("debug",),
                loops=os.environ.get("M7_SOAK_LOOPS", "50"),
            ),
            report_dir / "soak-debug.log",
        )
        append_step(
            steps,
            stress_command(
                report_dir,
                "soak-release",
                smps=("4",),
                mems=("256M",),
                profiles=("release",),
                loops=os.environ.get("M7_RELEASE_SOAK_LOOPS", "10"),
            ),
            report_dir / "soak-release.log",
        )

    status = "pass" if steps and all(step["returncode"] == 0 for step in steps) else "fail"
    report = {
        "schema_version": 1,
        "milestone": "M7-B",
        "status": status,
        "level": args.level,
        "git_head": head,
        "git_branch": branch,
        "worktree_clean": clean,
        "worktree_fingerprint": worktree_fingerprint(),
        "total_seconds": round(time.monotonic() - started, 6),
        "report_dir": str(report_dir),
        "soak_loops": int(os.environ.get("M7_SOAK_LOOPS", "50")),
        "release_soak_loops": int(os.environ.get("M7_RELEASE_SOAK_LOOPS", "10")),
        "steps": steps,
    }

    (report_dir / "report.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (report_dir / "report.md").write_text(
        "# SudoOS M7-B verification\n\n"
        f"- status: **{status}**\n"
        f"- level: `{args.level}`\n"
        f"- head: `{head}`\n"
        f"- branch: `{branch}`\n"
        f"- clean: `{clean}`\n"
        f"- duration: `{report['total_seconds']}s`\n",
        encoding="utf-8",
    )

    latest = ROOT / "build/m7/latest.txt"
    latest.parent.mkdir(parents=True, exist_ok=True)
    latest.write_text(str(report_dir) + "\n", encoding="utf-8")

    print("M7 report:", report_dir / "report.md")
    return 0 if status == "pass" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError, ValueError) as error:
        print(f"m7 verify: error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
