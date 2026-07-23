#!/usr/bin/env python3
import argparse
import os
import re
import signal
import subprocess
import sys
import time


def read_log(path):
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except FileNotFoundError:
        return ""


def has_complete_line(text, pattern):
    return pattern in text.splitlines()


def has_regex_line(text, pattern):
    regex = re.compile(pattern)
    return any(regex.fullmatch(line) for line in text.splitlines())


def evaluate_log(text, success_lines, success_regexes, failure_lines, failure_regexes):
    for pattern in failure_lines:
        if has_complete_line(text, pattern):
            return "failure", pattern
    for pattern in failure_regexes:
        if has_regex_line(text, pattern):
            return "failure", pattern

    lines_ok = all(has_complete_line(text, pattern) for pattern in success_lines)
    regexes_ok = all(has_regex_line(text, pattern) for pattern in success_regexes)
    if lines_ok and regexes_ok and (success_lines or success_regexes):
        return "success", None
    return "pending", None


def print_log_tail(path, count=80):
    lines = read_log(path).splitlines()
    print(f"qemu_log_wait.py: last {min(len(lines), count)} log lines:", file=sys.stderr)
    for line in lines[-count:]:
        print(line, file=sys.stderr)


def stop_process(process):
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    except PermissionError:
        process.terminate()
    try:
        process.wait(timeout=2.0)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    except PermissionError:
        process.kill()
    try:
        process.wait(timeout=2.0)
    except subprocess.TimeoutExpired:
        pass


def main():
    parser = argparse.ArgumentParser(description="run QEMU until serial-log conditions are met")
    parser.add_argument("--log", required=True)
    parser.add_argument("--pattern", action="append", default=[], help="legacy exact success line")
    parser.add_argument("--success-pattern", action="append", default=[])
    parser.add_argument("--success-regex", action="append", default=[])
    parser.add_argument("--failure-pattern", action="append", default=[])
    parser.add_argument("--failure-regex", action="append", default=[])
    parser.add_argument("--tail-lines", type=int, default=80)
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("missing command after --")

    success_lines = args.pattern + args.success_pattern
    if not success_lines and not args.success_regex:
        parser.error("at least one success condition is required")

    os.makedirs(os.path.dirname(args.log) or ".", exist_ok=True)
    try:
        os.unlink(args.log)
    except FileNotFoundError:
        pass

    process = subprocess.Popen(command, start_new_session=True)
    deadline = time.monotonic() + args.timeout
    result = "pending"
    matched_failure = None

    while time.monotonic() < deadline:
        text = read_log(args.log)
        result, matched_failure = evaluate_log(
            text,
            success_lines,
            args.success_regex,
            args.failure_pattern,
            args.failure_regex,
        )
        if result != "pending" or process.poll() is not None:
            break
        time.sleep(0.1)

    if result == "success":
        time.sleep(0.25)
        stop_process(process)
        final_result, _ = evaluate_log(
            read_log(args.log),
            success_lines,
            args.success_regex,
            args.failure_pattern,
            args.failure_regex,
        )
        if final_result == "success":
            return 0
        print("qemu_log_wait.py: success conditions disappeared after QEMU shutdown", file=sys.stderr)
        print_log_tail(args.log, args.tail_lines)
        return 1

    if result == "failure":
        stop_process(process)
        print(f"qemu_log_wait.py: matched failure condition {matched_failure!r}", file=sys.stderr)
        print_log_tail(args.log, args.tail_lines)
        return 2

    if process.poll() is None:
        stop_process(process)
        print("qemu_log_wait.py: timeout waiting for all success conditions", file=sys.stderr)
        print_log_tail(args.log, args.tail_lines)
        return 124

    rc = process.returncode
    print(f"qemu_log_wait.py: qemu exited before all success conditions (rc={rc})", file=sys.stderr)
    print_log_tail(args.log, args.tail_lines)
    return rc if rc else 1


if __name__ == "__main__":
    raise SystemExit(main())
