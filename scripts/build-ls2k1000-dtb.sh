#!/usr/bin/env bash
# Build a minimal LS2K1000 kernel device-tree blob for U-Boot bootm.
#
# The board's SPI-flash "dtb" partition (sf read ${fdt_addr} dtb) does not
# contain a valid DTB on this unit, and the vendor U-Boot runtime DTS
# (arch/loongarch/dts/ls2k1000_dp.dts) declares memory in cached-window VA
# form (0x9000000000000000...), which the kernel's DMW phys_to_cached mask
# rejects. This minimal DTB describes RAM with plain physical addresses and
# the UART0 console, matching the ls2k1000 platform constants.
#
# Stage-4 extension: when INITRD points at a newc initramfs, the chosen node
# gains linux,initrd-start/end and a matching /memreserve/ is emitted, so the
# kernel can locate and unpack the archive without any U-Boot initrd support
# (the vendor U-Boot keeps images->initrd_start/end at 0).
#   INITRD=build/initramfs/busybox-loongarch64.cpio \
#   INITRD_PHYS_ADDR=0x0b000000 \
#   scripts/build-ls2k1000-dtb.sh
#
# Output (no INITRD):   build/host-tools/ls2k1000/ls2k1000-minimal.dtb
# Output (with INITRD): build/host-tools/ls2k1000/ls2k1000-stage4.dtb
#
# On the board (U-Boot):
#   fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
#   fatload usb 0:1 0x900000000a000000 ls2k1000-stage4.dtb
#   fatload usb 0:1 0x900000000b000000 busybox-loongarch64.cpio
#   bootm 0x9000000002000000 - 0x900000000a000000
#
# Requires: dtc (device tree compiler), stat/grep (coreutils).
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
mkdir -p "${OUT_DIR}"

# ---- stage-4 external initramfs (optional) ----
INITRD="${INITRD:-}"
INITRD_PHYS_ADDR="${INITRD_PHYS_ADDR:-0x0b000000}"

# Boot command line written into /chosen/bootargs. Override to select the
# userland boot mode, e.g. "console=ttyS0,115200n8 rdinit=/init init.debug=1".
BOOTARGS="${BOOTARGS:-console=ttyS0,115200n8}"

# Output base name. Defaults to ls2k1000-minimal / ls2k1000-stage4; override to
# emit a named variant such as ls2k1000-stage4-init.
DTB_NAME="${DTB_NAME:-}"

# Contest fixture ramdisk (CodePlan C5): when CONTEST_DISK is a physical
# address, emit a /reserved-memory/contest-disk node
# (compatible = "sudoos,boot-ramdisk") so the kernel registers /dev/ram0 over
# the firmware-loaded image. The region is excluded from free memory by the
# /reserved-memory reservation. Default 32 MiB.
CONTEST_DISK="${CONTEST_DISK:-}"
CONTEST_DISK_SIZE="${CONTEST_DISK_SIZE:-0x02000000}"

RESERVED_MEMORY_LINES=""
if [[ -n "${CONTEST_DISK}" ]]; then
    CONTEST_DISK_ADDR=$(( CONTEST_DISK ))
    CONTEST_DISK_LEN=$(( CONTEST_DISK_SIZE ))
    if [[ ${CONTEST_DISK_ADDR} -eq 0 || ${CONTEST_DISK_LEN} -eq 0 ]]; then
        echo "error: contest disk address/size must be non-zero" >&2
        exit 1
    fi
    CONTEST_DISK_ADDR_HEX=$(printf '%08x' "${CONTEST_DISK_ADDR}")
    CONTEST_DISK_SIZE_HEX=$(printf '%08x' "${CONTEST_DISK_LEN}")
    RESERVED_MEMORY_LINES=$(cat <<EOT

    reserved-memory {
        #address-cells = <2>;
        #size-cells = <2>;
        ranges;

        contest_disk: contest-disk@${CONTEST_DISK_ADDR_HEX} {
            compatible = "sudoos,boot-ramdisk";
            reg = <0x0 0x${CONTEST_DISK_ADDR_HEX} 0x0 0x${CONTEST_DISK_SIZE_HEX}>;
            block-size = <512>;
            read-only;
        };
    };
EOT
)
fi

# Fixed physical staging layout (U-Boot cached-VA = phys + 0x9000000000000000):
#   kernel uImage   0x02000000
#   DTB             0x0a000000
#   raw initramfs   0x0b000000
#   U-Boot          0x0ec00000
KERNEL_BASE=0x02000000
DTB_BASE=0x0a000000
UBOOT_BASE=0x0ec00000
LOW_BANK_END=0x10000000

INITRD_PROPS=""
MEMRESERVE_LINES=""
if [[ -n "${DTB_NAME}" ]]; then
    OUT_BASENAME="${DTB_NAME}"
elif [[ -n "${CONTEST_DISK}" ]]; then
    OUT_BASENAME="ls2k1000-contest-fixture"
elif [[ -n "${INITRD}" ]]; then
    OUT_BASENAME="ls2k1000-stage4"
else
    OUT_BASENAME="ls2k1000-minimal"
fi

if [[ -n "${INITRD}" ]]; then
    if [[ ! -f "${INITRD}" ]]; then
        echo "error: INITRD is not a file: ${INITRD}" >&2
        exit 1
    fi
    if ! head -c 6 "${INITRD}" | grep -q "070701"; then
        echo "error: INITRD is not a newc cpio archive (bad magic): ${INITRD}" >&2
        exit 1
    fi
    INITRD_SIZE=$(stat -c %s "${INITRD}")
    INITRD_ADDR=$(( INITRD_PHYS_ADDR ))
    INITRD_END=$(( INITRD_ADDR + INITRD_SIZE ))

    # ---- reject overlaps with kernel/DTB staging and the U-Boot region ----
    if [[ ${INITRD_ADDR} -lt ${DTB_BASE} && ${KERNEL_BASE} -lt ${INITRD_END} ]]; then
        echo "error: initramfs [0x$(printf %x ${INITRD_ADDR}), 0x$(printf %x ${INITRD_END})) overlaps kernel/DTB staging [0x${KERNEL_BASE}, 0x${DTB_BASE})" >&2
        exit 1
    fi
    if [[ ${INITRD_ADDR} -lt ${LOW_BANK_END} && ${UBOOT_BASE} -lt ${INITRD_END} ]]; then
        echo "error: initramfs [0x$(printf %x ${INITRD_ADDR}), 0x$(printf %x ${INITRD_END})) overlaps U-Boot region [0x${UBOOT_BASE}, 0x${LOW_BANK_END})" >&2
        exit 1
    fi
    if [[ ${INITRD_END} -gt ${LOW_BANK_END} ]]; then
        echo "error: initramfs [0x$(printf %x ${INITRD_ADDR}), 0x$(printf %x ${INITRD_END})) exceeds low 256MiB bank" >&2
        exit 1
    fi

    INITRD_START_HEX=$(printf '%08x' "${INITRD_ADDR}")
    INITRD_END_HEX=$(printf '%08x' "${INITRD_END}")
    INITRD_SIZE_HEX=$(printf '%08x' "${INITRD_SIZE}")
    INITRD_PROPS="        linux,initrd-start = <0x0 0x${INITRD_START_HEX}>;
        linux,initrd-end   = <0x0 0x${INITRD_END_HEX}>;"
    MEMRESERVE_LINES="/memreserve/ 0x${INITRD_START_HEX} 0x${INITRD_SIZE_HEX};"
fi

OUT="${OUT_DIR}/${OUT_BASENAME}.dtb"
DTS_SRC="${OUT_DIR}/${OUT_BASENAME}.dts"

cat > "${DTS_SRC}" <<EOF
/dts-v1/;

${MEMRESERVE_LINES}

/ {
    #address-cells = <2>;
    #size-cells = <2>;

    compatible = "loongson,ls2k1000";
    model = "loongson-2k1000";

    chosen {
        stdout-path = "serial0:115200n8";
        bootargs = "${BOOTARGS}";
${INITRD_PROPS}
    };

    aliases {
        serial0 = "/soc/serial@1fe20000";
    };

    /*
     * 物理 RAM：主 bank [0x90000000, 0x100000000) = 1792 MiB。
     * 低 256 MiB bank [0x0, 0x10000000) 留给 U-Boot（uImage 暂存、DTB、initramfs 所在），
     * 内核不接管。
     */
    memory@90000000 {
        device_type = "memory";
        reg = <0x0 0x90000000 0x0 0x70000000>;
    };

${RESERVED_MEMORY_LINES}

    /* LA264 双核；内核要求 /cpus 节点，cpu 子节点用 reg 给硬件 ID。 */
    cpus {
        #address-cells = <1>;
        #size-cells = <0>;

        cpu@0 {
            device_type = "cpu";
            compatible = "loongarch";
            reg = <0>;
        };

        cpu@1 {
            device_type = "cpu";
            compatible = "loongarch";
            reg = <1>;
        };
    };

    soc {
        compatible = "simple-bus";
        #address-cells = <2>;
        #size-cells = <2>;
        ranges;

        serial@1fe20000 {
            compatible = "ns16550a";
            reg = <0x0 0x1fe20000 0x0 0x10>;
            current-speed = <115200>;
        };
    };
};
EOF

command -v dtc >/dev/null 2>&1 || {
    echo "error: dtc (device tree compiler) not found" >&2
    exit 1
}

# -p 4096 pads the blob so U-Boot can append/fixup /chosen in place.
dtc -I dts -O dtb -p 4096 -o "${OUT}" "${DTS_SRC}" >/dev/null

echo "${OUT_BASENAME}.dtb ready : ${OUT} ($(du -h "${OUT}" | cut -f1))"
echo "  bootargs      : ${BOOTARGS}"
if [[ -n "${INITRD}" ]]; then
    echo "  linux,initrd  : [0x${INITRD_START_HEX}, 0x${INITRD_END_HEX}) ${INITRD_SIZE} bytes"
    echo "  /memreserve/  : 0x${INITRD_START_HEX} 0x${INITRD_SIZE_HEX}"
fi
