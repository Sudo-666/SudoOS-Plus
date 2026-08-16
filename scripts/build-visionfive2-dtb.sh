#!/usr/bin/env bash
# Validate an external VisionFive 2 board DTB and derive the three FIT DTB
# variants (conf-selftest / conf-single / conf-smp), which differ only in
# /chosen/bootargs. No linux,initrd-* is embedded: U-Boot bootm writes
# /chosen at runtime from the FIT ramdisk.
#
# Usage:
#   VISIONFIVE2_DTB=/absolute/path/to/jh7110-starfive-visionfive-2-v1.3b.dtb \
#   scripts/build-visionfive2-dtb.sh
#
# Output: build/visionfive2/tftp/sudoos/vf2/dtbs/vf2-{selftest,single,smp}.dtb
#
# Requires: dtc (for fdtget/fdtput), a full board DTB matching the PCB.
# Hand-written minimal DTBs are rejected (see CodePlan §16).
set -Eeuo pipefail

SCRIPT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
    pwd
)"
ROOT_DIR="$(
    cd -- "${SCRIPT_DIR}/.."
    pwd
)"

OUT_DIR="${ROOT_DIR}/build/visionfive2/tftp/sudoos/vf2/dtbs"
mkdir -p "${OUT_DIR}"

if [[ -z "${VISIONFIVE2_DTB:-}" ]]; then
    echo "error: VISIONFIVE2_DTB=/absolute/path/to/board.dtb is required" >&2
    exit 2
fi
if [[ ! -f "${VISIONFIVE2_DTB}" ]]; then
    echo "error: VISIONFIVE2_DTB is not a file: ${VISIONFIVE2_DTB}" >&2
    exit 2
fi

command -v fdtget >/dev/null 2>&1 || {
    echo "error: fdtget (device-tree-compiler) not found" >&2
    exit 1
}

DTB="${VISIONFIVE2_DTB}"

# ---- helpers (fdtget fails loudly; read as optional) ----
getprop() { # getprop <path> <prop>
    fdtget -t s "${DTB}" "$1" "$2" 2>/dev/null || true
}

die() {
    echo "error: ${1}" >&2
    exit 1
}

# ---- 1. model / compatible ----
MODEL="$(getprop / model)"
COMPAT="$(getprop / compatible)"
[[ -n "${MODEL}" ]] || die "DTB has no /model — not a real board DTB?"
[[ -n "${COMPAT}" ]] || die "DTB has no /compatible"
if ! grep -qi "starfive" <<<"${COMPAT}"; then
    echo "warning: /compatible '${COMPAT}' does not mention starfive; is this a JH7110 DTB?"
fi
echo "model          : ${MODEL}"
echo "compatible     : ${COMPAT}"

# ---- 1b. size sanity: real VF2 DTBs are tens of KiB; a hand-written minimal
# DTB (988 B) sails through the per-node checks but is not board-grade. ----
DTB_SIZE="$(stat -c %s "${DTB}")"
if (( DTB_SIZE < 4096 )); then
    die "DTB is only ${DTB_SIZE} bytes (< 4096) — not a full board DTB; supply the real starfive/jh7110-starfive-visionfive-2-v1.*.dtb (check printenv fdtfile), not a hand-written minimal DTB"
fi
echo "size           : ${DTB_SIZE} bytes"

# ---- 2. CPU nodes: JH7110 exposes all 5 harts (cpu@0 S7 + cpu@1..4 U74) ----
for hart in 0 1 2 3 4; do
    TYPE="$(getprop "/cpus/cpu@${hart}" device_type)"
    [[ "${TYPE}" = "cpu" ]] || die "/cpus/cpu@${hart} is missing or not device_type=cpu (need all 5 JH7110 hart nodes)"
done
TIMEBASE="$(fdtget -t x "${DTB}" /cpus timebase-frequency 2>/dev/null || true)"
[[ -n "${TIMEBASE}" ]] || die "/cpus timebase-frequency is missing (kernel SBI timer frequency comes from here)"
echo "cpus           : /cpus/cpu@0..cpu@4 present (S7 + 4x U74), timebase=${TIMEBASE}"

# ---- 3. memory node ----
MEM_TYPE="$(getprop /memory device_type)"
[[ "${MEM_TYPE}" = "memory" ]] || die "/memory is missing or not device_type=memory"
echo "memory         : /memory present"

# ---- 4. UART0 (0x10000000, JH7110 DW_apb_uart) ----
UART_FOUND=0
for candidate in /soc/serial@10000000 /soc/uart@10000000 /serial@10000000; do
    COMPAT_UART="$(getprop "${candidate}" compatible)"
    if [[ -n "${COMPAT_UART}" ]]; then
        UART_FOUND=1
        echo "uart0          : ${candidate} compatible=${COMPAT_UART}"
        break
    fi
done
[[ "${UART_FOUND}" = 1 ]] || {
    echo "warning: no serial node at 0x10000000 found; check stdout-path below" >&2
}

# ---- 5. chosen / stdout-path ----
STDOUT="$(getprop /chosen stdout-path)"
[[ -n "${STDOUT}" ]] || die "/chosen/stdout-path is missing"
echo "stdout-path    : ${STDOUT}"

# ---- derive the variants ----
BOOTARGS_SELFTEST="console=ttyS0,115200n8 sudoos.maxcpus=1"
BOOTARGS_SINGLE="console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=1"
BOOTARGS_SMP="console=ttyS0,115200n8 rdinit=/init init.debug=1 sudoos.maxcpus=4"
# C9: contest fixture configs (TF card = mmcblk1). No rdinit=/init.
BOOTARGS_CONTEST_SINGLE="console=ttyS0,115200n8 sudoos.oscomp=preliminary sudoos.contest.dev=mmcblk1 sudoos.contest.fixture=1 sudoos.maxcpus=1"
BOOTARGS_CONTEST_SMP="console=ttyS0,115200n8 sudoos.oscomp=preliminary sudoos.contest.dev=mmcblk1 sudoos.contest.fixture=1 sudoos.maxcpus=4"

declare -A VARIANTS=(
    [selftest]="${BOOTARGS_SELFTEST}"
    [single]="${BOOTARGS_SINGLE}"
    [smp]="${BOOTARGS_SMP}"
    [contest-fixture-single]="${BOOTARGS_CONTEST_SINGLE}"
    [contest-fixture-smp]="${BOOTARGS_CONTEST_SMP}"
)

for variant in selftest single smp contest-fixture-single contest-fixture-smp; do
    out="${OUT_DIR}/vf2-${variant}.dtb"
    cp "${DTB}" "${out}"
    fdtput -t s "${out}" /chosen bootargs "${VARIANTS[${variant}]}"
    echo "derived        : ${out} (bootargs='${VARIANTS[${variant}]}')"
done

echo
echo "VISIONFIVE2_DTB_DERIVE : PASS (${#VARIANTS[@]} variants)"
