#!/usr/bin/env python3
"""Run quick/full/soak M5 gates and record the exact commit."""

from __future__ import annotations
import argparse, json, os, shlex, subprocess, sys, time
from datetime import datetime, timezone
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parent.parent


def stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT_DIR, text=True, stdout=subprocess.PIPE, check=True).stdout.strip()


def run(command: list[str], log: Path):
    started = time.monotonic()
    print("$", shlex.join(command))
    with log.open("w") as output:
        process = subprocess.Popen(command, cwd=ROOT_DIR, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, bufsize=1)
        assert process.stdout is not None
        for line in process.stdout:
            sys.stdout.write(line)
            output.write(line)
        code = process.wait()
    return {"command": command, "returncode": code, "seconds": round(time.monotonic() - started, 6), "log": str(log)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--level", choices=("quick", "full", "soak"), default="quick")
    parser.add_argument("--require-clean", action="store_true")
    args = parser.parse_args()
    head = git("rev-parse", "HEAD")
    clean = not git("status", "--porcelain")
    if args.require_clean and not clean:
        raise RuntimeError("release verification requires a clean worktree")
    report_dir = ROOT_DIR / "build/m5" / f"{stamp()}-{head[:12]}-{args.level}"
    report_dir.mkdir(parents=True)
    started, steps = time.monotonic(), []

    static = (
        ["make", "check", "harness-test"]
        if args.level in {"full", "soak"}
        else ["make", "source-tree-check", "fmt-check", "test", "harness-test", "build-riscv64", "build-loongarch64"]
    )
    steps.append(run(static, report_dir / "static.log"))

    stress_base = [
        sys.executable, str(ROOT_DIR / "scripts/stress-smp.py"),
        "--arches", "riscv64", "loongarch64",
        "--keep-going", "--fail-on-flaky",
    ]

    if steps[-1]["returncode"] == 0 and args.level == "quick":
        steps.append(run(
            stress_base + [
                "--log-root", str(report_dir / "stress-quick"),
                "--smps", "1", "4",
                "--mems", "256M",
                "--profiles", "debug",
                "--loops", "1",
            ],
            report_dir / "stress-quick.log",
        ))

    if steps[-1]["returncode"] == 0 and args.level in {"full", "soak"}:
        steps.append(run(
            stress_base + [
                "--log-root", str(report_dir / "stress-full"),
                "--smps", "1", "2", "4", "8",
                "--mems", "64M", "256M", "1G",
                "--profiles", "debug", "release",
                "--loops", "1",
            ],
            report_dir / "stress-full.log",
        ))

    if steps[-1]["returncode"] == 0 and args.level == "soak":
        steps.append(run(
            stress_base + [
                "--log-root", str(report_dir / "stress-soak-debug"),
                "--smps", "4",
                "--mems", "256M",
                "--profiles", "debug",
                "--loops", os.environ.get("M5_SOAK_LOOPS", "200"),
            ],
            report_dir / "stress-soak-debug.log",
        ))

    if steps[-1]["returncode"] == 0 and args.level == "soak":
        steps.append(run(
            stress_base + [
                "--log-root", str(report_dir / "stress-soak-release"),
                "--smps", "4",
                "--mems", "256M",
                "--profiles", "release",
                "--loops", os.environ.get("M5_RELEASE_SOAK_LOOPS", "20"),
            ],
            report_dir / "stress-soak-release.log",
        ))

    status = "pass" if all(step["returncode"] == 0 for step in steps) else "fail"
    report = {
        "schema_version": 1, "status": status, "level": args.level,
        "git_head": head, "worktree_clean": clean,
        "total_seconds": round(time.monotonic() - started, 6),
        "report_dir": str(report_dir), "steps": steps,
    }
    (report_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    (report_dir / "report.md").write_text(
        "# SudoOS M5 verification\n\n"
        f"- status: **{status}**\n- level: `{args.level}`\n"
        f"- head: `{head}`\n- clean: `{clean}`\n"
    )
    latest = ROOT_DIR / "build/m5/latest.txt"
    latest.parent.mkdir(parents=True, exist_ok=True)
    latest.write_text(str(report_dir) + "\n")
    print("M5 report:", report_dir / "report.md")
    return 0 if status == "pass" else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"m5 verify: error: {error}", file=sys.stderr)
        raise SystemExit(1) from error
