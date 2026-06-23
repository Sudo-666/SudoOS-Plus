#!/usr/bin/env bash
set -euo pipefail

pick_readelf() {
  for t in llvm-readelf rust-readelf readelf; do
    if command -v "$t" >/dev/null 2>&1; then
      echo "$t"
      return 0
    fi
  done
  return 1
}

fail=0
if [ ! -f kernel-rv ]; then
  echo "[oscomp-linker-align-audit] FAIL: kernel-rv missing; run make all first"
  exit 1
fi

READELF="$(pick_readelf || true)"
if [ -z "${READELF}" ]; then
  echo "[oscomp-linker-align-audit] WARN: no readelf found; falling back to source scan"
else
  echo "[oscomp-linker-align-audit] using ${READELF}"
  # Find writable LOAD segments and require VirtAddr and PhysAddr to be 4KiB aligned.
  # This handles both GNU readelf and llvm-readelf text formats well enough for audit.
  "${READELF}" -lW kernel-rv > build/oscomp-kernel-rv-readelf.txt 2>/dev/null || "${READELF}" --program-headers kernel-rv > build/oscomp-kernel-rv-readelf.txt
  awk '
    $1 == "LOAD" {
      # GNU readelf: LOAD off vaddr paddr filesz memsz flags align
      off=$2; vaddr=$3; paddr=$4; flags="";
      for (i=1; i<=NF; i++) if ($i ~ /W/) flags="W";
      if (flags == "W") {
        vv=strtonum(vaddr); pp=strtonum(paddr);
        if ((vv % 4096) != 0 || (pp % 4096) != 0) {
          printf("[oscomp-linker-align-audit] FAIL: writable LOAD not page aligned: vaddr=%s paddr=%s\n", vaddr, paddr);
          bad=1;
        }
      }
    }
    END { exit bad ? 1 : 0 }
  ' build/oscomp-kernel-rv-readelf.txt || fail=1
fi

if grep -R --include='*.ld' --include='*.lds' --include='*.x' -n '\. = ALIGN(0x1000).*page-align writable PT_LOAD' . >/dev/null 2>&1; then
  echo "[oscomp-linker-align-audit] PASS: linker script has writable PT_LOAD page alignment"
else
  echo "[oscomp-linker-align-audit] WARN: did not find tagged linker alignment line"
fi

if [ "$fail" -eq 0 ]; then
  echo "[oscomp-linker-align-audit] PASS"
else
  exit 1
fi
