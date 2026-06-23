#!/usr/bin/env python3
from __future__ import annotations

import os
import re
import sys
from pathlib import Path
from typing import Dict, Iterable, List, Set, Tuple

ROOT = Path.cwd().resolve()

SKIP_DIR_NAMES = {
    ".git",
    ".hg",
    ".svn",
    ".oscomp_patch_backup",
    "target",
    "build",
}

# Keep registry-vendored crates immutable. Path dependencies such as
# vendor/virtio-drivers and vendor/fdt-reader are intentionally NOT skipped.
SKIP_SUBSTRINGS = {
    f"{os.sep}vendor{os.sep}cargo{os.sep}",
    f"{os.sep}.cargo{os.sep}",
}

FEATURE_LINE_RE = re.compile(r"^\s*#!\s*\[\s*feature\s*\(([^)]*)\)\s*\]\s*(?://.*)?$")
LET_CHAIN_RE = re.compile(r"(?:&&|\|\|)\s*let\s+")
IS_MULTIPLE_RE = re.compile(r"\.\s*is_multiple_of\s*\(")


def should_skip(path: Path) -> bool:
    rel = path.resolve()
    parts = set(rel.parts)
    if parts & SKIP_DIR_NAMES:
        return True
    text = str(rel)
    return any(s in text for s in SKIP_SUBSTRINGS)


def iter_files(suffix: str) -> Iterable[Path]:
    for path in ROOT.rglob(f"*{suffix}"):
        if not should_skip(path):
            yield path


def split_features(raw: str) -> Set[str]:
    out: Set[str] = set()
    for part in raw.split(','):
        item = part.strip()
        if not item:
            continue
        # Remove accidental trailing comments / attributes fragments.
        item = item.split('//', 1)[0].strip()
        if item:
            out.add(item)
    return out


def collect_feature_lines(text: str) -> Set[str]:
    features: Set[str] = set()
    for line in text.splitlines():
        m = FEATURE_LINE_RE.match(line)
        if m:
            features.update(split_features(m.group(1)))
    return features


def remove_feature_lines(text: str) -> Tuple[str, int]:
    lines = text.splitlines(keepends=True)
    new_lines: List[str] = []
    removed = 0
    for line in lines:
        if FEATURE_LINE_RE.match(line.rstrip('\n')):
            removed += 1
            continue
        new_lines.append(line)
    return ''.join(new_lines), removed


def parse_lib_path(cargo_toml: Path) -> str | None:
    """Very small TOML parser for [lib] path = "..." only."""
    section = None
    try:
        lines = cargo_toml.read_text(encoding='utf-8').splitlines()
    except UnicodeDecodeError:
        lines = cargo_toml.read_text(errors='ignore').splitlines()
    for raw in lines:
        line = raw.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('[') and line.endswith(']'):
            section = line.strip('[]').strip()
            continue
        if section == 'lib' and line.startswith('path') and '=' in line:
            rhs = line.split('=', 1)[1].strip()
            if rhs.startswith(('"', "'")):
                quote = rhs[0]
                end = rhs.find(quote, 1)
                if end > 1:
                    return rhs[1:end]
    return None


def find_crate_roots() -> Dict[Path, Path]:
    roots: Dict[Path, Path] = {}
    for cargo in iter_files('.toml'):
        if cargo.name != 'Cargo.toml':
            continue
        crate_dir = cargo.parent
        explicit = parse_lib_path(cargo)
        candidates: List[Path] = []
        if explicit:
            candidates.append(crate_dir / explicit)
        candidates.append(crate_dir / 'src' / 'lib.rs')
        candidates.append(crate_dir / 'src' / 'main.rs')
        for candidate in candidates:
            if candidate.exists() and candidate.is_file() and not should_skip(candidate):
                roots[crate_dir.resolve()] = candidate.resolve()
                break
    return roots


def crate_rs_files(crate_dir: Path) -> List[Path]:
    src = crate_dir / 'src'
    if not src.exists():
        return []
    files: List[Path] = []
    for path in src.rglob('*.rs'):
        if not should_skip(path):
            files.append(path.resolve())
    return files


def needed_features_for_crate(files: Iterable[Path]) -> Set[str]:
    needed: Set[str] = set()
    for path in files:
        try:
            text = path.read_text(encoding='utf-8')
        except UnicodeDecodeError:
            text = path.read_text(errors='ignore')
        if IS_MULTIPLE_RE.search(text):
            needed.add('unsigned_is_multiple_of')
        if LET_CHAIN_RE.search(text):
            needed.add('let_chains')
    return needed


def insert_root_features(root_file: Path, features: Set[str]) -> bool:
    if not features:
        return False
    text = root_file.read_text(encoding='utf-8')
    # Remove old root feature lines again in case they were present in the root.
    text, _ = remove_feature_lines(text)
    prefix = ''.join(f"#![feature({name})]\n" for name in sorted(features))
    new_text = prefix + text.lstrip('\ufeff')
    if new_text != root_file.read_text(encoding='utf-8'):
        root_file.write_text(new_text, encoding='utf-8')
        return True
    return False


def feature_lines_inside_multiline_attrs(path: Path) -> List[int]:
    """Detect #![feature] inside a previous unclosed multiline #![... attr."""
    issues: List[int] = []
    depth = 0
    active_attr = False
    for lineno, line in enumerate(path.read_text(encoding='utf-8', errors='ignore').splitlines(), start=1):
        stripped = line.strip()
        if active_attr and FEATURE_LINE_RE.match(stripped):
            issues.append(lineno)
        # A simple bracket depth tracker for inner attrs.
        if stripped.startswith('#!['):
            active_attr = True
            depth = 0
        if active_attr:
            depth += line.count('[') + line.count('(')
            depth -= line.count(']') + line.count(')')
            if depth <= 0:
                active_attr = False
                depth = 0
    return issues


def root_has_feature(root: Path, feature: str) -> bool:
    text = root.read_text(encoding='utf-8', errors='ignore')
    return feature in collect_feature_lines(text)


def audit(roots: Dict[Path, Path]) -> int:
    failures: List[str] = []
    for path in iter_files('.rs'):
        bad_lines = feature_lines_inside_multiline_attrs(path)
        if bad_lines:
            failures.append(f"feature gate embedded in multiline attr: {path.relative_to(ROOT)} lines {bad_lines}")

    for crate_dir, root in sorted(roots.items(), key=lambda kv: str(kv[0])):
        files = crate_rs_files(crate_dir)
        needed = needed_features_for_crate(files)
        for feature in sorted(needed):
            if not root_has_feature(root, feature):
                failures.append(
                    f"crate {crate_dir.relative_to(ROOT)} needs feature({feature}) but root {root.relative_to(ROOT)} lacks it"
                )

    if failures:
        print('[oscomp-feature-gate-repair-audit] FAIL')
        for item in failures:
            print('  -', item)
        return 1
    print('[oscomp-feature-gate-repair-audit] PASS')
    return 0


def main() -> int:
    roots = find_crate_roots()
    if not roots:
        print('ERROR: no Rust crate roots found', file=sys.stderr)
        return 2

    # Preserve crate-root feature gates before global cleanup.
    preserved: Dict[Path, Set[str]] = {}
    for root in roots.values():
        preserved[root] = collect_feature_lines(root.read_text(encoding='utf-8', errors='ignore'))

    removed_total = 0
    cleaned_files = 0
    for path in iter_files('.rs'):
        text = path.read_text(encoding='utf-8', errors='ignore')
        new_text, removed = remove_feature_lines(text)
        if removed:
            removed_total += removed
            cleaned_files += 1
            path.write_text(new_text, encoding='utf-8')

    changed_roots = 0
    for crate_dir, root in roots.items():
        features = set(preserved.get(root, set()))
        features.update(needed_features_for_crate(crate_rs_files(crate_dir)))
        if insert_root_features(root, features):
            changed_roots += 1

    print(f'[oscomp-feature-gate-repair] crate roots scanned: {len(roots)}')
    print(f'[oscomp-feature-gate-repair] feature lines removed: {removed_total} from {cleaned_files} files')
    print(f'[oscomp-feature-gate-repair] crate roots updated: {changed_roots}')
    return audit(roots)


if __name__ == '__main__':
    raise SystemExit(main())
