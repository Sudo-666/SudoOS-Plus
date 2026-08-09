#!/bin/sh
# Verify the current ls2k1000 build has: ring-at-top in allocate() + raw a0/a1
# capture in handler.
# Usage: ./scripts/ls2k_verify_build.sh [kernel-ls2k1000-path]  (default ./kernel-ls2k1000)
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR" || exit 1
ELF="${1:-kernel-ls2k1000}"
[ -f "$ELF" ] || { echo "error: $ELF not found (pass a path or build kernel-ls2k1000 first)" >&2; exit 1; }

A2L=loongarch64-linux-gnu-addr2line
OBJ=loongarch64-linux-gnu-objdump
NM=loongarch64-linux-gnu-nm
command -v "$NM" >/dev/null 2>&1 || NM="llvm-nm"

echo "=== symbols ==="
"$NM" "$ELF" | grep -E "KernelGlobalAllocator8allocate|ls2k_alloc_error_handler|__rg_oom" || true

ALLOC_ADDR=$("$NM" "$ELF" | grep "KernelGlobalAllocator8allocate" | awk '{print $1}' | head -1)
HANDLER_ADDR=$("$NM" "$ELF" | grep "ls2k_alloc_error_handler" | awk '{print $1}' | head -1)
echo "allocate=$ALLOC_ADDR handler=$HANDLER_ADDR"

if [ -n "$ALLOC_ADDR" ]; then
  echo ""
  echo "=== allocate() first 0x140 bytes (ring write must precede beqz \$a1 size-0 check) ==="
  "$OBJ" -d --start-address=0x$ALLOC_ADDR --stop-address=0x$(python3 -c "print(hex(0x$ALLOC_ADDR+0x140))") "$ELF" 2>&1 | grep -aE "amadd_db|stx.d|beqz|pcaddu18i|jirl|addi.d|ld.d|bgeu" | head -30
fi

if [ -n "$HANDLER_ADDR" ]; then
  echo ""
  echo "=== handler first 0x30 bytes (or \$a0/\$a1 capture) ==="
  "$OBJ" -d --start-address=0x$HANDLER_ADDR --stop-address=0x$(python3 -c "print(hex(0x$HANDLER_ADDR+0x40))") "$ELF" 2>&1 | grep -aE "or |amadd_db|beqz|pcaddu18i|jirl" | head -12
fi
