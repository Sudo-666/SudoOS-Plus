/*
 * QEMU LoongArch secondary CPU entry.
 *
 * QEMU's ROM wait loop jumps here with translation disabled and a physical PC.
 * The boot CPU already published per-hardware-CPU boot data in the high kernel
 * image before sending the boot IPI.
 */
.section .text.secondary_entry, "ax", @progbits
.align 4

.globl __loongarch_secondary_entry
.type __loongarch_secondary_entry, @function

.equ CSR_CRMD, 0x0
.equ CSR_ECFG, 0x4
.equ CSR_DMW0, 0x180
.equ CSR_DMW1, 0x181
.equ CSR_DMW2, 0x182
.equ CSR_DMW3, 0x183
.equ CSR_CPUNUM, 0x20

.equ CRMD_MAPPED_KERNEL, 0xb0
.equ DMW_TEMP_LOW_CACHED, 0x0000000000000011
.equ DMW_UNCACHED_PLV0, 0x8000000000000001
.equ DMW_CACHED_PLV0,   0x9000000000000011
.equ CACHED_BASE, 0x9000000000000000

__loongarch_secondary_entry:
    # Disable local interrupts and all local sources inherited from ROM.
    li.w $r12, 0x4
    csrxchg $r0, $r12, CSR_CRMD
    csrwr $r0, CSR_ECFG

    csrrd $r22, CSR_CPUNUM
    andi $r22, $r22, 0x3ff

    li.d $r12, DMW_TEMP_LOW_CACHED
    csrwr $r12, CSR_DMW0
    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1
    csrwr $r0, CSR_DMW2
    csrwr $r0, CSR_DMW3

    la.pcrel $r12, .Lsecondary_high
    li.d $r13, CACHED_BASE
    or $r12, $r12, $r13

    li.w $r14, CRMD_MAPPED_KERNEL
    csrwr $r14, CSR_CRMD
    dbar 0
    ibar 0
    jirl $r0, $r12, 0

.Lsecondary_high:
    li.d $r12, DMW_UNCACHED_PLV0
    csrwr $r12, CSR_DMW0
    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1
    csrwr $r0, CSR_DMW2
    csrwr $r0, CSR_DMW3
    dbar 0
    ibar 0

    # BootSlot is 64 bytes because of repr(align(64)).
    la.pcrel $r12, __loongarch_secondary_boot_data
    slli.d $r13, $r22, 6
    add.d $r12, $r12, $r13

    ld.d $r3,  $r12, 0   # high bootstrap stack top
    ld.d $r21, $r12, 8   # logical CPU ID
    ld.d $r13, $r12, 16  # high Rust secondary entry
    ld.d $r14, $r12, 24  # publication marker

    beqz $r14, .Lsecondary_fault
    or $r4, $r21, $r0  # logical CPU ID
    or $r5, $r22, $r0  # hardware CPU ID
    jirl $r0, $r13, 0

.Lsecondary_fault:
    idle 0
    b .Lsecondary_fault

.size __loongarch_secondary_entry, . - __loongarch_secondary_entry
