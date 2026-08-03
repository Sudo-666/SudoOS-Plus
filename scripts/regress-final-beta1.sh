#!/usr/bin/env bash
set -euo pipefail

REPO=""
IMAGES=""
MODE="buildstorm"
ARCH="rv"
DO_BUILD=1
TIMEOUT="${QEMU_TIMEOUT:-18000}"
RV_MEM="${RV_MEM:-16G}"
RV_CPUS="${RV_CPUS:-8}"
LA_MEM="${LA_MEM:-36G}"
LA_CPUS="${LA_CPUS:-12}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="$2"; shift 2 ;;
    --images) IMAGES="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --no-build) DO_BUILD=0; shift ;;
    -h|--help)
      echo "Usage: $0 --repo PATH --images PATH --mode buildstorm|cagent|full --arch rv|la|both [--no-build]"
      exit 0 ;;
    *) echo "[regress-v6] unknown option: $1" >&2; exit 2 ;;
  esac
done

REPO="${REPO:-$PWD}"
REPO="$(cd "$REPO" && pwd)"
IMAGES="${IMAGES:-$HOME/Downloads/2026OSImage-Pub}"

existing_qemu="$(pgrep -f 'qemu-system-(riscv64|loongarch64).*kernel-(rv|la)' || true)"
if [[ -n "$existing_qemu" ]]; then
  echo "[regress-v6] REFUSE: concurrent/stale QEMU exists: $existing_qemu" >&2
  exit 3
fi

cd "$REPO"

python3 - <<'PY'
from pathlib import Path
import sys
u = Path("kernel/src/user.rs").read_text(encoding="utf-8")
required = [
    "// SUDOOS_FCNTL_RECORD_LOCKS_V6",
    "struct LinuxFlock64",
    "static POSIX_RECORD_LOCKS",
    "F_SETLK | F_SETLKW | F_OFD_SETLK | F_OFD_SETLKW",
    "SUDOOS_BUILDSTORM_BOOTSTRAP_V5 source=compile target=tmpfs",
]
missing = [x for x in required if x not in u]
if missing:
    print("[regress-v6] source audit FAIL:", *missing, sep="\n  ", file=sys.stderr)
    raise SystemExit(1)
print("[regress-v6] source audit PASS")
PY

if [[ $DO_BUILD -eq 1 ]]; then make all; fi

CACHE="$REPO/.cache/oscomp-final-images"
STAMP="$(date +%Y%m%d-%H%M%S)-v6"
LOG_ROOT="$REPO/artifacts/final-beta1-local/$STAMP"
mkdir -p "$CACHE" "$LOG_ROOT"

prepare_image() {
  local arch="$1"
  local raw="$IMAGES/sdcard-$arch-pub.img"
  local gz="$IMAGES/sdcard-$arch-pub.img.gz"
  local img="$CACHE/sdcard-$arch-pub.img"
  if [[ -s "$raw" ]]; then
    echo "[regress-v6] use raw image $raw" >&2
    printf '%s\n' "$raw"
    return 0
  fi
  [[ -f "$gz" ]] || { echo "[regress-v6] missing $gz" >&2; return 1; }
  if [[ ! -s "$img" || "$gz" -nt "$img" ]]; then
    echo "[regress-v6] decompress $gz -> $img" >&2
    gzip -dc "$gz" > "$img.tmp"
    mv "$img.tmp" "$img"
  fi
  printf '%s\n' "$img"
}

validate() {
  local mode="$1" log="$2"
  if [[ "$mode" != cagent ]]; then
    grep -Eq '^SUDOOS_BUILDSTORM_BOOTSTRAP_V5 rc=0 .*bytes=[1-9][0-9]*$' "$log" || return 1
    grep -Eq '^BUILDSTORM_COMPILE mode=multi ok=true .*rc=0 .*bytes=[1-9][0-9]{5,} .*' "$log" || return 1
    grep -Eq '^#### OS COMP TEST GROUP END buildstorm-glibc ####\r*$' "$log" || return 1
  fi
  if [[ "$mode" != buildstorm ]]; then
    grep -Eq '^#### OS COMP TEST GROUP END cagent-glibc ####\r*$' "$log" || return 1
    grep -Eq '^SMOKE_TEST: PASS\r*$' "$log" || return 1
  fi
  ! grep -Eq 'KERNEL PANIC|^panicked at |ok=false|BOOTSTRAP_V5 rc=[1-9]|Error code 3850: disk I/O error' "$log"
}

run_one() {
  local arch="$1" mode="$2" image="$3"
  local log="$LOG_ROOT/$mode-$arch.log"
  local append=""
  [[ "$mode" == buildstorm ]] && append="sudoos.oscomp=final-buildstorm"
  [[ "$mode" == cagent ]] && append="sudoos.oscomp=final-cagent"

  local -a cmd
  if [[ "$arch" == rv ]]; then
    cmd=(qemu-system-riscv64 -machine virt -kernel kernel-rv
      -m "$RV_MEM" -smp "$RV_CPUS" -bios default
      -drive "file=$image,if=none,format=raw,id=x0"
      -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0
      -snapshot -no-reboot
      -device virtio-net-device,netdev=net
      -netdev user,id=net -rtc base=utc
      -monitor none -display none -serial "file:$log")
  else
    cmd=(qemu-system-loongarch64 -kernel kernel-la
      -m "$LA_MEM" -smp "$LA_CPUS"
      -drive "file=$image,if=none,format=raw,id=x0"
      -device virtio-blk-pci,drive=x0
      -snapshot -no-reboot
      -device virtio-net-pci,netdev=net0
      -netdev user,id=net0 -rtc base=utc
      -monitor none -display none -serial "file:$log")
  fi
  [[ -z "$append" ]] || cmd+=(-append "$append")

  echo "[regress-v6] START arch=$arch mode=$mode log=$log"
  "${cmd[@]}" &
  local qpid=$!
  local elapsed=0

  while kill -0 "$qpid" 2>/dev/null; do
    sleep 30
    elapsed=$((elapsed + 30))
    local bytes last
    bytes="$(wc -c < "$log" 2>/dev/null || echo 0)"
    last="$(grep -E 'SUDOOS_BUILDSTORM_BOOTSTRAP_V5|BUILDSTORM_COMPILE|Error code 3850|PANIC|panicked' "$log" 2>/dev/null | tail -n 1 || true)"
    echo "[regress-v6] ACTIVE pid=$qpid arch=$arch mode=$mode elapsed_s=$elapsed log_bytes=$bytes last=${last:-<none>}"

    if grep -Eq 'KERNEL PANIC|^panicked at |BUILDSTORM_COMPILE mode=multi ok=false|BOOTSTRAP_V5 rc=[1-9]|Error code 3850: disk I/O error' "$log" 2>/dev/null; then
      kill -TERM "$qpid" 2>/dev/null || true
      wait "$qpid" 2>/dev/null || true
      echo "[regress-v6] FAIL early arch=$arch mode=$mode" >&2
      tail -n 260 "$log" >&2
      return 1
    fi

    if validate "$mode" "$log"; then
      kill -TERM "$qpid" 2>/dev/null || true
      wait "$qpid" 2>/dev/null || true
      echo "[regress-v6] PASS arch=$arch mode=$mode"
      grep -E 'SUDOOS_BUILDSTORM_BOOTSTRAP_V5|BUILDSTORM_COMPILE|OS COMP TEST GROUP END|SMOKE_TEST' "$log" | tail -n 120
      return 0
    fi

    if (( elapsed >= TIMEOUT )); then
      kill -TERM "$qpid" 2>/dev/null || true
      sleep 1
      kill -KILL "$qpid" 2>/dev/null || true
      wait "$qpid" 2>/dev/null || true
      echo "[regress-v6] FAIL timeout arch=$arch mode=$mode" >&2
      tail -n 320 "$log" >&2
      return 1
    fi
  done

  wait "$qpid" || true
  if validate "$mode" "$log"; then
    echo "[regress-v6] PASS arch=$arch mode=$mode"
    return 0
  fi
  echo "[regress-v6] FAIL QEMU exited before success arch=$arch mode=$mode" >&2
  tail -n 320 "$log" >&2
  return 1
}

case "$ARCH" in
  rv) run_one rv "$MODE" "$(prepare_image rv)" ;;
  la) run_one la "$MODE" "$(prepare_image la)" ;;
  both)
    run_one rv "$MODE" "$(prepare_image rv)"
    run_one la "$MODE" "$(prepare_image la)"
    ;;
  *) echo "[regress-v6] invalid arch: $ARCH" >&2; exit 2 ;;
esac

echo "[regress-v6] logs: $LOG_ROOT"
