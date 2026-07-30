.section .text
.align 2

.globl __riscv_switch_context
.type __riscv_switch_context, @function

/*
 * a0 = previous Context*
 * a1 = next Context*
 *
 * Context layout:
 *   0..104    ra, sp, s0..s11
 *   112..360  f0..f31
 *   368       fcsr
 *
 * The final-2026 RISC-V userland uses the hard-float lp64d ABI.  The kernel
 * is compiled without F/D instructions, but user FP state remains live while
 * the kernel schedules a task.  Enable FS and carry the complete FP register
 * file at every context switch so an SMP migration cannot mix rustc tasks.
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

    /* sstatus.FS = Dirty, permitting the explicit FP save/restore below. */
    li t0, 0x6000
    csrs sstatus, t0

    .option push
    .option arch, +f, +d
    fsd f0,  14*8(a0)
    fsd f1,  15*8(a0)
    fsd f2,  16*8(a0)
    fsd f3,  17*8(a0)
    fsd f4,  18*8(a0)
    fsd f5,  19*8(a0)
    fsd f6,  20*8(a0)
    fsd f7,  21*8(a0)
    fsd f8,  22*8(a0)
    fsd f9,  23*8(a0)
    fsd f10, 24*8(a0)
    fsd f11, 25*8(a0)
    fsd f12, 26*8(a0)
    fsd f13, 27*8(a0)
    fsd f14, 28*8(a0)
    fsd f15, 29*8(a0)
    fsd f16, 30*8(a0)
    fsd f17, 31*8(a0)
    fsd f18, 32*8(a0)
    fsd f19, 33*8(a0)
    fsd f20, 34*8(a0)
    fsd f21, 35*8(a0)
    fsd f22, 36*8(a0)
    fsd f23, 37*8(a0)
    fsd f24, 38*8(a0)
    fsd f25, 39*8(a0)
    fsd f26, 40*8(a0)
    fsd f27, 41*8(a0)
    fsd f28, 42*8(a0)
    fsd f29, 43*8(a0)
    fsd f30, 44*8(a0)
    fsd f31, 45*8(a0)
    csrr t0, fcsr
    sd t0, 46*8(a0)

    fld f0,  14*8(a1)
    fld f1,  15*8(a1)
    fld f2,  16*8(a1)
    fld f3,  17*8(a1)
    fld f4,  18*8(a1)
    fld f5,  19*8(a1)
    fld f6,  20*8(a1)
    fld f7,  21*8(a1)
    fld f8,  22*8(a1)
    fld f9,  23*8(a1)
    fld f10, 24*8(a1)
    fld f11, 25*8(a1)
    fld f12, 26*8(a1)
    fld f13, 27*8(a1)
    fld f14, 28*8(a1)
    fld f15, 29*8(a1)
    fld f16, 30*8(a1)
    fld f17, 31*8(a1)
    fld f18, 32*8(a1)
    fld f19, 33*8(a1)
    fld f20, 34*8(a1)
    fld f21, 35*8(a1)
    fld f22, 36*8(a1)
    fld f23, 37*8(a1)
    fld f24, 38*8(a1)
    fld f25, 39*8(a1)
    fld f26, 40*8(a1)
    fld f27, 41*8(a1)
    fld f28, 42*8(a1)
    fld f29, 43*8(a1)
    fld f30, 44*8(a1)
    fld f31, 45*8(a1)
    ld t0, 46*8(a1)
    csrw fcsr, t0
    .option pop

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
    # Fresh SP was validated before publication; touch no stack here.
jalr ra, 0(s0)
    call kernel_fresh_context_returned
    .4byte 0
    .size __riscv_fresh_context_entry, .-__riscv_fresh_context_entry
