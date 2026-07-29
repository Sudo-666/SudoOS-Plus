.section .text
.align 4
.globl __m7_enter_user
.type __m7_enter_user, @function

/*
 * r4 = user entry, r5 = user stack top.
 * SAVE0 (CSR 0x30) owns the kernel stack pointer while PLV3 is active.
 */
__m7_enter_user:
    addi.d $r3, $r3, -112
    st.d $r1, $r3, 0
    st.d $r2, $r3, 8
    st.d $r21, $r3, 16
    st.d $r22, $r3, 24
    st.d $r23, $r3, 32
    st.d $r24, $r3, 40
    st.d $r25, $r3, 48
    st.d $r26, $r3, 56
    st.d $r27, $r3, 64
    st.d $r28, $r3, 72
    st.d $r29, $r3, 80
    st.d $r30, $r3, 88
    st.d $r31, $r3, 96
    csrrd $r12, 0x0
    st.d $r12, $r3, 104

    ori $r12, $r3, 0
    csrwr $r12, 0x30
    csrwr $r4, 0x6
    addi.d $r12, $r0, 7       /* PPLV=3, PIE=1 */
    csrwr $r12, 0x1
    ori $r3, $r5, 0

    or $r1, $r0, $r0
    /*
     * Keep r2 (tp) as-is: the Thread::tls field is set before entering
     * user mode.  Zeroing tp breaks ld-linux which dereferences tp
     * immediately for GOT/TLS setup.
     */
    or $r4, $r0, $r0
    or $r5, $r0, $r0
    or $r6, $r0, $r0
    or $r7, $r0, $r0
    or $r8, $r0, $r0
    or $r9, $r0, $r0
    or $r10, $r0, $r0
    or $r11, $r0, $r0
    or $r12, $r0, $r0
    or $r13, $r0, $r0
    or $r14, $r0, $r0
    or $r15, $r0, $r0
    or $r16, $r0, $r0
    or $r17, $r0, $r0
    or $r18, $r0, $r0
    or $r19, $r0, $r0
    or $r20, $r0, $r0
    or $r21, $r0, $r0
    or $r22, $r0, $r0
    or $r23, $r0, $r0
    or $r24, $r0, $r0
    or $r25, $r0, $r0
    or $r26, $r0, $r0
    or $r27, $r0, $r0
    or $r28, $r0, $r0
    or $r29, $r0, $r0
    or $r30, $r0, $r0
    or $r31, $r0, $r0
    ertn

.globl __m7_user_return
.type __m7_user_return, @function
__m7_user_return:
    csrwr $r0, 0x30
    ld.d $r12, $r3, 104
    csrwr $r12, 0x0
    ld.d $r1, $r3, 0
    ld.d $r2, $r3, 8
    ld.d $r21, $r3, 16
    ld.d $r22, $r3, 24
    ld.d $r23, $r3, 32
    ld.d $r24, $r3, 40
    ld.d $r25, $r3, 48
    ld.d $r26, $r3, 56
    ld.d $r27, $r3, 64
    ld.d $r28, $r3, 72
    ld.d $r29, $r3, 80
    ld.d $r30, $r3, 88
    ld.d $r31, $r3, 96
    addi.d $r3, $r3, 112
    jirl $r0, $r1, 0
.size __m7_user_return, . - __m7_user_return
.size __m7_enter_user, . - __m7_enter_user

.globl __m12_enter_user_frame
.type __m12_enter_user_frame, @function
__m12_enter_user_frame:
    addi.d $r3, $r3, -112
    st.d $r1, $r3, 0
    st.d $r2, $r3, 8
    st.d $r21, $r3, 16
    st.d $r22, $r3, 24
    st.d $r23, $r3, 32
    st.d $r24, $r3, 40
    st.d $r25, $r3, 48
    st.d $r26, $r3, 56
    st.d $r27, $r3, 64
    st.d $r28, $r3, 72
    st.d $r29, $r3, 80
    st.d $r30, $r3, 88
    st.d $r31, $r3, 96
    csrrd $r12, 0x0
    st.d $r12, $r3, 104

    addi.d $r3, $r3, -304
    ori $r12, $r3, 0
    ori $r13, $r4, 0
    addi.d $r14, $r0, 38
.Lloong_m12_copy_frame:
    ld.d $r15, $r13, 0
    st.d $r15, $r12, 0
    addi.d $r12, $r12, 8
    addi.d $r13, $r13, 8
    addi.d $r14, $r14, -1
    bnez $r14, .Lloong_m12_copy_frame

    ld.d $r12, $r3, 32*8
    csrwr $r12, 0x1
    ld.d $r12, $r3, 33*8
    csrwr $r12, 0x6
    addi.d $r12, $r3, 304
    csrwr $r12, 0x30

    ld.d $r1, $r3, 1*8
    ld.d $r2, $r3, 2*8
    ld.d $r4, $r3, 4*8
    ld.d $r5, $r3, 5*8
    ld.d $r6, $r3, 6*8
    ld.d $r7, $r3, 7*8
    ld.d $r8, $r3, 8*8
    ld.d $r9, $r3, 9*8
    ld.d $r10, $r3, 10*8
    ld.d $r11, $r3, 11*8
    ld.d $r12, $r3, 12*8
    ld.d $r13, $r3, 13*8
    ld.d $r14, $r3, 14*8
    ld.d $r15, $r3, 15*8
    ld.d $r16, $r3, 16*8
    ld.d $r17, $r3, 17*8
    ld.d $r18, $r3, 18*8
    ld.d $r19, $r3, 19*8
    ld.d $r20, $r3, 20*8
    ld.d $r21, $r3, 21*8
    ld.d $r22, $r3, 22*8
    ld.d $r23, $r3, 23*8
    ld.d $r24, $r3, 24*8
    ld.d $r25, $r3, 25*8
    ld.d $r26, $r3, 26*8
    ld.d $r27, $r3, 27*8
    ld.d $r28, $r3, 28*8
    ld.d $r29, $r3, 29*8
    ld.d $r30, $r3, 30*8
    ld.d $r31, $r3, 31*8
    ld.d $r3, $r3, 3*8
    ertn
.size __m12_enter_user_frame, . - __m12_enter_user_frame

.section .rodata.m7_user, "a", @progbits
.align 12
.globl __m7_user_image_start
__m7_user_image_start:

.globl __m7_user_success
__m7_user_success:
  /* Adversarial regression: user state must not become CPU ID. */
  lu12i.w $r21, 0x1000
    addi.d $r4, $r0, 1
    lu12i.w $r5, 0x401
    addi.d $r6, $r0, 11
    addi.d $r11, $r0, 64
    syscall 0
    addi.d $r4, $r0, 0
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m7_user_unknown_syscall
__m7_user_unknown_syscall:
    addi.d $r11, $r0, 2047
    syscall 0
    addi.d $r12, $r0, -38
    bne $r4, $r12, .Lloong_unknown_failed
    addi.d $r4, $r0, 0
    addi.d $r11, $r0, 93
    syscall 0
.Lloong_unknown_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m7_user_bad_pointer
__m7_user_bad_pointer:
    addi.d $r4, $r0, 1
    lu12i.w $r5, 0x404
    addi.d $r6, $r0, 1
    addi.d $r11, $r0, 64
    syscall 0
    addi.d $r12, $r0, -14
    bne $r4, $r12, .Lloong_bad_pointer_failed
    addi.d $r4, $r0, 0
    addi.d $r11, $r0, 93
    syscall 0
.Lloong_bad_pointer_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m7_user_write_code
__m7_user_write_code:
    lu12i.w $r12, 0x400
    st.w $r0, $r12, 0
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0


.globl __m8_user_vm
__m8_user_vm:
    lu12i.w $r12, 0x500
    st.d $r0, $r12, 0
    lu12i.w $r12, 0x7ff
    st.d $r0, $r12, 0

    lu12i.w $r4, 0x601
    addi.d $r11, $r0, 214
    syscall 0
    lu12i.w $r12, 0x601
    bne $r4, $r12, .Lloong_m8_vm_failed
    lu12i.w $r12, 0x600
    st.d $r0, $r12, 0
    lu12i.w $r4, 0x600
    addi.d $r11, $r0, 214
    syscall 0
    lu12i.w $r12, 0x600
    bne $r4, $r12, .Lloong_m8_vm_failed

    or $r4, $r0, $r0
    lu12i.w $r5, 0x1
    addi.d $r6, $r0, 3
    addi.d $r7, $r0, 0x22
    addi.d $r8, $r0, -1
    or $r9, $r0, $r0
    addi.d $r11, $r0, 222
    syscall 0
    lu12i.w $r12, 0x1000
    bne $r4, $r12, .Lloong_m8_vm_failed
    or $r23, $r4, $r0
    st.d $r0, $r23, 0

    or $r4, $r23, $r0
    lu12i.w $r5, 0x1
    addi.d $r6, $r0, 1
    addi.d $r11, $r0, 226
    syscall 0
    bne $r4, $r0, .Lloong_m8_vm_failed

    or $r4, $r23, $r0
    lu12i.w $r5, 0x1
    addi.d $r11, $r0, 215
    syscall 0
    bne $r4, $r0, .Lloong_m8_vm_failed

    or $r4, $r0, $r0
    addi.d $r11, $r0, 93
    syscall 0
.Lloong_m8_vm_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m8_user_mprotect_fault
__m8_user_mprotect_fault:
    or $r4, $r0, $r0
    lu12i.w $r5, 0x1
    addi.d $r6, $r0, 3
    addi.d $r7, $r0, 0x22
    addi.d $r8, $r0, -1
    or $r9, $r0, $r0
    addi.d $r11, $r0, 222
    syscall 0
    lu12i.w $r12, 0x1000
    bne $r4, $r12, .Lloong_m8_mprotect_failed
    or $r23, $r4, $r0
    st.d $r0, $r23, 0
    or $r4, $r23, $r0
    lu12i.w $r5, 0x1
    addi.d $r6, $r0, 1
    addi.d $r11, $r0, 226
    syscall 0
    bne $r4, $r0, .Lloong_m8_mprotect_failed
    st.d $r0, $r23, 0
.Lloong_m8_mprotect_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m8_user_munmap_fault
__m8_user_munmap_fault:
    or $r4, $r0, $r0
    lu12i.w $r5, 0x1
    addi.d $r6, $r0, 3
    addi.d $r7, $r0, 0x22
    addi.d $r8, $r0, -1
    or $r9, $r0, $r0
    addi.d $r11, $r0, 222
    syscall 0
    lu12i.w $r12, 0x1000
    bne $r4, $r12, .Lloong_m8_munmap_failed
    or $r23, $r4, $r0
    st.d $r0, $r23, 0
    or $r4, $r23, $r0
    lu12i.w $r5, 0x1
    addi.d $r11, $r0, 215
    syscall 0
    bne $r4, $r0, .Lloong_m8_munmap_failed
    ld.d $r12, $r23, 0
.Lloong_m8_munmap_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m9_user_sched_yield
__m9_user_sched_yield:
    addi.d $r12, $r0, 8
.Lloong_m9_yield_loop:
    addi.d $r11, $r0, 124
    syscall 0
    addi.d $r12, $r12, -1
    bnez $r12, .Lloong_m9_yield_loop
    addi.d $r4, $r0, 0
    addi.d $r11, $r0, 94
    syscall 0
    break 0

.globl __m11_user_vfs
__m11_user_vfs:
    addi.d $r4, $r0, -100
    lu12i.w $r5, 0x401
    addi.d $r6, $r0, 578       /* O_CREAT | O_RDWR | O_TRUNC */
    addi.d $r11, $r0, 56
    syscall 0
    addi.d $r12, $r0, 3
    blt $r4, $r12, .Lloong_m11_vfs_failed
    or $r23, $r4, $r0

    or $r4, $r23, $r0
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0x20
    addi.d $r6, $r0, 4
    addi.d $r11, $r0, 64
    syscall 0
    addi.d $r12, $r0, 4
    bne $r4, $r12, .Lloong_m11_vfs_failed

    or $r4, $r23, $r0
    or $r5, $r0, $r0
    or $r6, $r0, $r0
    addi.d $r11, $r0, 62
    syscall 0
    bnez $r4, .Lloong_m11_vfs_failed

    or $r4, $r23, $r0
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0x40
    addi.d $r6, $r0, 4
    addi.d $r11, $r0, 63
    syscall 0
    addi.d $r12, $r0, 4
    bne $r4, $r12, .Lloong_m11_vfs_failed
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x40
    ld.bu $r12, $r13, 0
    addi.d $r14, $r0, 117
    bne $r12, $r14, .Lloong_m11_vfs_failed
    ld.bu $r12, $r13, 1
    addi.d $r14, $r0, 118
    bne $r12, $r14, .Lloong_m11_vfs_failed
    ld.bu $r12, $r13, 2
    addi.d $r14, $r0, 102
    bne $r12, $r14, .Lloong_m11_vfs_failed
    ld.bu $r12, $r13, 3
    addi.d $r14, $r0, 115
    bne $r12, $r14, .Lloong_m11_vfs_failed

    or $r4, $r23, $r0
    addi.d $r11, $r0, 57
    syscall 0
    bnez $r4, .Lloong_m11_vfs_failed
    or $r4, $r0, $r0
    addi.d $r11, $r0, 93
    syscall 0
.Lloong_m11_vfs_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m12_exec_success
__m12_exec_success:
    or $r4, $r0, $r0
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m12_m13_user_probe
__m12_m13_user_probe:
    addi.d $r4, $r0, 17        /* SIGCHLD */
    or $r5, $r0, $r0
    or $r6, $r0, $r0
    or $r7, $r0, $r0
    or $r8, $r0, $r0
    addi.d $r11, $r0, 220
    syscall 0
    beqz $r4, .Lloong_m12_m13_child
    blt $r4, $r0, .Lloong_m12_m13_failed
    or $r26, $r4, $r0

    or $r4, $r26, $r0
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0xc0
    or $r6, $r0, $r0
    or $r7, $r0, $r0
    addi.d $r11, $r0, 260
    syscall 0
    bne $r4, $r26, .Lloong_m12_m13_failed
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0xc0
    ld.w $r12, $r13, 0
    addi.d $r14, $r0, 42
    bne $r12, $r14, .Lloong_m12_m13_failed

    lu12i.w $r4, 0x401
    addi.d $r4, $r4, 0x20
    or $r5, $r0, $r0
    addi.d $r11, $r0, 59
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed
    lu12i.w $r12, 0x401
    addi.d $r12, $r12, 0x20
    ld.w $r23, $r12, 0
    ld.w $r24, $r12, 4

    or $r4, $r24, $r0
    lu12i.w $r5, 0x401
    addi.d $r6, $r0, 4
    addi.d $r11, $r0, 64
    syscall 0
    addi.d $r12, $r0, 4
    bne $r4, $r12, .Lloong_m12_m13_failed

    or $r4, $r24, $r0
    addi.d $r11, $r0, 57
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed

    or $r4, $r23, $r0
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0x40
    addi.d $r6, $r0, 4
    addi.d $r11, $r0, 63
    syscall 0
    addi.d $r12, $r0, 4
    bne $r4, $r12, .Lloong_m12_m13_failed
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x40
    ld.bu $r12, $r13, 0
    addi.d $r14, $r0, 112
    bne $r12, $r14, .Lloong_m12_m13_failed
    ld.bu $r12, $r13, 1
    addi.d $r14, $r0, 105
    bne $r12, $r14, .Lloong_m12_m13_failed
    ld.bu $r12, $r13, 2
    addi.d $r14, $r0, 112
    bne $r12, $r14, .Lloong_m12_m13_failed
    ld.bu $r12, $r13, 3
    addi.d $r14, $r0, 101
    bne $r12, $r14, .Lloong_m12_m13_failed

    or $r4, $r23, $r0
    addi.d $r11, $r0, 57
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed

    addi.d $r11, $r0, 172
    syscall 0
    bge $r0, $r4, .Lloong_m12_m13_failed
    or $r25, $r4, $r0
    addi.d $r11, $r0, 155
    syscall 0
    bne $r4, $r25, .Lloong_m12_m13_failed
    or $r4, $r0, $r0
    addi.d $r11, $r0, 156
    syscall 0
    bne $r4, $r25, .Lloong_m12_m13_failed

    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x120
    la.local $r12, .Lloong_m12_signal_handler
    st.d $r12, $r13, 0
    st.d $r0, $r13, 8
    la.local $r12, .Lloong_m12_signal_restorer
    st.d $r12, $r13, 16
    st.d $r0, $r13, 24
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x148
    st.d $r0, $r13, 0
    addi.d $r4, $r0, 15
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0x120
    or $r6, $r0, $r0
    addi.d $r7, $r0, 8
    addi.d $r11, $r0, 134
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed
    or $r4, $r25, $r0
    addi.d $r5, $r0, 15
    addi.d $r11, $r0, 129
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x148
    ld.d $r12, $r13, 0
    addi.d $r14, $r0, 1
    bne $r12, $r14, .Lloong_m12_m13_failed

    addi.d $r4, $r0, 1
    lu12i.w $r5, 0x401
    addi.d $r5, $r5, 0x60
    addi.d $r11, $r0, 113
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed

    lu12i.w $r4, 0x401
    addi.d $r4, $r4, 0x80
    addi.d $r11, $r0, 160
    syscall 0
    bnez $r4, .Lloong_m12_m13_failed

    lu12i.w $r4, 0x401
    addi.d $r4, $r4, 0x40
    addi.d $r5, $r0, 8
    or $r6, $r0, $r0
    addi.d $r11, $r0, 278
    syscall 0
    addi.d $r12, $r0, 8
    bne $r4, $r12, .Lloong_m12_m13_failed

    lu12i.w $r12, 0x401
    addi.d $r12, $r12, 0x100
    addi.d $r13, $r0, 47
    st.b $r13, $r12, 0
    addi.d $r13, $r0, 46
    st.b $r13, $r12, 1
    addi.d $r13, $r0, 109
    st.b $r13, $r12, 2
    addi.d $r13, $r0, 49
    st.b $r13, $r12, 3
    addi.d $r13, $r0, 50
    st.b $r13, $r12, 4
    st.b $r0, $r12, 5
    or $r4, $r12, $r0
    or $r5, $r0, $r0
    or $r6, $r0, $r0
    addi.d $r11, $r0, 221
    syscall 0
    b .Lloong_m12_m13_failed
.Lloong_m12_m13_child:
    addi.d $r4, $r0, 42
    addi.d $r11, $r0, 93
    syscall 0
.Lloong_m12_m13_failed:
    addi.d $r4, $r0, 1
    addi.d $r11, $r0, 93
    syscall 0
    break 0
.Lloong_m12_signal_handler:
    addi.d $r12, $r0, 15
    bne $r4, $r12, .Lloong_m12_m13_failed
    lu12i.w $r13, 0x401
    addi.d $r13, $r13, 0x148
    ld.d $r12, $r13, 0
    addi.d $r12, $r12, 1
    st.d $r12, $r13, 0
    jirl $r0, $r1, 0
.Lloong_m12_signal_restorer:
    addi.d $r11, $r0, 139
    syscall 0
    addi.d $r4, $r0, 3
    addi.d $r11, $r0, 93
    syscall 0
    break 0

.globl __m7_user_image_end
__m7_user_image_end:

/* SIGNAL_EXTENDED_STATE_V1 */
.section .text
.align 4
.globl __sudoos_loongarch_save_signal_extended_state
.type __sudoos_loongarch_save_signal_extended_state, @function
/*
 * r4 = LoongArchSignalExtendedState*
 *
 * Layout:
 *   0..511   vr0..vr31 (32 x 16 bytes)
 *   512      fcsr0
 *   520..583 fcc0..fcc7
 */
__sudoos_loongarch_save_signal_extended_state:
    csrrd $r12, 0x2
    ori $r12, $r12, 0x3
    csrwr $r12, 0x2

    vst $vr0, $r4, 0
    vst $vr1, $r4, 16
    vst $vr2, $r4, 32
    vst $vr3, $r4, 48
    vst $vr4, $r4, 64
    vst $vr5, $r4, 80
    vst $vr6, $r4, 96
    vst $vr7, $r4, 112
    vst $vr8, $r4, 128
    vst $vr9, $r4, 144
    vst $vr10, $r4, 160
    vst $vr11, $r4, 176
    vst $vr12, $r4, 192
    vst $vr13, $r4, 208
    vst $vr14, $r4, 224
    vst $vr15, $r4, 240
    vst $vr16, $r4, 256
    vst $vr17, $r4, 272
    vst $vr18, $r4, 288
    vst $vr19, $r4, 304
    vst $vr20, $r4, 320
    vst $vr21, $r4, 336
    vst $vr22, $r4, 352
    vst $vr23, $r4, 368
    vst $vr24, $r4, 384
    vst $vr25, $r4, 400
    vst $vr26, $r4, 416
    vst $vr27, $r4, 432
    vst $vr28, $r4, 448
    vst $vr29, $r4, 464
    vst $vr30, $r4, 480
    vst $vr31, $r4, 496

    movfcsr2gr $r12, $fcsr0
    st.d $r12, $r4, 512
    movcf2gr $r12, $fcc0
    st.d $r12, $r4, 520
    movcf2gr $r12, $fcc1
    st.d $r12, $r4, 528
    movcf2gr $r12, $fcc2
    st.d $r12, $r4, 536
    movcf2gr $r12, $fcc3
    st.d $r12, $r4, 544
    movcf2gr $r12, $fcc4
    st.d $r12, $r4, 552
    movcf2gr $r12, $fcc5
    st.d $r12, $r4, 560
    movcf2gr $r12, $fcc6
    st.d $r12, $r4, 568
    movcf2gr $r12, $fcc7
    st.d $r12, $r4, 576

    jirl $r0, $r1, 0
.size __sudoos_loongarch_save_signal_extended_state, . - __sudoos_loongarch_save_signal_extended_state

.align 4
.globl __sudoos_loongarch_restore_signal_extended_state
.type __sudoos_loongarch_restore_signal_extended_state, @function
__sudoos_loongarch_restore_signal_extended_state:
    csrrd $r12, 0x2
    ori $r12, $r12, 0x3
    csrwr $r12, 0x2

    vld $vr0, $r4, 0
    vld $vr1, $r4, 16
    vld $vr2, $r4, 32
    vld $vr3, $r4, 48
    vld $vr4, $r4, 64
    vld $vr5, $r4, 80
    vld $vr6, $r4, 96
    vld $vr7, $r4, 112
    vld $vr8, $r4, 128
    vld $vr9, $r4, 144
    vld $vr10, $r4, 160
    vld $vr11, $r4, 176
    vld $vr12, $r4, 192
    vld $vr13, $r4, 208
    vld $vr14, $r4, 224
    vld $vr15, $r4, 240
    vld $vr16, $r4, 256
    vld $vr17, $r4, 272
    vld $vr18, $r4, 288
    vld $vr19, $r4, 304
    vld $vr20, $r4, 320
    vld $vr21, $r4, 336
    vld $vr22, $r4, 352
    vld $vr23, $r4, 368
    vld $vr24, $r4, 384
    vld $vr25, $r4, 400
    vld $vr26, $r4, 416
    vld $vr27, $r4, 432
    vld $vr28, $r4, 448
    vld $vr29, $r4, 464
    vld $vr30, $r4, 480
    vld $vr31, $r4, 496

    ld.d $r12, $r4, 512
    movgr2fcsr $fcsr0, $r12
    ld.d $r12, $r4, 520
    movgr2cf $fcc0, $r12
    ld.d $r12, $r4, 528
    movgr2cf $fcc1, $r12
    ld.d $r12, $r4, 536
    movgr2cf $fcc2, $r12
    ld.d $r12, $r4, 544
    movgr2cf $fcc3, $r12
    ld.d $r12, $r4, 552
    movgr2cf $fcc4, $r12
    ld.d $r12, $r4, 560
    movgr2cf $fcc5, $r12
    ld.d $r12, $r4, 568
    movgr2cf $fcc6, $r12
    ld.d $r12, $r4, 576
    movgr2cf $fcc7, $r12

    jirl $r0, $r1, 0
.size __sudoos_loongarch_restore_signal_extended_state, . - __sudoos_loongarch_restore_signal_extended_state

