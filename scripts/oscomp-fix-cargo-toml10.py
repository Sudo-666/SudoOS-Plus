#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import re
import shutil
from pathlib import Path

SKIP_DIRS = {
    ".git",
    ".hg",
    ".svn",
    ".cargo",
    ".oscomp_patch_backup",
    "target",
    "build",
    "vendor/cargo",
}

ASSIGN_INLINE_RE = re.compile(
    r"^\s*(?:\"[^\"]+\"|'[^']+'|[A-Za-z0-9_.-]+)\s*=\s*\{"
)


def strip_comment_outside_strings(s: str) -> str:
    out = []
    in_str = False
    quote = ""
    escape = False
    i = 0
    while i < len(s):
        ch = s[i]
        if in_str:
            out.append(ch)
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == quote:
                in_str = False
                quote = ""
        else:
            if ch in ('"', "'"):
                in_str = True
                quote = ch
                out.append(ch)
            elif ch == "#":
                break
            else:
                out.append(ch)
        i += 1
    return "".join(out)


def brace_delta_outside_strings(s: str) -> int:
    delta = 0
    in_str = False
    quote = ""
    escape = False
    for ch in s:
        if in_str:
            if escape:
                escape = False
            elif ch == "\\":
                escape = True
            elif ch == quote:
                in_str = False
                quote = ""
        else:
            if ch in ('"', "'"):
                in_str = True
                quote = ch
            elif ch == "{":
                delta += 1
            elif ch == "}":
                delta -= 1
    return delta


def compact_inline_table(block: list[str]) -> str:
    if not block:
        return ""
    indent = re.match(r"^(\s*)", block[0]).group(1)
    parts: list[str] = []
    for raw in block:
        clean = strip_comment_outside_strings(raw.rstrip("\n")).strip()
        if clean:
            parts.append(clean)
    text = " ".join(parts)
    text = re.sub(r"\s+", " ", text)
    text = re.sub(r"\{\s+", "{ ", text)
    text = re.sub(r"\s+\}", " }", text)
    text = re.sub(r"\[\s+", "[", text)
    text = re.sub(r"\s+\]", "]", text)
    # TOML 1.0 does not allow trailing commas in inline tables.  Remove trailing
    # commas before both inline-table and array closers for maximum compatibility.
    while True:
        new = re.sub(r",\s*([}\]])", r"\1", text)
        if new == text:
            break
        text = new
    # Cosmetic cleanup around array commas after whitespace normalization.
    text = re.sub(r"\s+,", ",", text)
    text = re.sub(r",\s+", ", ", text)
    return indent + text + "\n"


def transform_toml(path: Path) -> tuple[bool, int]:
    original = path.read_text(encoding="utf-8")
    lines = original.splitlines(keepends=True)
    out: list[str] = []
    changed_blocks = 0
    i = 0
    while i < len(lines):
        line = lines[i]
        clean = strip_comment_outside_strings(line)
        if ASSIGN_INLINE_RE.match(clean) and brace_delta_outside_strings(clean) > 0:
            block: list[str] = []
            depth = 0
            start = i
            while i < len(lines):
                block.append(lines[i])
                depth += brace_delta_outside_strings(strip_comment_outside_strings(lines[i]))
                i += 1
                if depth <= 0:
                    break
            if depth == 0:
                out.append(compact_inline_table(block))
                changed_blocks += 1
            else:
                # Malformed input; preserve it exactly so the user can inspect it.
                out.extend(block)
                print(f"WARN: unmatched inline table starting at {path}:{start + 1}")
            continue
        out.append(line)
        i += 1

    new_text = "".join(out)
    if new_text != original:
        backup = path.with_suffix(path.suffix + ".toml10.bak")
        if not backup.exists():
            shutil.copy2(path, backup)
        path.write_text(new_text, encoding="utf-8")
        return True, changed_blocks
    return False, 0


def is_skipped(path: Path) -> bool:
    parts = path.parts
    for skip in SKIP_DIRS:
        skip_parts = tuple(skip.split("/"))
        for i in range(0, len(parts) - len(skip_parts) + 1):
            if tuple(parts[i : i + len(skip_parts)]) == skip_parts:
                return True
    return False


def find_cargo_tomls(root: Path) -> list[Path]:
    result: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        dpath = Path(dirpath)
        dirnames[:] = [d for d in dirnames if not is_skipped(dpath / d)]
        if "Cargo.toml" in filenames and not is_skipped(dpath):
            result.append(dpath / "Cargo.toml")
    return sorted(result)


def main() -> int:
    parser = argparse.ArgumentParser(description="Normalize Cargo.toml multi-line inline tables for TOML 1.0 Cargo parsers.")
    parser.add_argument("root", nargs="?", default=".", help="repository root")
    parser.add_argument("--check", action="store_true", help="fail if any file would be changed")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    changed: list[tuple[Path, int]] = []
    for cargo_toml in find_cargo_tomls(root):
        if args.check:
            before = cargo_toml.read_text(encoding="utf-8")
            lines = before.splitlines(keepends=True)
            # Reuse transform logic on a temporary copy would be overkill; do a
            # conservative detection here.
            depth = 0
            suspect = False
            for line in lines:
                clean = strip_comment_outside_strings(line)
                if depth == 0 and ASSIGN_INLINE_RE.match(clean) and brace_delta_outside_strings(clean) > 0:
                    suspect = True
                    break
            if suspect:
                print(f"NEEDS-FIX {cargo_toml.relative_to(root)}")
                changed.append((cargo_toml, 0))
            continue
        did_change, blocks = transform_toml(cargo_toml)
        if did_change:
            changed.append((cargo_toml, blocks))

    if changed:
        for path, blocks in changed:
            try:
                display = path.relative_to(root)
            except ValueError:
                display = path
            if args.check:
                print(display)
            else:
                print(f"fixed {display} ({blocks} inline table block(s))")
        return 1 if args.check else 0
    print("Cargo.toml TOML-1.0 check: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
