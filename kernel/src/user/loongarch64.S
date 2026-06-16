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
    or $r2, $r0, $r0
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

.section .rodata.m7_user, "a", @progbits
.align 4
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

.globl __m7_user_image_end
__m7_user_image_end:
