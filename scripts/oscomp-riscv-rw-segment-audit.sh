#!/usr/bin/env bash
set -euo pipefail

elf="${1:-kernel-rv}"
if [ ! -f "$elf" ]; then
  echo "[oscomp-riscv-rw-audit] ERROR: $elf not found; run make all first" >&2
  exit 1
fi

reader=""
for c in llvm-readelf readelf; do
  if command -v "$c" >/dev/null 2>&1; then
    reader="$c"
    break
  fi
done
if [ -z "$reader" ]; then
  echo "[oscomp-riscv-rw-audit] WARN: neither llvm-readelf nor readelf found; skipping ELF PT_LOAD audit"
  exit 0
fi

mkdir -p build
out="$($reader -lW "$elf")"
printf '%s\n' "$out" > build/oscomp-riscv-rw-segments.txt || true

bad=0
while IFS= read -r line; do
  case "$line" in
    *LOAD*W*)
      vaddr="$(printf '%s\n' "$line" | awk '{
        seen=0;
        for (i=1; i<=NF; i++) {
          if ($i == "LOAD") { seen=1; next }
          if (seen && $i ~ /^0x[0-9a-fA-F]+$/) {
            for (j=i+1; j<=NF; j++) {
              if ($j ~ /^0x[0-9a-fA-F]+$/) { print $j; exit }
            }
          }
        }
      }')"
      if [ -n "$vaddr" ]; then
        if [ $((vaddr % 4096)) -ne 0 ]; then
          echo "[oscomp-riscv-rw-audit] FAIL: writable LOAD vaddr is not page aligned: $vaddr" >&2
          echo "[oscomp-riscv-rw-audit] line: $line" >&2
          bad=1
        fi
      fi
      ;;
  esac
done <<EOF
$out
EOF

if [ "$bad" -ne 0 ]; then
  exit 1
fi

echo "[oscomp-riscv-rw-audit] PASS: writable LOAD segment starts are page aligned"
