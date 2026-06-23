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
