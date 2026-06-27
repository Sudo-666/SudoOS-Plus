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

.globl __m12_enter_user_frame
.type __m12_enter_user_frame, @function
__m12_enter_user_frame:
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

    addi sp, sp, -304
    mv t0, sp
    mv t1, a0
    li t2, 38
.Lriscv_m12_copy_frame:
    ld t3, 0(t1)
    sd t3, 0(t0)
    addi t0, t0, 8
    addi t1, t1, 8
    addi t2, t2, -1
    bnez t2, .Lriscv_m12_copy_frame

    ld t0, 32*8(sp)
    csrw sstatus, t0
    ld t1, 33*8(sp)
    csrw sepc, t1

    addi t1, sp, 304
    sd t1, 35*8(sp)
    sd tp, 36*8(sp)
    sd zero, 37*8(sp)
    addi t1, sp, 35*8
    csrw sscratch, t1

    ld tp, 4*8(sp)
    ld ra, 1*8(sp)
    ld gp, 3*8(sp)
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
.size __m12_enter_user_frame, . - __m12_enter_user_frame

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

.globl __m11_user_vfs
__m11_user_vfs:
    .option push
    .option norvc
    li a0, -100
    li a1, 0x401000
    li a2, 578              /* O_CREAT | O_RDWR | O_TRUNC */
    li a7, 56
    ecall
    li t0, 3
    blt a0, t0, .Lriscv_m11_vfs_failed
    mv s0, a0

    mv a0, s0
    li a1, 0x401020
    li a2, 4
    li a7, 64
    ecall
    li t0, 4
    bne a0, t0, .Lriscv_m11_vfs_failed

    mv a0, s0
    li a1, 0
    li a2, 0
    li a7, 62
    ecall
    bnez a0, .Lriscv_m11_vfs_failed

    mv a0, s0
    li a1, 0x401040
    li a2, 4
    li a7, 63
    ecall
    li t0, 4
    bne a0, t0, .Lriscv_m11_vfs_failed
    li t1, 0x401040
    lbu t0, 0(t1)
    li t2, 117
    bne t0, t2, .Lriscv_m11_vfs_failed
    lbu t0, 1(t1)
    li t2, 118
    bne t0, t2, .Lriscv_m11_vfs_failed
    lbu t0, 2(t1)
    li t2, 102
    bne t0, t2, .Lriscv_m11_vfs_failed
    lbu t0, 3(t1)
    li t2, 115
    bne t0, t2, .Lriscv_m11_vfs_failed

    mv a0, s0
    li a7, 57
    ecall
    bnez a0, .Lriscv_m11_vfs_failed
    li a0, 0
    li a7, 93
    ecall
.Lriscv_m11_vfs_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m12_exec_success
__m12_exec_success:
    .option push
    .option norvc
    li a0, 0
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m12_m13_user_probe
__m12_m13_user_probe:
    .option push
    .option norvc
    li a0, 17               /* SIGCHLD */
    li a1, 0
    li a2, 0
    li a3, 0
    li a4, 0
    li a7, 220
    ecall
    beqz a0, .Lriscv_m12_m13_child
    bltz a0, .Lriscv_m12_m13_failed
    mv s3, a0

    mv a0, s3
    li a1, 0x4010c0
    li a2, 0
    li a3, 0
    li a7, 260
    ecall
    bne a0, s3, .Lriscv_m12_m13_failed
    li t1, 0x4010c0
    lw t0, 0(t1)
    li t2, 42
    bne t0, t2, .Lriscv_m12_m13_failed

    li a0, 0x401020
    li a1, 0
    li a7, 59
    ecall
    bnez a0, .Lriscv_m12_m13_failed
    li t0, 0x401020
    lw s0, 0(t0)
    lw s1, 4(t0)

    mv a0, s1
    li a1, 0x401000
    li a2, 4
    li a7, 64
    ecall
    li t0, 4
    bne a0, t0, .Lriscv_m12_m13_failed

    mv a0, s1
    li a7, 57
    ecall
    bnez a0, .Lriscv_m12_m13_failed

    mv a0, s0
    li a1, 0x401040
    li a2, 4
    li a7, 63
    ecall
    li t0, 4
    bne a0, t0, .Lriscv_m12_m13_failed
    li t1, 0x401040
    lbu t0, 0(t1)
    li t2, 112
    bne t0, t2, .Lriscv_m12_m13_failed
    lbu t0, 1(t1)
    li t2, 105
    bne t0, t2, .Lriscv_m12_m13_failed
    lbu t0, 2(t1)
    li t2, 112
    bne t0, t2, .Lriscv_m12_m13_failed
    lbu t0, 3(t1)
    li t2, 101
    bne t0, t2, .Lriscv_m12_m13_failed

    mv a0, s0
    li a7, 57
    ecall
    bnez a0, .Lriscv_m12_m13_failed

    li a7, 172
    ecall
    blez a0, .Lriscv_m12_m13_failed
    mv s2, a0
    li a7, 155
    ecall
    bne a0, s2, .Lriscv_m12_m13_failed
    li a0, 0
    li a7, 156
    ecall
    bne a0, s2, .Lriscv_m12_m13_failed

    li t1, 0x401120
    la t0, .Lriscv_m12_signal_handler
    sd t0, 0(t1)
    sd zero, 8(t1)
    la t0, .Lriscv_m12_signal_restorer
    sd t0, 16(t1)
    sd zero, 24(t1)
    li t1, 0x401148
    sd zero, 0(t1)
    li a0, 15
    li a1, 0x401120
    li a2, 0
    li a3, 8
    li a7, 134
    ecall
    bnez a0, .Lriscv_m12_m13_failed
    mv a0, s2
    li a1, 15
    li a7, 129
    ecall
    bnez a0, .Lriscv_m12_m13_failed
    li t1, 0x401148
    ld t0, 0(t1)
    li t2, 1
    bne t0, t2, .Lriscv_m12_m13_failed

    li a0, 1
    li a1, 0x401060
    li a7, 113
    ecall
    bnez a0, .Lriscv_m12_m13_failed

    li a0, 0x401080
    li a7, 160
    ecall
    bnez a0, .Lriscv_m12_m13_failed

    li a0, 0x401040
    li a1, 8
    li a2, 0
    li a7, 278
    ecall
    li t0, 8
    bne a0, t0, .Lriscv_m12_m13_failed

    li t0, 0x401100
    li t1, 47
    sb t1, 0(t0)
    li t1, 46
    sb t1, 1(t0)
    li t1, 109
    sb t1, 2(t0)
    li t1, 49
    sb t1, 3(t0)
    li t1, 50
    sb t1, 4(t0)
    sb zero, 5(t0)
    mv a0, t0
    li a1, 0
    li a2, 0
    li a7, 221
    ecall
    j .Lriscv_m12_m13_failed
.Lriscv_m12_m13_child:
    li a0, 42
    li a7, 93
    ecall
.Lriscv_m12_m13_failed:
    li a0, 1
    li a7, 93
    ecall
    ebreak
.Lriscv_m12_signal_handler:
    li t0, 15
    bne a0, t0, .Lriscv_m12_m13_failed
    li t1, 0x401148
    ld t0, 0(t1)
    addi t0, t0, 1
    sd t0, 0(t1)
    ret
.Lriscv_m12_signal_restorer:
    li a7, 139
    ecall
    li a0, 3
    li a7, 93
    ecall
    ebreak
    .option pop

.globl __m7_user_image_end
__m7_user_image_end:
