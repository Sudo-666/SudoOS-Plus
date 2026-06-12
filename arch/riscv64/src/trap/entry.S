.section .text
.align 2

.globl __riscv_trap_entry
.type __riscv_trap_entry, @function

/*
 * TrapFrame layout (304 bytes, 16-byte aligned):
 *
 *   gpr[0..31], sstatus, sepc, scause, stval, guard, padding
 *
 * sscratch invariant:
 *
 *   - while executing kernel code: sscratch == 0
 *   - while executing user code:   sscratch == kernel stack top
 *
 * A trap from user swaps the user SP into sscratch and starts on the
 * kernel stack.  sscratch is then cleared before calling Rust so a nested
 * kernel trap cannot mistake a stale value for a kernel stack pointer.
 */
__riscv_trap_entry:
    csrrw sp, sscratch, sp
    bnez sp, .Lriscv_from_user

    /* Kernel trap: recover the interrupted SP and restore the invariant. */
    csrr sp, sscratch
    csrw sscratch, zero
    j .Lriscv_allocate_frame

.Lriscv_from_user:
    /* sp is now the task's kernel stack top; sscratch contains user SP. */

.Lriscv_allocate_frame:
    addi sp, sp, -304

    sd ra, 1*8(sp)
    sd gp, 3*8(sp)
    sd tp, 4*8(sp)
    sd t0, 5*8(sp)
    sd t1, 6*8(sp)
    sd t2, 7*8(sp)
    sd s0, 8*8(sp)
    sd s1, 9*8(sp)
    sd a0, 10*8(sp)
    sd a1, 11*8(sp)
    sd a2, 12*8(sp)
    sd a3, 13*8(sp)
    sd a4, 14*8(sp)
    sd a5, 15*8(sp)
    sd a6, 16*8(sp)
    sd a7, 17*8(sp)
    sd s2, 18*8(sp)
    sd s3, 19*8(sp)
    sd s4, 20*8(sp)
    sd s5, 21*8(sp)
    sd s6, 22*8(sp)
    sd s7, 23*8(sp)
    sd s8, 24*8(sp)
    sd s9, 25*8(sp)
    sd s10, 26*8(sp)
    sd s11, 27*8(sp)
    sd t3, 28*8(sp)
    sd t4, 29*8(sp)
    sd t5, 30*8(sp)
    sd t6, 31*8(sp)

    /* sscratch is non-zero only for a trap that originated in user mode. */
    csrr t0, sscratch
    beqz t0, .Lriscv_save_kernel_sp

    sd t0, 2*8(sp)
    csrw sscratch, zero
    j .Lriscv_save_csrs

.Lriscv_save_kernel_sp:
    addi t0, sp, 304
    sd t0, 2*8(sp)

.Lriscv_save_csrs:
    sd zero, 0*8(sp)

    csrr t0, sstatus
    sd t0, 32*8(sp)

    csrr t0, sepc
    sd t0, 33*8(sp)

    csrr t0, scause
    sd t0, 34*8(sp)

    csrr t0, stval
    sd t0, 35*8(sp)

    li t0, 0x5a5
    sd t0, 36*8(sp)
    sd zero, 37*8(sp)

    mv a0, sp
    call kernel_riscv_trap

    ld t0, 36*8(sp)
    li t1, 0x5a5
    bne t0, t1, .Lriscv_frame_corrupted

    /*
     * Restore the sscratch invariant before restoring temporary registers.
     * SPP=0 means sret will return to user mode.
     */
    ld t0, 32*8(sp)
    csrw sstatus, t0

    ld t1, 33*8(sp)
    csrw sepc, t1

    andi t1, t0, 0x100
    bnez t1, .Lriscv_return_to_kernel

    addi t1, sp, 304
    csrw sscratch, t1
    j .Lriscv_restore_registers

.Lriscv_return_to_kernel:
    csrw sscratch, zero

.Lriscv_restore_registers:
    ld ra, 1*8(sp)
    ld gp, 3*8(sp)
    ld tp, 4*8(sp)
    ld t0, 5*8(sp)
    ld t1, 6*8(sp)
    ld t2, 7*8(sp)
    ld s0, 8*8(sp)
    ld s1, 9*8(sp)
    ld a0, 10*8(sp)
    ld a1, 11*8(sp)
    ld a2, 12*8(sp)
    ld a3, 13*8(sp)
    ld a4, 14*8(sp)
    ld a5, 15*8(sp)
    ld a6, 16*8(sp)
    ld a7, 17*8(sp)
    ld s2, 18*8(sp)
    ld s3, 19*8(sp)
    ld s4, 20*8(sp)
    ld s5, 21*8(sp)
    ld s6, 22*8(sp)
    ld s7, 23*8(sp)
    ld s8, 24*8(sp)
    ld s9, 25*8(sp)
    ld s10, 26*8(sp)
    ld s11, 27*8(sp)
    ld t3, 28*8(sp)
    ld t4, 29*8(sp)
    ld t5, 30*8(sp)
    ld t6, 31*8(sp)

    ld sp, 2*8(sp)
    sret

.Lriscv_frame_corrupted:
    mv a0, sp
    call kernel_trap_frame_corrupted
    .4byte 0

.size __riscv_trap_entry, . - __riscv_trap_entry

/*
 * Exercises caller-saved registers, one callee-saved register, RA and SP
 * across a real trap/return round trip. Returns 1 on success and 0 on error.
 */
.globl __riscv_trap_register_self_test
.type __riscv_trap_register_self_test, @function
__riscv_trap_register_self_test:
    addi sp, sp, -32
    sd ra, 0(sp)
    sd s0, 8(sp)
    sd s1, 16(sp)

    mv s1, sp

    li ra, 0x101
    li t0, 0x111
    li t1, 0x122
    li t2, 0x133
    li t3, 0x144
    li t4, 0x155
    li t5, 0x166
    li t6, 0x177
    li a1, 0x211
    li a2, 0x222
    li a3, 0x233
    li a4, 0x244
    li a5, 0x255
    li a6, 0x266
    li a7, 0x277
    li s0, 0x311

    .option push
    .option norvc
    .4byte 0x00100073
    .option pop

    bne sp, s1, .Lriscv_register_test_failed

    li a0, 0x101
    bne ra, a0, .Lriscv_register_test_failed
    li a0, 0x111
    bne t0, a0, .Lriscv_register_test_failed
    li a0, 0x122
    bne t1, a0, .Lriscv_register_test_failed
    li a0, 0x133
    bne t2, a0, .Lriscv_register_test_failed
    li a0, 0x144
    bne t3, a0, .Lriscv_register_test_failed
    li a0, 0x155
    bne t4, a0, .Lriscv_register_test_failed
    li a0, 0x166
    bne t5, a0, .Lriscv_register_test_failed
    li a0, 0x177
    bne t6, a0, .Lriscv_register_test_failed
    li a0, 0x211
    bne a1, a0, .Lriscv_register_test_failed
    li a0, 0x222
    bne a2, a0, .Lriscv_register_test_failed
    li a0, 0x233
    bne a3, a0, .Lriscv_register_test_failed
    li a0, 0x244
    bne a4, a0, .Lriscv_register_test_failed
    li a0, 0x255
    bne a5, a0, .Lriscv_register_test_failed
    li a0, 0x266
    bne a6, a0, .Lriscv_register_test_failed
    li a0, 0x277
    bne a7, a0, .Lriscv_register_test_failed
    li a0, 0x311
    bne s0, a0, .Lriscv_register_test_failed

    li a0, 1
    j .Lriscv_register_test_done

.Lriscv_register_test_failed:
    li a0, 0

.Lriscv_register_test_done:
    ld ra, 0(sp)
    ld s0, 8(sp)
    ld s1, 16(sp)
    addi sp, sp, 32
    ret

.size __riscv_trap_register_self_test, . - __riscv_trap_register_self_test
