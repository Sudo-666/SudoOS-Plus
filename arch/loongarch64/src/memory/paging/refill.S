/*
 * LoongArch TLB refill fast path.
 *
 * The processor enters this vector in direct-address mode, so the vector must
 * not touch the stack or ordinary paged virtual memory.  CSR.TLBRSAVE is the
 * only scratch storage used here.
 */

    .section .text.tlb_refill, "ax", @progbits
    .balign 4096
    .global __loongarch_tlb_refill_entry
    .type __loongarch_tlb_refill_entry, @function
__loongarch_tlb_refill_entry:
    /* Preserve t0/r12 across the refill exception. */
    csrwr       $r12, 0x8b              /* CSR.TLBRSAVE */

    /*
     * CSR.PGD selects PGDL or PGDH from the faulting virtual address.
     * Directory entries are physical addresses; LDDIR/LDPTE therefore keep
     * the walk valid while the refill exception is in direct-address mode.
     */
    csrrd       $r12, 0x1b              /* CSR.PGD */
    lddir       $r12, $r12, 3
    lddir       $r12, $r12, 2
    lddir       $r12, $r12, 1
    ldpte       $r12, 0
    ldpte       $r12, 1
    tlbfill

    csrrd       $r12, 0x8b              /* CSR.TLBRSAVE */
    ertn
    .size __loongarch_tlb_refill_entry, . - __loongarch_tlb_refill_entry

/* Small ABI helper used by Rust to query CPUCFG words. */
    .section .text, "ax", @progbits
    .balign 4
    .global __loongarch_cpucfg_word
    .type __loongarch_cpucfg_word, @function
__loongarch_cpucfg_word:
    cpucfg      $r4, $r4
    jirl        $r0, $r1, 0
    .size __loongarch_cpucfg_word, . - __loongarch_cpucfg_word
