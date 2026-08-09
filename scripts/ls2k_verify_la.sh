#!/bin/sh
# Verify qemu_virt kernel-la has zero ls2k markers (isolation check).
cd /mnt/d/oskernel2026-0xdeadbeef || exit 1
echo "=== build log tail ==="
tail -2 .tmp_build_la.log
echo "=== kernel-la ls2k markers (expect all 0) ==="
python3 - <<'EOF'
data = open("kernel-la", "rb").read()
for s in [b"OOM-HANDLER", b"HEAP-FATAL", b"RING total", b"HEAP-STATE",
          b"ls2k", b"LS2K", b"TASK00", b"raw_a0", b"HEAP-INSTALLED",
          b"HEAP-FATAL-START"]:
    print(f"{s.decode():20s}: {data.count(s)}")
print(f"{'memory allocation of':20s}: {data.count(b'memory allocation of')}")
print(f"kernel-la size: {len(data)}")
EOF
