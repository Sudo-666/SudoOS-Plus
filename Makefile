# OSKernel2026 submission wrapper.
# The judge runs exactly: make all
# Keep local smoke/stress/soak/QEMU out of `all`.

.PHONY: all oscomp-all oscomp-audit oscomp-vendor oscomp-clean oscomp-local help oscomp-riscv-boot-to-runtime-big-repair-audit

FINAL_IMAGE_RV ?= /Volumes/U/sudoos-final-2026/images/sdcard-rv-pub.img
FINAL_IMAGE_LA ?= /Volumes/U/sudoos-final-2026/images/sdcard-la-pub.img
FINAL_LOG_DIR ?= artifacts/final-2026/logs
FINAL_CPUS ?= 8
FINAL_MEM ?= 8G
FINAL_RUN_ID ?= $(shell date +%Y%m%d-%H%M%S)
FINAL_CAGENT_TIMEOUT ?= 300
FINAL_CAGENT_REJECT_GUARD ?= --failure-regex '^testcase cagent .* reject [0-9]+$$'
FINAL_BUILDSTORM_TIMEOUT ?= 15000
FINAL_LIFECYCLE_TIMEOUT ?= 15000

all: oscomp-all

oscomp-all:
	@bash scripts/oscomp-build.sh

oscomp-audit:
	@python3 scripts/oscomp-audit.py

.PHONY: verify-final-script-sha256
verify-final-script-sha256:
	@scripts/verify-final-script-sha256.sh

.PHONY: oscomp-baseline-check
oscomp-baseline-check:
	@python3 scripts/oscomp_baseline_guard.py

oscomp-vendor:
	@bash scripts/oscomp-vendor.sh

oscomp-clean:
	@rm -f kernel-rv kernel-la disk-rv.img
	@$(MAKE) -f Makefile.project clean || true

oscomp-local:
	@bash scripts/oscomp-local-eval.sh

help:
	@echo "Contest targets: make all | make oscomp-audit | make oscomp-vendor | make oscomp-local"
	@echo "Original project targets are forwarded to Makefile.project."

# Forward ordinary developer targets to the original Makefile.
%:
	@$(MAKE) -f Makefile.project $@

.PHONY: oscomp-buildstd-audit
oscomp-buildstd-audit:
	@./scripts/oscomp-buildstd-audit.sh

.PHONY: oscomp-rust2025-compat-audit
oscomp-rust2025-compat-audit:
	@./scripts/oscomp-rust2025-compat-audit.sh

.PHONY: oscomp-rust2025-global-audit
oscomp-rust2025-global-audit:
	@scripts/oscomp-rust2025-global-audit.sh

.PHONY: oscomp-feature-gate-repair oscomp-feature-gate-repair-audit
oscomp-feature-gate-repair:
	@python3 scripts/oscomp-rust2025-feature-gate-repair.py

oscomp-feature-gate-repair-audit:
	@python3 scripts/oscomp-rust2025-feature-gate-repair.py



.PHONY: oscomp-virtio-letchains-audit
oscomp-virtio-letchains-audit:
	@./scripts/oscomp-virtio-letchains-audit.sh .

.PHONY: oscomp-rust-src-audit
oscomp-rust-src-audit:
	@./scripts/oscomp-rust-src-audit.sh

.PHONY: oscomp-rust-src-repair-audit
oscomp-rust-src-repair-audit:
	@./scripts/oscomp-rust-src-repair-audit.sh

.PHONY: oscomp-linker-align-audit
oscomp-linker-align-audit:
	@bash scripts/oscomp-linker-align-audit.sh


.PHONY: oscomp-riscv-rw-segment-audit
oscomp-riscv-rw-segment-audit:
	@./scripts/oscomp-riscv-rw-segment-audit.sh kernel-rv

.PHONY: oscomp-riscv-lowmap-audit
oscomp-riscv-lowmap-audit:
	@./scripts/oscomp-riscv-lowmap-audit.sh

.PHONY: oscomp-riscv-highhalf-audit
oscomp-riscv-highhalf-audit:
	@python3 scripts/oscomp-riscv-highhalf-audit.py

.PHONY: oscomp-riscv-stack-handoff-audit
oscomp-riscv-stack-handoff-audit:
	@./scripts/oscomp-riscv-stack-handoff-audit.sh

.PHONY: oscomp-riscv-highhalf-linuxlike-audit
oscomp-riscv-highhalf-linuxlike-audit:
	@python3 scripts/oscomp-riscv-highhalf-linuxlike-audit.py

.PHONY: oscomp-full-contest-preflight
oscomp-full-contest-preflight:
	@./scripts/oscomp-full-contest-preflight.sh


.PHONY: oscomp-riscv-linuxlike-handoff-audit
oscomp-riscv-linuxlike-handoff-audit:
	@bash scripts/oscomp-riscv-linuxlike-handoff-audit.sh .

.PHONY: oscomp-riscv-early-trap-audit
oscomp-riscv-early-trap-audit:
	@bash scripts/oscomp-riscv-early-trap-audit.sh

.PHONY: oscomp-riscv-post-final-trace-audit
oscomp-riscv-post-final-trace-audit:
	@sh scripts/oscomp-riscv-post-final-trace-audit.sh

.PHONY: oscomp-riscv-chunked-buddy-audit

.PHONY: oscomp-riscv-final-clean-audit oscomp-riscv-buddy-order-audit oscomp-riscv-chunked-buddy-audit
oscomp-riscv-final-clean-audit:
	python3 scripts/oscomp-riscv-final-clean-audit.py
oscomp-riscv-buddy-order-audit:
	python3 scripts/oscomp-riscv-buddy-order-audit.py
oscomp-riscv-chunked-buddy-audit:
	python3 scripts/oscomp-riscv-chunked-buddy-audit.py

.PHONY: oscomp-riscv-allocator-summary-audit
oscomp-riscv-allocator-summary-audit:
	python3 scripts/oscomp-riscv-allocator-summary-audit.py $(CURDIR)

.PHONY: oscomp-riscv-allocator-preinstall-audit
oscomp-riscv-allocator-preinstall-audit:
	python3 scripts/oscomp-riscv-allocator-preinstall-audit.py

.PHONY: oscomp-riscv-allocator-install-first-audit
oscomp-riscv-allocator-install-first-audit:
	python3 scripts/oscomp-riscv-allocator-install-first-audit.py

.PHONY: oscomp-riscv-boot-pagealloc-chain-audit
oscomp-riscv-boot-pagealloc-chain-audit:
	python3 scripts/oscomp-riscv-boot-pagealloc-chain-audit.py

.PHONY: oscomp-riscv-boot-pagealloc-install-audit
oscomp-riscv-boot-pagealloc-install-audit: oscomp-riscv-boot-pagealloc-chain-audit

oscomp-riscv-boot-pagealloc-effective-audit:
	python3 scripts/oscomp-riscv-boot-pagealloc-effective-audit.py

.PHONY: oscomp-riscv-post-install-probe-cleanup-audit
oscomp-riscv-post-install-probe-cleanup-audit:
	python3 scripts/oscomp-riscv-post-install-probe-cleanup-audit.py

oscomp-riscv-boot-to-runtime-big-repair-audit:
	python3 scripts/oscomp-riscv-boot-to-runtime-big-repair-audit.py

.PHONY: oscomp-riscv-kernel-image-gap-audit
oscomp-riscv-kernel-image-gap-audit:
	python3 scripts/oscomp-riscv-kernel-image-gap-audit.py

.PHONY: oscomp-sdcard-test-discovery-exec-audit
oscomp-sdcard-test-discovery-exec-audit:
	python3 scripts/oscomp-sdcard-test-discovery-exec-audit.py

.PHONY: oscomp-sdcard-bounded-discovery-exec-audit
oscomp-sdcard-bounded-discovery-exec-audit:
	python3 scripts/oscomp-sdcard-bounded-discovery-exec-audit.py

.PHONY: oscomp-newtest-p0-abi-audit
oscomp-newtest-p0-abi-audit:
	python3 scripts/oscomp-newtest-p0-abi-audit.py

.PHONY: oscomp-newtest-p2-vfs-audit
oscomp-newtest-p2-vfs-audit:
	python3 scripts/oscomp-newtest-p2-vfs-audit.py

.PHONY: oscomp-newtest-p3-sched-audit
oscomp-newtest-p3-sched-audit:
	python3 scripts/oscomp-newtest-p3-sched-audit.py

.PHONY: oscomp-newtest-p4-dynamic-elf-audit
oscomp-newtest-p4-dynamic-elf-audit:
	python3 scripts/oscomp-newtest-p4-dynamic-elf-audit.py

.PHONY: oscomp-newtest-p5-clone-futex-audit
oscomp-newtest-p5-clone-futex-audit:
	python3 scripts/oscomp-newtest-p5-clone-futex-audit.py

.PHONY: oscomp-newtest-p6-network-audit
oscomp-newtest-p6-network-audit:
	python3 scripts/oscomp-newtest-p6-network-audit.py

.PHONY: oscomp-newtest-full-audit
oscomp-newtest-full-audit: oscomp-newtest-p0-abi-audit oscomp-newtest-p2-vfs-audit oscomp-newtest-p3-sched-audit oscomp-newtest-p4-dynamic-elf-audit oscomp-newtest-p5-clone-futex-audit oscomp-newtest-p6-network-audit
	@echo "newtest full audit: all milestones passed"

.PHONY: oscomp-final-p1-runtime-audit
oscomp-final-p1-runtime-audit:
	python3 scripts/oscomp-final-p1-runtime-audit.py

# ── P9-G7: local contest QEMU targets (require sdcard-rv.img / sdcard-la.img) ──

.PHONY: contest-rv
contest-rv: kernel-rv
	qemu-system-riscv64 \
		-machine virt \
		-kernel kernel-rv \
		-m 1G \
		-nographic \
		-smp 1 \
		-bios default \
		-drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-no-reboot \
		-device virtio-net-device,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc

.PHONY: contest-la
contest-la: kernel-la
	qemu-system-loongarch64 \
		-kernel kernel-la \
		-m 1G \
		-nographic \
		-smp 1 \
		-drive file=sdcard-la.img,if=none,format=raw,id=x0 \
		-device virtio-blk-pci,drive=x0 \
		-no-reboot \
		-device virtio-net-pci,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc

.PHONY: final-cagent-rv
final-cagent-rv: kernel-rv
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/cagent-rv-$(FINAL_RUN_ID).log \
		--success-pattern "#### OS COMP TEST GROUP END cagent-glibc ####" \
		--success-regex '^testcase cagent factorial pass [0-9]+$$' \
		--success-regex '^testcase cagent date pass [0-9]+$$' \
		--success-regex '^testcase cagent network pass [0-9]+$$' \
		--success-regex '^testcase cagent cpu pass [0-9]+$$' \
		--success-regex '^testcase cagent kernel pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-create pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-readwrite pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-directory pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-search pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-usage pass [0-9]+$$' \
		$(FINAL_CAGENT_REJECT_GUARD) \
		--failure-regex '^panicked at .*' --timeout $(FINAL_CAGENT_TIMEOUT) -- qemu-system-riscv64 \
		-machine virt \
		-kernel kernel-rv \
		-m 1G \
		-smp 1 \
		-bios default \
		-drive file=$(FINAL_IMAGE_RV),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-device,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-cagent" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/cagent-rv-$(FINAL_RUN_ID).log

.PHONY: final-cagent-la
final-cagent-la: kernel-la
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/cagent-la-$(FINAL_RUN_ID).log \
		--success-pattern "#### OS COMP TEST GROUP END cagent-glibc ####" \
		--success-regex '^testcase cagent factorial pass [0-9]+$$' \
		--success-regex '^testcase cagent date pass [0-9]+$$' \
		--success-regex '^testcase cagent network pass [0-9]+$$' \
		--success-regex '^testcase cagent cpu pass [0-9]+$$' \
		--success-regex '^testcase cagent kernel pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-create pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-readwrite pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-directory pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-search pass [0-9]+$$' \
		--success-regex '^testcase cagent fs-usage pass [0-9]+$$' \
		$(FINAL_CAGENT_REJECT_GUARD) \
		--failure-regex '^panicked at .*' --timeout $(FINAL_CAGENT_TIMEOUT) -- qemu-system-loongarch64 \
		-kernel kernel-la \
		-m 1G \
		-smp 1 \
		-drive file=$(FINAL_IMAGE_LA),if=none,format=raw,id=x0 \
		-device virtio-blk-pci,drive=x0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-pci,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-cagent" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/cagent-la-$(FINAL_RUN_ID).log

.PHONY: final-buildstorm-rv
final-buildstorm-rv: verify-final-script-sha256 kernel-rv
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/buildstorm-rv-$(FINAL_RUN_ID).log \
		--success-regex '^BUILDSTORM_COMPILE mode=multi ok=true .*cores=8 .*bytes=[1-9][0-9]{5,} .*' \
		--success-pattern "#### OS COMP TEST GROUP END buildstorm-glibc ####" \
		--failure-pattern "BUILDSTORM_TOOLCHAIN fail" --failure-pattern "BUILDSTORM_MINIBUILD fail" \
		--failure-regex '^BUILDSTORM_COMPILE mode=multi ok=false .*' --failure-regex '^panicked at .*' \
		--timeout $(FINAL_BUILDSTORM_TIMEOUT) -- qemu-system-riscv64 \
		-machine virt \
		-kernel kernel-rv \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-bios default \
		-drive file=$(FINAL_IMAGE_RV),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-device,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-buildstorm" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/buildstorm-rv-$(FINAL_RUN_ID).log

.PHONY: final-buildstorm-la
final-buildstorm-la: verify-final-script-sha256 kernel-la
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/buildstorm-la-$(FINAL_RUN_ID).log \
		--success-regex '^BUILDSTORM_COMPILE mode=multi ok=true .*cores=8 .*bytes=[1-9][0-9]{5,} .*' \
		--success-pattern "#### OS COMP TEST GROUP END buildstorm-glibc ####" \
		--failure-pattern "BUILDSTORM_TOOLCHAIN fail" --failure-pattern "BUILDSTORM_MINIBUILD fail" \
		--failure-regex '^BUILDSTORM_COMPILE mode=multi ok=false .*' --failure-regex '^panicked at .*' \
		--timeout $(FINAL_BUILDSTORM_TIMEOUT) -- qemu-system-loongarch64 \
		-kernel kernel-la \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-drive file=$(FINAL_IMAGE_LA),if=none,format=raw,id=x0 \
		-device virtio-blk-pci,drive=x0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-pci,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-buildstorm" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/buildstorm-la-$(FINAL_RUN_ID).log

.PHONY: final-buildstorm-rv-diag
final-buildstorm-rv-diag: kernel-rv
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/buildstorm-rv-diag-$(FINAL_RUN_ID).log \
		--success-pattern "sudoos-diag: final-buildstorm: write preflight ok" \
		--success-pattern "BUILDSTORM_DIAG_NEW_RC=0" \
		--success-pattern "BUILDSTORM_DIAG_BUILD_RC=0" \
		--success-pattern "Hello, world!" \
		--success-pattern "BUILDSTORM_DIAG_RUN_RC=0" \
		--success-pattern "sudoos-diag: final-buildstorm: diagnostic exit=0" \
		--failure-regex '^BUILDSTORM_DIAG_(NEW|BUILD|RUN)_RC=[1-9][0-9]*$$' \
		--failure-regex '^panicked at .*' --timeout $(FINAL_BUILDSTORM_TIMEOUT) -- qemu-system-riscv64 \
		-machine virt \
		-kernel kernel-rv \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-bios default \
		-drive file=$(FINAL_IMAGE_RV),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-device,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-buildstorm-diag" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/buildstorm-rv-diag-$(FINAL_RUN_ID).log

.PHONY: final-lifecycle-rv
final-lifecycle-rv: kernel-rv
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/lifecycle-rv-smp$(FINAL_CPUS)-$(FINAL_RUN_ID).log \
		--success-pattern "G2_LIFECYCLE_STRESS: PASS" \
		--failure-regex '^G2_(PHASE|STEADY_STATE|LIFECYCLE_STRESS).*FAIL.*' \
		--failure-regex '^panicked at .*' --timeout $(FINAL_LIFECYCLE_TIMEOUT) -- qemu-system-riscv64 \
		-machine virt \
		-kernel kernel-rv \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-bios default \
		-drive file=$(FINAL_IMAGE_RV),if=none,format=raw,id=x0 \
		-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
		-snapshot \
		-no-reboot \
		-append "sudoos.oscomp=lifecycle-stress" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/lifecycle-rv-smp$(FINAL_CPUS)-$(FINAL_RUN_ID).log

.PHONY: final-buildstorm-la-diag
final-buildstorm-la-diag: kernel-la
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/buildstorm-la-diag-$(FINAL_RUN_ID).log \
		--success-pattern "sudoos-diag: final-buildstorm: write preflight ok" \
		--success-pattern "BUILDSTORM_DIAG_NEW_RC=0" \
		--success-pattern "BUILDSTORM_DIAG_BUILD_RC=0" \
		--success-pattern "Hello, world!" \
		--success-pattern "BUILDSTORM_DIAG_RUN_RC=0" \
		--success-pattern "sudoos-diag: final-buildstorm: diagnostic exit=0" \
		--failure-regex '^BUILDSTORM_DIAG_(NEW|BUILD|RUN)_RC=[1-9][0-9]*$$' \
		--failure-regex '^panicked at .*' --timeout $(FINAL_BUILDSTORM_TIMEOUT) -- qemu-system-loongarch64 \
		-kernel kernel-la \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-drive file=$(FINAL_IMAGE_LA),if=none,format=raw,id=x0 \
		-device virtio-blk-pci,drive=x0 \
		-snapshot \
		-no-reboot \
		-device virtio-net-pci,netdev=net0 \
		-netdev user,id=net0 \
		-rtc base=utc \
		-append "sudoos.oscomp=final-buildstorm-diag" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/buildstorm-la-diag-$(FINAL_RUN_ID).log

.PHONY: final-lifecycle-la
final-lifecycle-la: kernel-la
	@mkdir -p $(FINAL_LOG_DIR)
	python3 scripts/qemu_log_wait.py --log $(FINAL_LOG_DIR)/lifecycle-la-smp$(FINAL_CPUS)-$(FINAL_RUN_ID).log \
		--success-pattern "G2_LIFECYCLE_STRESS: PASS" \
		--failure-regex '^G2_(PHASE|STEADY_STATE|LIFECYCLE_STRESS).*FAIL.*' \
		--failure-regex '^panicked at .*' --timeout $(FINAL_LIFECYCLE_TIMEOUT) -- qemu-system-loongarch64 \
		-kernel kernel-la \
		-m $(FINAL_MEM) \
		-smp $(FINAL_CPUS) \
		-drive file=$(FINAL_IMAGE_LA),if=none,format=raw,id=x0 \
		-device virtio-blk-pci,drive=x0 \
		-snapshot \
		-no-reboot \
		-append "sudoos.oscomp=lifecycle-stress" \
		-monitor none \
		-display none \
		-serial file:$(FINAL_LOG_DIR)/lifecycle-la-smp$(FINAL_CPUS)-$(FINAL_RUN_ID).log

.PHONY: final-buildstorm-rv-debug-small final-buildstorm-la-debug-small
final-buildstorm-rv-debug-small:
	@$(MAKE) final-buildstorm-rv FINAL_CPUS=1 FINAL_MEM=1G

final-buildstorm-la-debug-small:
	@$(MAKE) final-buildstorm-la FINAL_CPUS=1 FINAL_MEM=1G
