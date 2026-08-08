#!/usr/bin/env bash
# OS COMP note: run `make oscomp-riscv-rw-segment-audit` after build to verify RISC-V writable LOAD alignment.

set -Eeuo pipefail

# OSKernel2026: ensure -Z build-std has a complete rust-src in the active sysroot.
if [ -x "./scripts/oscomp-prepare-rust-src.sh" ]; then
    ./scripts/oscomp-prepare-rust-src.sh
fi

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
PLATFORM="${PLATFORM:-}"

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

Platforms:
    qemu-virt      QEMU virtual machine (riscv64/loongarch64)
    ls2k1000       Loongson 2K1000 real hardware (loongarch64, default)
    visionfive2    StarFive VisionFive 2 / JH7110 (riscv64)

Examples:
    ./scripts/build.sh riscv64 debug
    ./scripts/build.sh loongarch64 release
    PLATFORM=qemu-virt ARCH=loongarch64 PROFILE=release ./scripts/build.sh
    PLATFORM=ls2k1000 ARCH=loongarch64 PROFILE=debug ./scripts/build.sh
    PLATFORM=visionfive2 ARCH=riscv64 PROFILE=release ./scripts/build.sh

The same values may also be supplied through environment variables:

    ARCH=riscv64 PROFILE=debug PLATFORM=qemu-virt ./scripts/build.sh
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

select_platform_features() {
    # 校验平台与架构的兼容性
    case "${ARCH}:${PLATFORM}" in
        riscv64:ls2k1000)
            die "platform 'ls2k1000' is loongarch64-only (riscv64 platforms: qemu-virt, visionfive2)"
            ;;

        loongarch64:visionfive2)
            die "platform 'visionfive2' is riscv64-only (loongarch64 platforms: qemu-virt, ls2k1000)"
            ;;
    esac

    # 未指定 PLATFORM 时，按架构选择默认平台。
    # riscv64 的平台化由 arch-riscv64 的 default feature 兜底,这里显式
    # 传参保证 build.rs 能根据 CARGO_FEATURE_* 选对链接脚本。
    if [ -z "${PLATFORM}" ]; then
        if [ "${ARCH}" = "riscv64" ]; then
            echo "  platform     : qemu-virt (default for riscv64)"
            CARGO_PLATFORM_ARGS=(--no-default-features --features platform-qemu-virt)
        else
            echo "  platform     : default (from Cargo.toml)"
            CARGO_PLATFORM_ARGS=()
        fi
        return
    fi

    case "${PLATFORM}" in
        qemu-virt)
            echo "  platform     : qemu-virt"
            CARGO_PLATFORM_ARGS=(--no-default-features --features platform-qemu-virt)
            ;;

        ls2k1000)
            echo "  platform     : ls2k1000"
            CARGO_PLATFORM_ARGS=(--no-default-features --features platform-ls2k1000)
            ;;

        visionfive2)
            echo "  platform     : visionfive2"
            CARGO_PLATFORM_ARGS=(--no-default-features --features platform-visionfive2)
            ;;

        *)
            die "unsupported platform '${PLATFORM}' (valid: qemu-virt, ls2k1000, visionfive2)"
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

    # Competition: hidden directories are filtered by the evaluator.
    # Restore .cargo from the un-hidden cargo-dot before building.
    if [[ ! -d "${ROOT_DIR}/.cargo" ]] && [[ -d "${ROOT_DIR}/cargo-dot" ]]; then
        echo "Restoring .cargo from cargo-dot"
        cp -R "${ROOT_DIR}/cargo-dot" "${ROOT_DIR}/.cargo"
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
    -Z build-std=core,alloc \
    -Z build-std-features=compiler-builtins-mem \
    ${CARGO_PLATFORM_ARGS[@]+"${CARGO_PLATFORM_ARGS[@]}"} \
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
    select_platform_features
    check_environment
    build_kernel
}

main "$@"