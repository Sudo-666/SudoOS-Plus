ARCH ?= riscv64
PROFILE ?= debug

SMP ?= 1
MEM ?= 256M
SMOKE_TIMEOUT ?= 30
SMP_SMOKE_TIMEOUT ?= 75
STRESS_ARCHES ?= $(ARCH)
STRESS_SMPS ?= $(SMP)
STRESS_MEMS ?= $(MEM)
STRESS_PROFILES ?= $(PROFILE)
STRESS_LOOPS ?= 1
STRESS_TIMEOUT ?= $(SMP_SMOKE_TIMEOUT)

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
export STRESS_ARCHES
export STRESS_SMPS
export STRESS_MEMS
export STRESS_PROFILES
export STRESS_LOOPS
export STRESS_TIMEOUT

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

.PHONY: smoke
smoke:
	@./scripts/smoke.py --arch "$(ARCH)" --profile "$(PROFILE)" --timeout "$(SMOKE_TIMEOUT)"

.PHONY: smoke-riscv64
smoke-riscv64:
	@./scripts/smoke.py --arch riscv64 --profile "$(PROFILE)" --timeout "$(SMOKE_TIMEOUT)"

.PHONY: smoke-loongarch64
smoke-loongarch64:
	@./scripts/smoke.py --arch loongarch64 --profile "$(PROFILE)" --timeout "$(SMOKE_TIMEOUT)"

.PHONY: smoke-all
smoke-all: smoke-riscv64 smoke-loongarch64

.PHONY: smoke-smp-riscv64
smoke-smp-riscv64:
	@SMP=4 ./scripts/smoke.py --arch riscv64 --profile "$(PROFILE)" --timeout "$(SMP_SMOKE_TIMEOUT)"

.PHONY: smoke-smp-loongarch64
smoke-smp-loongarch64:
	@SMP=4 ./scripts/smoke.py --arch loongarch64 --profile "$(PROFILE)" --timeout "$(SMP_SMOKE_TIMEOUT)"

.PHONY: smoke-smp-all
smoke-smp-all: smoke-smp-riscv64 smoke-smp-loongarch64

.PHONY: stress-smp
stress-smp:
	@./scripts/stress-smp.sh

.PHONY: fmt
fmt:
	@cargo fmt --all

.PHONY: fmt-check
fmt-check:
	@cargo fmt --all -- --check

.PHONY: test
test:
	@cargo test \
		-p myos-boot \
		-p myos-runtime \
		-p myos-fdt \
		-p myos-mm \
		-p myos-sync

.PHONY: clippy
clippy: clippy-riscv64 clippy-loongarch64 clippy-host

.PHONY: clippy-riscv64
clippy-riscv64:
	@cargo clippy \
		--manifest-path Cargo.toml \
		--package "$(KERNEL_PACKAGE)" \
		--bin "$(KERNEL_BINARY)" \
		--target riscv64imac-unknown-none-elf \
		-Z build-std=core,alloc \
		-Z build-std-features=compiler-builtins-mem \
		-- -D warnings

.PHONY: clippy-loongarch64
clippy-loongarch64:
	@cargo clippy \
		--manifest-path Cargo.toml \
		--package "$(KERNEL_PACKAGE)" \
		--bin "$(KERNEL_BINARY)" \
		--target loongarch64-unknown-none-softfloat \
		-Z build-std=core,alloc \
		-Z build-std-features=compiler-builtins-mem \
		-- -D warnings

.PHONY: clippy-host
clippy-host:
	@cargo clippy \
		-p myos-boot \
		-p myos-runtime \
		-p myos-fdt \
		-p myos-mm \
		-p myos-sync \
		--all-targets \
		-- -D warnings

.PHONY: source-tree-check
source-tree-check:
	@./scripts/check-source-tree.sh

.PHONY: check
check: source-tree-check fmt-check test build-riscv64 build-loongarch64 clippy

.PHONY: verify
verify: check smoke-all smoke-smp-all

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
	@command -v python3 >/dev/null || \
		(echo "error: python3 is not installed" && exit 1)

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
	@test -x scripts/smoke.py || \
		echo "warning: scripts/smoke.py is not executable"
	@test -x scripts/stress-smp.sh || \
		echo "warning: scripts/stress-smp.sh is not executable"
	@test -x scripts/check-source-tree.sh || \
		echo "warning: scripts/check-source-tree.sh is not executable"

	@echo "Doctor check completed"

.PHONY: help
help:
	@echo "SudoOS build commands"
	@echo ""
	@echo "  make build ARCH=riscv64"
	@echo "  make build ARCH=loongarch64"
	@echo ""
	@echo "  make run ARCH=riscv64"
	@echo "  make run ARCH=loongarch64"
	@echo ""
	@echo "  make smoke ARCH=riscv64"
	@echo "  make smoke-all"
	@echo "  make smoke-smp-all"
	@echo "  make stress-smp"
	@echo "  make check"
	@echo "  make verify"
	@echo ""
	@echo "  make debug ARCH=riscv64"
	@echo "  make debug ARCH=loongarch64"
	@echo ""
	@echo "Variables:"
	@echo "  ARCH=riscv64|loongarch64"
	@echo "  PROFILE=debug|release"
	@echo "  SMP=<cpu count>"
	@echo "  MEM=<memory size>"
	@echo "  SMOKE_TIMEOUT=<seconds>"
	@echo "  SMP_SMOKE_TIMEOUT=<seconds>"
	@echo "  STRESS_ARCHES='<arch list>'"
	@echo "  STRESS_SMPS='<cpu count list>'"
	@echo "  STRESS_MEMS='<memory size list>'"
	@echo "  STRESS_PROFILES='<profile list>'"
	@echo "  STRESS_LOOPS=<count>"
	@echo "  STRESS_TIMEOUT=<seconds>"
	@echo "  QEMU_ARGS='<additional arguments>'"
