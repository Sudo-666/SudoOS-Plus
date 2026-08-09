#!/bin/sh
# Verify qemu_virt kernel-la has zero ls2k markers (isolation check).
# Usage: ./scripts/ls2k_verify_la.sh [kernel-la-path]   (default: ./kernel-la)
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR" || exit 1
ELF="${1:-kernel-la}"
[ -f "$ELF" ] || { echo "error: $ELF not found (pass a path or build kernel-la first)" >&2; exit 1; }
echo "=== build log tail ==="
tail -2 .tmp_build_la.log 2>/dev/null || echo "(no .tmp_build_la.log)"
echo "=== $ELF ls2k markers (expect all 0) ==="
python3 - "$ELF" <<'EOF'
import sys
data = open(sys.argv[1], "rb").read()
for s in [b"OOM-HANDLER", b"HEAP-FATAL", b"RING total", b"HEAP-STATE",
          b"ls2k", b"LS2K", b"TASK00", b"raw_a0", b"HEAP-INSTALLED",
          b"HEAP-FATAL-START"]:
    print(f"{s.decode():20s}: {data.count(s)}")
print(f"{'memory allocation of':20s}: {data.count(b'memory allocation of')}")
print(f"kernel-la size: {len(data)}")
EOF
