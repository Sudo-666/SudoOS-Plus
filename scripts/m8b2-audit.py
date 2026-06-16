#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
USER = ROOT / "mm/src/user_space.rs"
DOC = ROOT / "docs/m8b-demand-fault.md"

checks = [
    ("fault plan enum", "pub enum UserFaultPlan"),
    ("demand-fault planner", "pub fn plan_user_fault"),
    ("stack fault path", "UserFaultPlan::GrowStack"),
    ("anonymous page plan", "UserFaultPlan::MapAnonymous"),
    ("post-install TLB", "pub fn plan_post_install_tlb"),
    ("bounded defaults", "DEFAULT_STACK_GUARD_GAP"),
    ("COW explicit", "CopyOnWriteUnsupported"),
]

text = USER.read_text(encoding="utf-8") if USER.is_file() else ""
missing = [name for name, marker in checks if marker not in text]
if not DOC.is_file():
    missing.append("documentation")

if missing:
    print("M8-B2 audit: FAIL")
    for item in missing:
        print(f"  missing: {item}")
    raise SystemExit(1)

print("M8-B2 audit: PASS")
for name, _ in checks:
    print(f"  {name:<22}: present")
print("  hardware integration    : supplied by M8-B3/B4")
