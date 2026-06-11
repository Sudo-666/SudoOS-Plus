ARCH ?= riscv64
PROFILE ?= debug

SMP ?= 1
MEM ?= 256M

KERNEL_PACKAGE ?= myos-kernel
KERNEL_BINARY ?= myos-kernel

QEMU_ARGS ?=

export ARCH
export PROFILE
export SMP
export MEM
export KERNEL_PACKAGE
export KERNEL_BINARY
export QEMU_ARGS

.PHONY: all
all: build

.PHONY: build
build:
	@./scripts/build.sh

.PHONY: run
run:
	@./scripts/run-qemu.sh

.PHONY: debug
debug:
	@QEMU_DEBUG=1 ./scripts/run-qemu.sh

.PHONY: build-riscv64
build-riscv64:
	@ARCH=riscv64 ./scripts/build.sh

.PHONY: build-loongarch64
build-loongarch64:
	@ARCH=loongarch64 ./scripts/build.sh

.PHONY: run-riscv64
run-riscv64:
	@ARCH=riscv64 ./scripts/run-qemu.sh

.PHONY: run-loongarch64
run-loongarch64:
	@ARCH=loongarch64 ./scripts/run-qemu.sh

.PHONY: fmt
fmt:
	@cargo fmt --all

.PHONY: fmt-check
fmt-check:
	@cargo fmt --all -- --check

.PHONY: clippy
clippy:
	@cargo clippy --workspace --all-targets

.PHONY: clean
clean:
	@rm -rf build
	@echo "Removed build directory"

.PHONY: doctor
doctor:
	@echo "Checking Rust toolchain..."
	@command -v rustup >/dev/null || \
		(echo "error: rustup is not installed" && exit 1)
	@command -v cargo >/dev/null || \
		(echo "error: cargo is not installed" && exit 1)
	@command -v rustc >/dev/null || \
		(echo "error: rustc is not installed" && exit 1)

	@echo "Checking QEMU..."
	@command -v qemu-system-riscv64 >/dev/null || \
		echo "warning: qemu-system-riscv64 was not found"
	@command -v qemu-system-loongarch64 >/dev/null || \
		echo "warning: qemu-system-loongarch64 was not found"

	@echo "Checking project scripts..."
	@test -x scripts/build.sh || \
		echo "warning: scripts/build.sh is not executable"
	@test -x scripts/run-qemu.sh || \
		echo "warning: scripts/run-qemu.sh is not executable"

	@echo "Doctor check completed"

.PHONY: help
help:
	@echo "MyOS build commands"
	@echo ""
	@echo "  make build ARCH=riscv64"
	@echo "  make build ARCH=loongarch64"
	@echo ""
	@echo "  make run ARCH=riscv64"
	@echo "  make run ARCH=loongarch64"
	@echo ""
	@echo "  make debug ARCH=riscv64"
	@echo "  make debug ARCH=loongarch64"
	@echo ""
	@echo "  make build-riscv64"
	@echo "  make build-loongarch64"
	@echo "  make run-riscv64"
	@echo "  make run-loongarch64"
	@echo ""
	@echo "Variables:"
	@echo "  ARCH=riscv64|loongarch64"
	@echo "  PROFILE=debug|release"
	@echo "  SMP=<cpu count>"
	@echo "  MEM=<memory size>"
	@echo "  QEMU_ARGS='<additional arguments>'"