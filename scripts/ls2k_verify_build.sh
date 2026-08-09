#!/bin/sh
# Verify the 16:45 ls2k1000 build has: ring-at-top in allocate() + raw a0/a1 capture in handler.
cd /mnt/d/oskernel2026-0xdeadbeef || exit 1

A2L=loongarch64-linux-gnu-addr2line
OBJ=loongarch64-linux-gnu-objdump
NM=loongarch64-linux-gnu-nm

echo "=== symbols ==="
"$NM" kernel-ls2k1000 | grep -E "KernelGlobalAllocator8allocate|ls2k_alloc_error_handler|__rg_oom"

ALLOC_ADDR=$("$NM" kernel-ls2k1000 | grep "KernelGlobalAllocator8allocate" | awk '{print $1}')
HANDLER_ADDR=$("$NM" kernel-ls2k1000 | grep "ls2k_alloc_error_handler" | awk '{print $1}')
echo "allocate=$ALLOC_ADDR handler=$HANDLER_ADDR"

echo ""
echo "=== allocate() first 0x140 bytes (ring write must precede beqz \$a1 size-0 check) ==="
"$OBJ" -d --start-address=0x$ALLOC_ADDR --stop-address=0x$(python3 -c "print(hex(0x$ALLOC_ADDR+0x140))") kernel-ls2k1000 2>&1 | grep -aE "amadd_db|stx.d|beqz|pcaddu18i|jirl|addi.d|ld.d|bgeu" | head -30

echo ""
echo "=== handler first 0x30 bytes (or \$a0/\$a1 capture) ==="
"$OBJ" -d --start-address=0x$HANDLER_ADDR --stop-address=0x$(python3 -c "print(hex(0x$HANDLER_ADDR+0x40))") kernel-ls2k1000 2>&1 | grep -aE "or |amadd_db|beqz|pcaddu18i|jirl" | head -12
