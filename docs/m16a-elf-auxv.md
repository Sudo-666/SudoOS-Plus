# M16-A ELF metadata and auxv front half

<!-- SUDOOS_M16A_ELF_AUXV_PATCH_V1 -->

This patch is deliberately fail-closed.

Completed in this patch:

- ELF parsing accepts `ET_EXEC` and `ET_DYN`.
- Program headers expose `PT_INTERP`, `PT_PHDR`, and `PT_DYNAMIC` metadata.
- `ET_DYN` segments are adjusted through an explicit load-bias policy.
- VMA overlap is rejected during ELF load planning.
- Initial user stack now follows the Linux shape: `argc`, `argv[]`, NULL,
  empty `envp[]`, NULL, `auxv[]`, strings, `AT_RANDOM`, and `AT_EXECFN`.
- Auxv includes `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_BASE`, `AT_ENTRY`,
  `AT_PAGESZ`, `AT_RANDOM`, and `AT_EXECFN`.

Still intentionally incomplete:

- `PT_INTERP` is not loaded yet.
- dynamic relocations are not applied yet.
- TLS setup is not implemented yet.
- RELRO/mprotect sealing is not implemented yet.
- `AT_RANDOM` currently uses a deterministic fallback seed until a real kernel
  RNG exists; do not treat it as a security boundary.
- persistent ext4 is still a separate M15 blocker.

The important design point is Linux-like failure behavior: after this patch the
kernel can identify dynamic binaries and prepare loader metadata, but `exec`
returns `DynamicInterpreterUnsupported` rather than jumping into a binary that
needs an interpreter, relocations, or TLS.
