.section .text
.balign 4
.globl __m7_enter_user
.type __m7_enter_user, @function

.equ RISCV_USER_ANCHOR_SIZE, 24
.equ RISCV_USER_ANCHOR_KERNEL_SP, 0
.equ RISCV_USER_ANCHOR_KERNEL_TP, 8
.equ RISCV_USER_ANCHOR_USER_TP, 16

/*
 * a0 = user entry, a1 = user stack top.
 *
 * M9-B runs as a schedulable task. While U-mode is active, sscratch
 * points at a task-stack anchor containing the saved kernel SP and kernel tp.
 * Trap entry restores both before entering Rust and preserves user tp as part
 * of the TrapFrame, matching Linux's separation of user TLS from kernel
 * current/per-CPU state.
 */
__m7_enter_user:
    addi sp, sp, -128
    sd ra, 0(sp)
    sd gp, 8(sp)
    sd tp, 16(sp)
    sd s0, 24(sp)
    sd s1, 32(sp)
    sd s2, 40(sp)
    sd s3, 48(sp)
    sd s4, 56(sp)
    sd s5, 64(sp)
    sd s6, 72(sp)
    sd s7, 80(sp)
    sd s8, 88(sp)
    sd s9, 96(sp)
    sd s10, 104(sp)
    sd s11, 112(sp)
    csrr t0, sstatus
    sd t0, 120(sp)

    /*
     * Build the 24-byte entry anchor in the area that the next TrapFrame will
     * occupy.  Trap entry consumes these words before writing stval/guard/pad.
     */
    addi t2, sp, -RISCV_USER_ANCHOR_SIZE
    sd sp, RISCV_USER_ANCHOR_KERNEL_SP(t2)
    sd tp, RISCV_USER_ANCHOR_KERNEL_TP(t2)
    sd zero, RISCV_USER_ANCHOR_USER_TP(t2)
    csrw sscratch, t2
    csrw sepc, a0
    li t1, 0x40102          /* SUM | SPP | SIE */
    not t1, t1
    and t0, t0, t1
    ori t0, t0, 0x20        /* SPIE: enable timer/IPI after sret */
    csrw sstatus, t0

    mv sp, a1
    li ra, 0
    li gp, 0
    li tp, 0
    li t0, 0
    li t1, 0
    li t2, 0
    li s0, 0
    li s1, 0
    li a0, 0
    li a1, 0
    li a2, 0
    li a3, 0
    li a4, 0
    li a5, 0
    li a6, 0
    li a7, 0
    li s2, 0
    li s3, 0
    li s4, 0
    li s5, 0
    li s6, 0
    li s7, 0
    li s8, 0
    li s9, 0
    li s10, 0
    li s11, 0
    li t3, 0
    li t4, 0
    li t5, 0
    li t6, 0
    sret

.globl __m7_user_return
.type __m7_user_return, @function
__m7_user_return:
    csrw sscratch, zero
    ld t0, 120(sp)
    csrw sstatus, t0
    ld ra, 0(sp)
    ld gp, 8(sp)
    ld tp, 16(sp)
    ld s0, 24(sp)
    ld s1, 32(sp)
    ld s2, 40(sp)
    ld s3, 48(sp)
    ld s4, 56(sp)
    ld s5, 64(sp)
    ld s6, 72(sp)
    ld s7, 80(sp)
    ld s8, 88(sp)
    ld s9, 96(sp)
    ld s10, 104(sp)
    ld s11, 112(sp)
    addi sp, sp, 128
    ret
.size __m7_user_return, . - __m7_user_return
.size __m7_enter_user, . - __m7_enter_user

.section .rodata.m7_user, "a", @progbits
.balign 4
.globl __m7_user_image_start
__m7_user_image_start:

.globl __m7_user_success
__m7_user_success:
    .option push
    .option norvc
    li a0, 1
    li a1, 0x401000
    li a2, 11
    li a7, 64
    ecall
    li a0, 0
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m7_user_unknown_syscall
__m7_user_unknown_syscall:
    .option push
    .option norvc
    li a7, 2047
    ecall
    li t0, -38
    bne a0, t0, .Lriscv_unknown_failed
    li a0, 0
    li a7, 93
    ecall
.Lriscv_unknown_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m7_user_bad_pointer
__m7_user_bad_pointer:
    .option push
    .option norvc
    li a0, 1
    li a1, 0x404000
    li a2, 1
    li a7, 64
    ecall
    li t0, -14
    bne a0, t0, .Lriscv_bad_pointer_failed
    li a0, 0
    li a7, 93
    ecall
.Lriscv_bad_pointer_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m7_user_write_code
__m7_user_write_code:
    .option push
    .option norvc
    li t0, 0x400000
    sw zero, 0(t0)
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop


.globl __m8_user_vm
__m8_user_vm:
    .option push
    .option norvc
    li t0, 0x500000
    sd zero, 0(t0)
    li t0, 0x7ff000
    sd zero, 0(t0)

    li a0, 0x601000
    li a7, 214
    ecall
    li t0, 0x601000
    bne a0, t0, .Lriscv_m8_vm_failed
    li t0, 0x600000
    sd zero, 0(t0)
    li a0, 0x600000
    li a7, 214
    ecall
    li t0, 0x600000
    bne a0, t0, .Lriscv_m8_vm_failed

    li a0, 0
    li a1, 4096
    li a2, 3
    li a3, 0x22
    li a4, -1
    li a5, 0
    li a7, 222
    ecall
    li t0, 0x01000000
    bne a0, t0, .Lriscv_m8_vm_failed
    mv s0, a0
    sd zero, 0(s0)

    mv a0, s0
    li a1, 4096
    li a2, 1
    li a7, 226
    ecall
    bnez a0, .Lriscv_m8_vm_failed

    mv a0, s0
    li a1, 4096
    li a7, 215
    ecall
    bnez a0, .Lriscv_m8_vm_failed

    li a0, 0
    li a7, 93
    ecall
.Lriscv_m8_vm_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m8_user_mprotect_fault
__m8_user_mprotect_fault:
    .option push
    .option norvc
    li a0, 0
    li a1, 4096
    li a2, 3
    li a3, 0x22
    li a4, -1
    li a5, 0
    li a7, 222
    ecall
    li t0, 0x01000000
    bne a0, t0, .Lriscv_m8_mprotect_failed
    mv s0, a0
    sd zero, 0(s0)
    mv a0, s0
    li a1, 4096
    li a2, 1
    li a7, 226
    ecall
    bnez a0, .Lriscv_m8_mprotect_failed
    sd zero, 0(s0)
.Lriscv_m8_mprotect_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m8_user_munmap_fault
__m8_user_munmap_fault:
    .option push
    .option norvc
    li a0, 0
    li a1, 4096
    li a2, 3
    li a3, 0x22
    li a4, -1
    li a5, 0
    li a7, 222
    ecall
    li t0, 0x01000000
    bne a0, t0, .Lriscv_m8_munmap_failed
    mv s0, a0
    sd zero, 0(s0)
    mv a0, s0
    li a1, 4096
    li a7, 215
    ecall
    bnez a0, .Lriscv_m8_munmap_failed
    ld t1, 0(s0)
.Lriscv_m8_munmap_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m9_user_sched_yield
__m9_user_sched_yield:
    .option push
    .option norvc
    li t0, 8
.Lriscv_m9_yield_loop:
    li a7, 124
    ecall
    bnez tp, .Lriscv_m9_tp_leaked
    addi t0, t0, -1
    bnez t0, .Lriscv_m9_yield_loop
    li a0, 0
    li a7, 94
    ecall
.Lriscv_m9_tp_leaked:
    li a0, 1
    li a7, 94
    ecall
    ebreak
    .option pop

.globl __m7_user_image_end
__m7_user_image_end:
