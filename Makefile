# OSKernel2026 submission wrapper.
# The judge runs exactly: make all
# Keep local smoke/stress/soak/QEMU out of `all`.

.PHONY: all oscomp-all oscomp-audit oscomp-vendor oscomp-clean oscomp-local help

all: oscomp-all

oscomp-all:
	@bash scripts/oscomp-build.sh

oscomp-audit:
	@python3 scripts/oscomp-audit.py

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
