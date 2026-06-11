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

KERNEL_PACKAGE="${KERNEL_PACKAGE:-myos-kernel}"
KERNEL_BINARY="${KERNEL_BINARY:-myos-kernel}"

usage() {
    cat <<'EOF'
Usage:
    ./scripts/build.sh [architecture] [profile]

Architectures:
    riscv64
    loongarch64

Profiles:
    debug
    release

Examples:
    ./scripts/build.sh riscv64 debug
    ./scripts/build.sh loongarch64 release

The same values may also be supplied through environment variables:

    ARCH=riscv64 PROFILE=debug ./scripts/build.sh
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

select_architecture() {
    case "${ARCH}" in
        riscv64)
            TARGET="riscv64imac-unknown-none-elf"
            ;;

        loongarch64)
            TARGET="loongarch64-unknown-none-softfloat"
            ;;

        -h | --help)
            usage
            exit 0
            ;;

        *)
            die "unsupported architecture '${ARCH}'"
            ;;
    esac
}

select_profile() {
    case "${PROFILE}" in
        debug)
            CARGO_PROFILE_ARGS=()
            CARGO_PROFILE_DIR="debug"
            ;;

        release)
            CARGO_PROFILE_ARGS=(--release)
            CARGO_PROFILE_DIR="release"
            ;;

        *)
            die "unsupported build profile '${PROFILE}'"
            ;;
    esac
}

check_environment() {
    command -v cargo >/dev/null 2>&1 ||
        die "cargo is not installed"

    command -v rustc >/dev/null 2>&1 ||
        die "rustc is not installed"

    if [[ ! -f "${ROOT_DIR}/kernel/Cargo.toml" ]]; then
        cat >&2 <<EOF
error: kernel/Cargo.toml does not exist yet

The root build environment has been initialized correctly,
but the kernel crate has not been created.

Next project stage:

    kernel/
    ├── Cargo.toml
    └── src/
        ├── main.rs
        └── panic.rs
EOF
        exit 1
    fi
}

build_kernel() {
    local architecture_dir="${ROOT_DIR}/build/${ARCH}"

    export CARGO_TARGET_DIR="${architecture_dir}/cargo"

    mkdir -p "${architecture_dir}"

    echo "Building MyOS"
    echo "  architecture : ${ARCH}"
    echo "  rust target  : ${TARGET}"
    echo "  profile      : ${PROFILE}"
    echo "  target dir   : ${CARGO_TARGET_DIR}"
    echo

    cargo build \
    --manifest-path "${ROOT_DIR}/Cargo.toml" \
    --package "${KERNEL_PACKAGE}" \
    --bin "${KERNEL_BINARY}" \
    --target "${TARGET}" \
    -Z build-std=core \
    -Z build-std-features=compiler-builtins-mem \
    ${CARGO_PROFILE_ARGS[@]+"${CARGO_PROFILE_ARGS[@]}"}

    KERNEL_ELF="${CARGO_TARGET_DIR}/${TARGET}/${CARGO_PROFILE_DIR}/${KERNEL_BINARY}"

    if [[ ! -f "${KERNEL_ELF}" ]]; then
        die "kernel ELF was not produced at '${KERNEL_ELF}'"
    fi

    printf '%s\n' "${KERNEL_ELF}" \
        > "${architecture_dir}/kernel.path"

    echo
    echo "Build completed"
    echo "  kernel ELF: ${KERNEL_ELF}"
}

main() {
    select_architecture
    select_profile
    check_environment
    build_kernel
}

main "$@"