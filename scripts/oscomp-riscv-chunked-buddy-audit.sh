#!/usr/bin/env bash
set -euo pipefail
python3 - <<'PY'
from pathlib import Path
ROOT = Path.cwd()
text = (ROOT / "kernel/src/memory.rs").read_text(encoding="utf-8")
checks = []
def add(name, ok): checks.append((name, bool(ok)))
add("ranges helper exists", "fn release_early_ranges_to_buddy_chunked" in text)
add("single range helper exists", "fn release_early_range_to_buddy_chunked" in text)
add("initialize uses chunked handoff", "release_early_ranges_to_buddy_chunked(&mut page_allocator, &early_allocator)" in text)
add("chunks use MAX_ORDER_NR_PAGES", "MAX_ORDER_NR_PAGES" in text)
add("chunks call normal release_range", "page_allocator.release_range(chunk)?" in text)
add("no runtime chunk begin trace", "P8R:release-chunk-begin" not in text)
add("no runtime chunk done trace", "P8S:release-chunk-done" not in text)
add("no trace release helper", "release_range_with_trace" not in text)
fail = 0
for name, ok in checks:
    print(("PASS" if ok else "FAIL") + f": {name}")
    fail += 0 if ok else 1
print(f"oscomp-riscv-chunked-buddy-audit: PASS={len(checks)-fail} FAIL={fail}")
raise SystemExit(1 if fail else 0)
PY
