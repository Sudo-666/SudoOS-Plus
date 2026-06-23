#!/usr/bin/env bash
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "[oscomp-full] cleaning required kernel outputs"
rm -f kernel-rv kernel-la

echo "[oscomp-full] building both contest kernels"
make all

echo "[oscomp-full] static audits"
make oscomp-riscv-highhalf-linuxlike-audit
make oscomp-audit

# Optional runtime smoke if local contest images and QEMU are present.
if command -v qemu-system-riscv64 >/dev/null 2>&1 && [ -f sdcard-rv.img ]; then
  echo "[oscomp-full] running local RISC-V contest-style boot"
  set +e
  timeout 90 qemu-system-riscv64 -machine virt -kernel kernel-rv -m 1G -nographic -smp 1 -bios default \
    -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
    -no-reboot -device virtio-net-device,netdev=net -netdev user,id=net -rtc base=utc \
    > build/oscomp-riscv-local.log 2>&1
  rc=$?
  set -e
  if grep -q "RISC-V final address space" build/oscomp-riscv-local.log && \
     ! grep -q "physical page allocator:" build/oscomp-riscv-local.log; then
    echo "[oscomp-full] FAIL: RISC-V still stops after high-half handoff"
    tail -80 build/oscomp-riscv-local.log
    exit 1
  fi
  echo "[oscomp-full] RISC-V local log: build/oscomp-riscv-local.log (qemu rc=$rc)"
else
  echo "[oscomp-full] SKIP: local RISC-V QEMU/image not available"
fi

echo "[oscomp-full] PASS"
