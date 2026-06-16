#!/usr/bin/env python3
"""M8-B4 demand paging and VM syscall structural audit."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]

CHECKS = {
    "mm/src/vma.rs": (
        "#[derive(Clone)]\npub struct VmAreaSet",
        "pub fn remove_range(&mut self, range: VirtRange)",
        "pub fn protect_range(",
        "The update is transactional",
        "pub fn remove_kind(&mut self, kind: VmAreaKind)",
        "previous.range().end() == area.range().start()",
        "munmap_splits_and_accepts_holes_transactionally",
        "mprotect_splits_full_coverage_and_rejects_gaps",
    ),
    "mm/src/address_space.rs": (
        "#[derive(Clone)]\npub struct AddressSpace",
        "pub fn set_program_break_and_sync_heap(",
        "let old_areas = self.areas.clone();",
        "pub fn unmap_range(&mut self, range: VirtRange)",
        "pub fn protect_range(",
    ),
    "kernel/src/user_mm.rs": (
        "pub enum UserFaultResolution",
        "pub fn resolve_user_fault(",
        "UserFaultPlan::MapAnonymous",
        "UserFaultPlan::GrowStack",
        "fn retire_range_locked(",
        "fn finish_retirement(",
        "crate::tlb::shootdown_user_local(request);",
        "pub fn set_program_break(",
        "pub fn map_anonymous(",
        "pub fn unmap_range(",
        "pub fn protect_range(",
        "let old_layout = state.core.layout().clone();",
        "mprotect rollback could not restore a leaf PTE",
        "retirement preflight found a mismatched backing frame",
    ),
    "kernel/src/tlb.rs": (
        "pub fn shootdown_user_local(request: PerMmTlbRequest)",
        "local-only per-mm request targeted another CPU",
        "crate::context::assert_interrupts_disabled();",
    ),
    "kernel/src/user.rs": (
        "SYS_BRK: usize = 214",
        "SYS_MUNMAP: usize = 215",
        "SYS_MMAP: usize = 222",
        "SYS_MPROTECT: usize = 226",
        "resolve_active_fault",
        "M8-B4 demand paging/VM gate:",
        "TLB-before-free   : verified",
        "demand fault path : verified",
        "protection & (PROT_WRITE | PROT_EXEC)",
        "length.checked_add(PAGE_SIZE - 1)",
    ),
    "kernel/src/user/riscv64.S": (
        ".globl __m8_user_vm",
        ".globl __m8_user_mprotect_fault",
        ".globl __m8_user_munmap_fault",
        "li a0, 0x600000",
    ),
    "kernel/src/user/loongarch64.S": (
        ".globl __m8_user_vm",
        ".globl __m8_user_mprotect_fault",
        ".globl __m8_user_munmap_fault",
        "lu12i.w $r4, 0x600",
    ),
}

missing: list[str] = []
for relative, markers in CHECKS.items():
    path = ROOT / relative
    if not path.is_file():
        missing.append(f"{relative}: file")
        continue
    text = path.read_text(encoding="utf-8")
    missing.extend(f"{relative}: {marker}" for marker in markers if marker not in text)

if missing:
    print("M8-B4 audit: FAIL", file=sys.stderr)
    for item in missing:
        print(f"  missing: {item}", file=sys.stderr)
    raise SystemExit(1)

user_mm = (ROOT / "kernel/src/user_mm.rs").read_text(encoding="utf-8")
retire_start = user_mm.index("fn retire_range_locked(")
retire_end = user_mm.index("\nfn finish_retirement(", retire_start)
retire = user_mm[retire_start:retire_end]
plan = retire.index("let request = state.core.plan_tlb_request(")
unmap = retire.index(".unmap_page(mapping.page)")
if plan > unmap:
    print("M8-B4 audit: FAIL: TLB generation is planned after destructive unmap", file=sys.stderr)
    raise SystemExit(1)

finish_start = user_mm.index("fn finish_retirement(")
finish_end = user_mm.index("\nfn validate_range(", finish_start)
finish = user_mm[finish_start:finish_end]
flush = finish.index("crate::tlb::shootdown_user_local(request);")
free_backing = finish.index("crate::page_alloc::free(backing)?;")
free_table = finish.index("crate::page_alloc::free(table)?;")
if not (flush < free_backing < free_table):
    print("M8-B4 audit: FAIL: retired pages are not freed after TLB invalidation", file=sys.stderr)
    raise SystemExit(1)

protect_start = user_mm.index("    pub fn protect_range(")
protect_end = user_mm.index("\n    pub fn resolve_user_fault(", protect_start)
protect = user_mm[protect_start:protect_end]
for marker in (
    "let old_layout = state.core.layout().clone();",
    "let request = if changed_pages.is_empty()",
    ".protect_page(*page, old_area.mapping_options())",
    "*state.core.layout_mut() = old_layout;",
):
    if marker not in protect:
        print(f"M8-B4 audit: FAIL: mprotect transaction missing {marker}", file=sys.stderr)
        raise SystemExit(1)

print("M8-B4 audit: PASS")
print("  transactional VMA split : present")
print("  VMA coalescing           : present")
print("  anonymous demand fault  : wired")
print("  bounded stack growth    : wired")
print("  brk/mmap/munmap         : wired")
print("  mprotect rollback       : wired")
print("  local per-mm TLB gate   : present")
print("  TLB-before-free order   : present")
print("  dual-arch user probes   : present")
