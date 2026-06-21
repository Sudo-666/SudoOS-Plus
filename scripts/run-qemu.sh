#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
    pwd
)"

ROOT_DIR="$(
    cd -- "${SCRIPT_DIR}/.."
    pwd
)"

ARCH="${1:-${ARCH:-riscv64}}"
PROFILE="${2:-${PROFILE:-debug}}"

SMP="${SMP:-1}"
MEM="${MEM:-256M}"

QEMU_DEBUG="${QEMU_DEBUG:-0}"
QEMU_ARGS="${QEMU_ARGS:-}"

die() {
    echo "error: $*" >&2
    exit 1
}

select_qemu() {
    case "${ARCH}" in
        riscv64)
            QEMU="${QEMU_RISCV64:-qemu-system-riscv64}"

            MACHINE_ARGS=(
                -machine virt
                -bios default
            )
            ;;

        loongarch64)
            QEMU="${QEMU_LOONGARCH64:-qemu-system-loongarch64}"

            MACHINE_ARGS=(
                -machine virt
            )
            ;;

        *)
            die "unsupported architecture '${ARCH}'"
            ;;
    esac
}

build_kernel() {
    ARCH="${ARCH}" \
    PROFILE="${PROFILE}" \
        "${ROOT_DIR}/scripts/build.sh"
}

read_kernel_path() {
    local path_file="${ROOT_DIR}/build/${ARCH}/kernel.path"

    [[ -f "${path_file}" ]] ||
        die "kernel path file '${path_file}' was not generated"

    KERNEL_ELF="$(<"${path_file}")"

    [[ -f "${KERNEL_ELF}" ]] ||
        die "kernel ELF '${KERNEL_ELF}' does not exist"
}

check_qemu() {
    command -v "${QEMU}" >/dev/null 2>&1 ||
        die "'${QEMU}' was not found in PATH"
}

prepare_extra_arguments() {
    EXTRA_ARGS=()

    if [[ -n "${QEMU_ARGS}" ]]; then
        # QEMU_ARGS intended for ordinary whitespace-separated options.
        # Complex quoted values should be added directly to this script later.
        read -r -a EXTRA_ARGS <<< "${QEMU_ARGS}"
    fi
}

run_qemu() {
    QEMU_COMMAND=(
        "${QEMU}"

        "${MACHINE_ARGS[@]}"

        -m "${MEM}"
        -smp "${SMP}"

        -display none
        -serial stdio
        -monitor none

        -no-reboot
        -kernel "${KERNEL_ELF}"
    )

    if [[ "${QEMU_DEBUG}" == "1" ]]; then
        QEMU_COMMAND+=(
            -S
            -gdb tcp::1234
        )

        echo "QEMU will start paused."
        echo "GDB server: localhost:1234"
        echo
    fi

    QEMU_COMMAND+=(${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"})

    echo "Running MyOS"
    echo "  architecture : ${ARCH}"
    echo "  profile      : ${PROFILE}"
    echo "  qemu         : ${QEMU}"
    echo "  kernel       : ${KERNEL_ELF}"
    echo "  memory       : ${MEM}"
    echo "  cpus         : ${SMP}"
    echo

    exec "${QEMU_COMMAND[@]}"
}

main() {
    select_qemu
    build_kernel
    read_kernel_path
    check_qemu
    prepare_extra_arguments
    run_qemu
}

main "$@"