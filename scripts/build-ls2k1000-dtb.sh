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
# Output: build/host-tools/ls2k1000/ls2k1000-minimal.dtb
#
# On the board (U-Boot):
#   fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
#   bootm 0x9000000002000000 - 0x900000000a000000
#
# Requires: dtc (device tree compiler).
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
OUT="${OUT_DIR}/ls2k1000-minimal.dtb"

mkdir -p "${OUT_DIR}"

DTS_SRC="${OUT_DIR}/ls2k1000-minimal.dts"
cat > "${DTS_SRC}" <<'EOF'
/dts-v1/;

/ {
    #address-cells = <2>;
    #size-cells = <2>;

    compatible = "loongson,ls2k1000";
    model = "loongson-2k1000";

    chosen {
        stdout-path = "serial0:115200n8";
        bootargs = "console=ttyS0,115200n8";
    };

    aliases {
        serial0 = "/soc/serial@1fe20000";
    };

    /*
     * 物理 RAM：主 bank [0x90000000, 0x100000000) = 1792 MiB。
     * 低 256 MiB bank [0x0, 0x10000000) 留给 U-Boot（uImage 暂存、DTB 所在），
     * 内核不接管。
     */
    memory@90000000 {
        device_type = "memory";
        reg = <0x0 0x90000000 0x0 0x70000000>;
    };

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

dtc -I dts -O dtb -o "${OUT}" "${DTS_SRC}" >/dev/null

echo "minimal DTB ready : ${OUT} ($(du -h "${OUT}" | cut -f1))"
