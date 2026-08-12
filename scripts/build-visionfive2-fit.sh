#!/usr/bin/env bash
# Build the single sudoos-visionfive2.itb FIT image and its manifest.
#
# Usage (usually via `make visionfive2-tftp-bundle`):
#   VISIONFIVE2_DTB=/path/to/board.dtb \
#   KERNEL_RAW=/path/to/kernel-vf2.bin \
#   KERNEL_ELF=/path/to/kernel-visionfive2 \
#   INITRAMFS=/path/to/busybox-riscv64.cpio \
#   PCB=v1.3b \
#   scripts/build-visionfive2-fit.sh
#
# Output (build/visionfive2/tftp/sudoos/vf2/):
#   sudoos-visionfive2.itb     single FIT (raw kernel + 3 DTBs + ramdisk)
#   visionfive2-manifest.txt   reproducible provenance + hashes
#
# Requires: mkimage (FIT), the pre-built raw kernel, derived DTBs and cpio.
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
DTB_DIR="${OUT_DIR}/dtbs"
mkdir -p "${OUT_DIR}"

KERNEL_RAW="${KERNEL_RAW:-${OUT_DIR}/kernel-vf2.bin}"
KERNEL_ELF="${KERNEL_ELF:-${ROOT_DIR}/kernel-visionfive2}"
INITRAMFS="${INITRAMFS:-${ROOT_DIR}/build/initramfs/busybox-riscv64.cpio}"
VISIONFIVE2_DTB="${VISIONFIVE2_DTB:-}"
PCB="${PCB:-unknown}"
ITS_TEMPLATE="${SCRIPT_DIR}/visionfive2-fit.its.in"
ITS_GEN="${OUT_DIR}/sudoos-visionfive2.its"
ITB="${OUT_DIR}/sudoos-visionfive2.itb"

for f in "${KERNEL_RAW}" "${KERNEL_ELF}" "${INITRAMFS}" \
         "${DTB_DIR}/vf2-selftest.dtb" "${DTB_DIR}/vf2-single.dtb" \
         "${DTB_DIR}/vf2-smp.dtb"; do
    [[ -f "${f}" ]] || {
        echo "error: required input missing: ${f}" >&2
        exit 2
    }
done
command -v mkimage >/dev/null 2>&1 || {
    echo "error: mkimage not found" >&2
    exit 1
}

# ---- substitute /incbin/ paths into the generated .its ----
sed \
    -e "s|KERNEL_RAW|$(realpath "${KERNEL_RAW}")|g" \
    -e "s|DTB_SELFTEST|$(realpath "${DTB_DIR}/vf2-selftest.dtb")|g" \
    -e "s|DTB_SINGLE|$(realpath "${DTB_DIR}/vf2-single.dtb")|g" \
    -e "s|DTB_SMP|$(realpath "${DTB_DIR}/vf2-smp.dtb")|g" \
    -e "s|INITRAMFS|$(realpath "${INITRAMFS}")|g" \
    "${ITS_TEMPLATE}" > "${ITS_GEN}"

# ---- build the FIT ----
mkimage -f "${ITS_GEN}" "${ITB}" >/dev/null
echo "FIT built      : ${ITB} ($(du -h "${ITB}" | cut -f1))"

# ---- manifest ----
sha256() { sha256sum "$1" | cut -d' ' -f1; }
size() { stat -c %s "$1"; }
bool() { if git rev-parse --git-dir >/dev/null 2>&1; then
        if git status --porcelain --untracked-files=no -- "${1}" 2>/dev/null | grep -q .; then
            echo dirty
        else
            echo clean
        fi
    else
        echo n/a
    fi; }

{
    echo "SudoOS VisionFive 2 TFTP/FIT bundle manifest"
    echo "============================================="
    echo "git_commit      = $(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "git_branch      = $(git branch --show-current 2>/dev/null || echo unknown)"
    echo "git_dirty       = $(bool '')"
    echo "source_dtb      = ${VISIONFIVE2_DTB:-unset}"
    echo "source_dtb_sha256 = $(sha256 "${VISIONFIVE2_DTB}" 2>/dev/null || echo unknown)"
    echo "source_dtb_size = $(size "${VISIONFIVE2_DTB}" 2>/dev/null || echo 0)"
    echo "pcb_revision    = ${PCB}"
    echo
    echo "kernel_elf      = ${KERNEL_ELF}"
    echo "kernel_elf_sha256 = $(sha256 "${KERNEL_ELF}")"
    echo "kernel_elf_size = $(size "${KERNEL_ELF}")"
    echo "kernel_raw      = ${KERNEL_RAW}"
    echo "kernel_raw_sha256 = $(sha256 "${KERNEL_RAW}")"
    echo "kernel_raw_size = $(size "${KERNEL_RAW}")"
    echo "kernel_load     = 0x40200000"
    echo "kernel_entry    = 0x40200000"
    echo
    echo "initramfs       = ${INITRAMFS}"
    echo "initramfs_sha256 = $(sha256 "${INITRAMFS}")"
    echo "initramfs_size  = $(size "${INITRAMFS}")"
    echo "fit             = ${ITB}"
    echo "fit_sha256      = $(sha256 "${ITB}")"
    echo "fit_size        = $(size "${ITB}")"
    echo
    echo "fit_configs     = conf-selftest conf-single conf-smp (default conf-smp)"
    echo "dtb_addr        = 0x46000000"
    echo "initrd_addr     = 0x46200000"
    echo "fit_staging     = 0x60000000"
    echo
    echo "bootargs_selftest = console=ttyS0,115200n8 sudoos.maxcpus=1"
    echo "bootargs_single   = console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=1"
    echo "bootargs_smp      = console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=4"
    echo
    echo "rustc           = $(rustc -Vv 2>/dev/null | tr '\n' ' ')"
    echo "cargo           = $(cargo -V 2>/dev/null)"
    echo "mkimage         = $(mkimage -V 2>/dev/null | head -1)"
    echo "dtc             = $(dtc -v 2>/dev/null | head -1)"
    echo "busybox_sha256  = $(sha256 "${ROOT_DIR}/vendor/userland/riscv64/busybox-static" 2>/dev/null || echo unknown)"
} > "${OUT_DIR}/visionfive2-manifest.txt"
echo "manifest       : ${OUT_DIR}/visionfive2-manifest.txt"

echo
echo "VISIONFIVE2_FIT_BUILD : PASS"
