#!/usr/bin/env python3
# ls2k_stabilize: verify qemu_virt kernel-la carries zero ls2k markers.
import sys

path = sys.argv[1] if len(sys.argv) > 1 else "kernel-la"
data = open(path, "rb").read()
for s in [
    b"OOM-HANDLER",
    b"HEAP-FATAL",
    b"RING total",
    b"HEAP-STATE",
    b"ls2k",
    b"LS2K",
    b"TASK00",
    b"raw_a0",
    b"HEAP-INSTALLED",
    b"HEAP-FATAL-START",
    b"memory allocation of",
    b"sudoos_alloc_trace",
    b"PROBE176",
    b"HEAPD",
]:
    print(f"{s.decode():24s}: {data.count(s)}")
print(f"kernel-la size: {len(data)}")
