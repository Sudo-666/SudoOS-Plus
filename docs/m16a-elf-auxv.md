# M16-A ELF metadata and auxv front half

<!-- SUDOOS_M16A_ELF_AUXV_PATCH_V1 -->

This patch is deliberately fail-closed for interpreter-backed dynamic binaries.

Completed in this patch:

- ELF parsing accepts `ET_EXEC` and `ET_DYN`.
- Program headers expose `PT_INTERP`, `PT_PHDR`, and `PT_DYNAMIC` metadata.
- `ET_DYN` segments are adjusted through an explicit load-bias policy.
- VMA overlap is rejected during ELF load planning.
- Initial user stack now follows the Linux shape: `argc`, `argv[]`, NULL,
  empty `envp[]`, NULL, `auxv[]`, strings, `AT_RANDOM`, and `AT_EXECFN`.
- Auxv includes `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`, `AT_BASE`, `AT_ENTRY`,
  `AT_PAGESZ`, `AT_RANDOM`, and `AT_EXECFN`.
- `execve` copies real `argv[]` and `envp[]` from user memory and lays every
  string out on the initial stack.
- No-interpreter static PIE binaries can use `R_RELATIVE` RELA relocations;
  this is enough for the vendor RISC-V static PIE BusyBox smoke gate.

Still intentionally incomplete:

- `PT_INTERP` is not loaded yet.
- interpreter-backed dynamic relocation, symbol lookup, and shared-object
  loading are not implemented yet.
- TLS setup is not implemented yet.
- RELRO/mprotect sealing is not implemented yet.
- `AT_RANDOM` currently uses a deterministic fallback seed until a real kernel
  RNG exists; do not treat it as a security boundary.
- persistent ext4 is still a separate M15 blocker.

The important design point is Linux-like failure behavior: after this patch the
kernel can identify dynamic binaries and prepare loader metadata, but `exec`
returns `DynamicInterpreterUnsupported` rather than jumping into a binary that
needs an interpreter, symbol relocations, shared libraries, or TLS.
