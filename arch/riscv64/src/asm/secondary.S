/*
 * SBI HSM secondary-hart entry.
 *
 * Firmware enters with satp=0, sstatus.SIE=0, a0=hart ID and a1=the physical
 * address of SecondaryBootData.
 */
.section .smp.trampoline, "ax", @progbits
.align 2

.globl __riscv_secondary_entry
.type __riscv_secondary_entry, @function
__riscv_secondary_entry:
    mv t5, a0       # firmware hardware hart ID

    csrci sstatus, 2
    csrw sie, zero
    csrw sscratch, zero

    # Pair with CPU0's release fence before consuming boot data.
    fence rw, rw

    # Load every value needed after translation is enabled.
    ld t0, 0(a1)      # satp
    ld t1, 8(a1)      # high virtual stack top
    ld t2, 16(a1)     # logical CPU ID
    ld t3, 24(a1)     # high Rust entry
    ld t4, 32(a1)     # high global pointer

    sfence.vma zero, zero
    csrw satp, t0
    sfence.vma zero, zero
    fence.i

    mv sp, t1
    andi sp, sp, -16
    mv tp, t2
    mv gp, t4
    mv a0, t2       # logical CPU ID
    mv a1, t5       # hardware hart ID

    jr t3

.size __riscv_secondary_entry, . - __riscv_secondary_entry

# Keep the low trampoline address in high-half read-only data. The .dword uses
# an unrestricted R_RISCV_64 relocation, while Rust only PC-relatively accesses
# this nearby object.
.section .rodata.smp, "a", @progbits
.align 3
.globl __riscv_secondary_entry_phys
.type __riscv_secondary_entry_phys, @object
__riscv_secondary_entry_phys:
    .dword __riscv_secondary_entry
.size __riscv_secondary_entry_phys, . - __riscv_secondary_entry_phys

.globl __riscv_smp_trampoline_start_phys
.type __riscv_smp_trampoline_start_phys, @object
__riscv_smp_trampoline_start_phys:
    .dword __smp_trampoline_start
.size __riscv_smp_trampoline_start_phys, . - __riscv_smp_trampoline_start_phys

.globl __riscv_smp_trampoline_end_phys
.type __riscv_smp_trampoline_end_phys, @object
__riscv_smp_trampoline_end_phys:
    .dword __smp_trampoline_end
.size __riscv_smp_trampoline_end_phys, . - __riscv_smp_trampoline_end_phys
