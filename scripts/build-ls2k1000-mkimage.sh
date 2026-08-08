#!/usr/bin/env bash
# Build the LS2K1000 vendor U-Boot mkimage host tool.
#
# The 2K1000 board's U-Boot (2022.04-v2.1.0 BSP) uses a positional arch enum
# where LoongArch is IH_ARCH_LA = 27 (immediately after RISC-V = 26). Only the
# vendor mkimage knows that numbering, so LS2K1000 uImages MUST be produced by
# this tool — never by a system/distro mkimage or by hand-built Python headers.
#
# This script builds ONLY the host tool (out-of-tree `O=` build), it does not
# build or touch the rest of U-Boot, and it never installs into /usr/bin.
#
# Output: build/host-tools/ls2k1000/mkimage
#
# Environment:
#   LS2K1000_MKIMAGE   absolute path of an existing mkimage to use instead of
#                      building one (e.g. for a cross-built/special build)
#   UBOOT_SRC          vendor U-Boot source tree (default: auto-discovered)
set -Eeuo pipefail

SCRIPT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
    pwd
)"
ROOT_DIR="$(
    cd -- "${SCRIPT_DIR}/.."
    pwd
)"

OUT_DIR="${ROOT_DIR}/build/host-tools/ls2k1000"
MKIMAGE="${OUT_DIR}/mkimage"

# ── 1. Manual override ─────────────────────────────────────────────────────
if [ -n "${LS2K1000_MKIMAGE:-}" ]; then
    if [ -x "${LS2K1000_MKIMAGE}" ]; then
        echo "Using LS2K1000_MKIMAGE override: ${LS2K1000_MKIMAGE}"
        exit 0
    fi
    echo "error: LS2K1000_MKIMAGE is set but not executable: ${LS2K1000_MKIMAGE}" >&2
    exit 1
fi

# ── 2. Reuse an existing build ─────────────────────────────────────────────
if [ -x "${MKIMAGE}" ]; then
    echo "mkimage already built: ${MKIMAGE}"
    exit 0
fi

# ── 3. Locate the vendor U-Boot source ─────────────────────────────────────
UBOOT_SRC="${UBOOT_SRC:-}"
if [ -z "${UBOOT_SRC}" ]; then
    for cand in "${ROOT_DIR}"/vendor/u-boot-*; do
        if [ -f "${cand}/Makefile" ]; then
            UBOOT_SRC="${cand}"
            break
        fi
    done
fi
if [ -z "${UBOOT_SRC}" ] || [ ! -f "${UBOOT_SRC}/Makefile" ]; then
    echo "error: vendor U-Boot source not found under ${ROOT_DIR}/vendor/u-boot-*" >&2
    echo "       set UBOOT_SRC=<dir> to point at the 2K1000 BSP tree" >&2
    exit 1
fi
echo "vendor U-Boot source : ${UBOOT_SRC}"

# ── 4. Out-of-tree host-tools build (only the mkimage binary) ──────────────
echo "building vendor mkimage (host tool only, O=${OUT_DIR})"
mkdir -p "${OUT_DIR}"
make -C "${UBOOT_SRC}" "O=${OUT_DIR}" tools-only_defconfig

# The tools-only config turns on CONFIG_TOOLS_MKEFICAPSULE (EFI capsule tool);
# on hosts without the gnutls headers that tool fails to compile. mkimage does
# not need it, so drop it and regenerate before the real tools build.
if grep -q '^CONFIG_TOOLS_MKEFICAPSULE=y' "${OUT_DIR}/.config"; then
    sed -i 's/^CONFIG_TOOLS_MKEFICAPSULE=y$/# CONFIG_TOOLS_MKEFICAPSULE is not set/' \
        "${OUT_DIR}/.config"
    make -C "${UBOOT_SRC}" "O=${OUT_DIR}" olddefconfig >/dev/null
fi

make -C "${UBOOT_SRC}" "O=${OUT_DIR}" tools-only

cp "${OUT_DIR}/tools/mkimage" "${MKIMAGE}"
chmod +x "${MKIMAGE}"

# ── 5. Verify it understands the loongarch arch name ───────────────────────
# NOTE: `-A help` is itself an error path, so mkimage exits non-zero here;
# capture the output explicitly (pipefail would mask grep's success).
arch_out="$("${MKIMAGE}" -A help 2>&1 || true)"
if ! printf '%s\n' "${arch_out}" | grep -q "loongarch"; then
    echo "error: built mkimage does not support '-A loongarch'" >&2
    exit 1
fi

echo "vendor mkimage ready : ${MKIMAGE}"
"${MKIMAGE}" --help >/dev/null 2>&1 || true
