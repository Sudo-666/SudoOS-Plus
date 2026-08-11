#!/usr/bin/env bash
# Compile the network-address-free U-Boot script into a loadable .scr.
#
# Output: build/visionfive2/tftp/sudoos/vf2/sudoos-vf2-tftp.scr
#
# Requires: mkimage.
set -Eeuo pipefail

SCRIPT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
    pwd
)"
ROOT_DIR="$(
    cd -- "${SCRIPT_DIR}/.."
    pwd
)"

OUT_DIR="${ROOT_DIR}/build/visionfive2/tftp/sudoos/vf2"
mkdir -p "${OUT_DIR}"

CMD="${SCRIPT_DIR}/visionfive2-tftp.cmd"
SCR="${OUT_DIR}/sudoos-vf2-tftp.scr"

command -v mkimage >/dev/null 2>&1 || {
    echo "error: mkimage not found" >&2
    exit 1
}

mkimage -T script -C none -d "${CMD}" "${SCR}" >/dev/null
cp "${CMD}" "${OUT_DIR}/sudoos-vf2-tftp.cmd"
echo "U-Boot script  : ${SCR} ($(du -h "${SCR}" | cut -f1))"
echo "VISIONFIVE2_UBOOT_SCRIPT : PASS"
