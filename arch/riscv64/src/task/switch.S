.section .text
.align 2

.globl __riscv_switch_context
.type __riscv_switch_context, @function

/*
 * a0 = previous Context*
 * a1 = next Context*
 *
 * Context layout:
 *   ra, sp, s0..s11
 */
__riscv_switch_context:
    sd ra,   0*8(a0)
    sd sp,   1*8(a0)
    sd s0,   2*8(a0)
    sd s1,   3*8(a0)
    sd s2,   4*8(a0)
    sd s3,   5*8(a0)
    sd s4,   6*8(a0)
    sd s5,   7*8(a0)
    sd s6,   8*8(a0)
    sd s7,   9*8(a0)
    sd s8,  10*8(a0)
    sd s9,  11*8(a0)
    sd s10, 12*8(a0)
    sd s11, 13*8(a0)

    ld ra,   0*8(a1)
    ld sp,   1*8(a1)
    ld s0,   2*8(a1)
    ld s1,   3*8(a1)
    ld s2,   4*8(a1)
    ld s3,   5*8(a1)
    ld s4,   6*8(a1)
    ld s5,   7*8(a1)
    ld s6,   8*8(a1)
    ld s7,   9*8(a1)
    ld s8,  10*8(a1)
    ld s9,  11*8(a1)
    ld s10, 12*8(a1)
    ld s11, 13*8(a1)

    ret

.size __riscv_switch_context, . - __riscv_switch_context

    .section .text
    .align 2
    .globl __riscv_fresh_context_entry
    .type __riscv_fresh_context_entry, @function
__riscv_fresh_context_entry:
    # s0 carries the Rust bootstrap entry. Build a mapped ABI caller frame.
    addi sp, sp, -16
    sd zero, 0(sp)
    sd zero, 8(sp)
    jalr ra, 0(s0)
    call kernel_fresh_context_returned
    .4byte 0
    .size __riscv_fresh_context_entry, .-__riscv_fresh_context_entry
