#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${BUSYBOX:-}" ]]; then
  echo "error: BUSYBOX=/absolute/path/to/static/busybox is required" >&2
  exit 2
fi

OUT="${OUT:-${BUSYBOX_INITRAMFS:-build/initramfs/busybox.cpio}}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "${SCRIPT_DIR}/build-static-busybox-initramfs.py" --busybox "${BUSYBOX}" --out "${OUT}" "$@"
