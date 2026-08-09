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

    write_buildinfo "${KERNEL_ELF}"

    echo
    echo "Build completed"
    echo "  kernel ELF: ${KERNEL_ELF}"
}

# PR-0 (reproducible build): write a .buildinfo file next to the kernel ELF
# recording the exact source/toolchain that produced it (git commit, rustc /
# cargo versions, release profile settings, vendored alloc hashes, ELF hash).
# A board image can then always be tied back to the committed sources.
write_buildinfo() {
    local elf="$1"
    local info="${elf}.buildinfo"
    local vendored="${ROOT_DIR}/vendor/rust-src/library"
    # Full-tree `git status` walks the huge vendor/rust-src tree over the 9p
    # mount and times out (10-20 s+), which previously produced a silently
    # WRONG "git_dirty_files=0". Restrict the walk to the actual build-input
    # paths (returns in ~7 s) and record `UNKNOWN(timeout)` when even that
    # exceeds the timeout, so the field is never a misleading "clean".
    local dirty dirty_list out
    if out="$(timeout 15 git -C "${ROOT_DIR}" status --porcelain --untracked-files=no -- \
            kernel mm arch scripts vendor/rust-src/library/alloc Makefile.project Cargo.toml .cargo/config.toml 2>/dev/null)"; then
        dirty="$(printf '%s\n' "$out" | grep -c . || true)"
        dirty_list="$(printf '%s\n' "$out" | awk '{print $2}' | paste -sd, -)"
        [ -n "$dirty_list" ] || dirty_list="clean"
    else
        dirty="UNKNOWN(timeout)"
        dirty_list="UNKNOWN(timeout)"
    fi
    {
        echo "SudoOS buildinfo"
        echo "git_commit=$(timeout 10 git -C "${ROOT_DIR}" rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "git_branch=$(timeout 10 git -C "${ROOT_DIR}" branch --show-current 2>/dev/null || echo unknown)"
        echo "git_dirty_files=${dirty}"
        echo "git_dirty_list=${dirty_list}"
        echo "rustc=$(rustc -Vv 2>/dev/null | tr '\n' ' ')"
        echo "cargo=$(cargo -V 2>/dev/null)"
        echo "target=${TARGET} profile=${PROFILE} platform=${PLATFORM:-default}"
        echo "release_profile=$(grep -A8 '^\[profile.release\]' "${ROOT_DIR}/Cargo.toml" 2>/dev/null | grep -E 'opt-level|lto|codegen-units|panic|overflow-checks' | tr '\n' ';')"
        echo "vendor_alloc_rs_sha256=$(sha256sum "${vendored}/alloc/src/alloc.rs" 2>/dev/null | cut -d' ' -f1)"
        echo "vendor_raw_vec_rs_sha256=$(sha256sum "${vendored}/alloc/src/raw_vec.rs" 2>/dev/null | cut -d' ' -f1)"
        echo "kernel_elf_sha256=$(sha256sum "${elf}" 2>/dev/null | cut -d' ' -f1)"
        echo "kernel_elf_size=$(stat -c %s "${elf}" 2>/dev/null || echo 0)"
    } > "${info}"
    echo "buildinfo: ${info}"
}

main() {
    select_architecture
    select_profile
    select_platform_features
    check_environment
    build_kernel
}

main "$@"