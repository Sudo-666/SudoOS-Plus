.section .text
.align 4

.globl __loongarch_trap_entry
.type __loongarch_trap_entry, @function

/*
 * r0  = zero
 * r1  = ra
 * r2  = tp
 * r3  = sp
 * r4..r11  = a0..a7
 * r12..r20 = t0..t8
 * r21 = u0
 * r22 = fp
 * r23..r31 = s0..s8
 *
 * TrapFrame is 304 bytes and 16-byte aligned.
 * User-mode entry is deliberately not enabled yet: a future task layer must
 * install a per-task kernel stack before PLV3 execution is allowed.
 */
__loongarch_trap_entry:
    addi.d $r3, $r3, -304

    st.d $r1, $r3, 1*8
    st.d $r2, $r3, 2*8
    st.d $r4, $r3, 4*8
    st.d $r5, $r3, 5*8
    st.d $r6, $r3, 6*8
    st.d $r7, $r3, 7*8
    st.d $r8, $r3, 8*8
    st.d $r9, $r3, 9*8
    st.d $r10, $r3, 10*8
    st.d $r11, $r3, 11*8
    st.d $r12, $r3, 12*8
    st.d $r13, $r3, 13*8
    st.d $r14, $r3, 14*8
    st.d $r15, $r3, 15*8
    st.d $r16, $r3, 16*8
    st.d $r17, $r3, 17*8
    st.d $r18, $r3, 18*8
    st.d $r19, $r3, 19*8
    st.d $r20, $r3, 20*8
    st.d $r21, $r3, 21*8
    st.d $r22, $r3, 22*8
    st.d $r23, $r3, 23*8
    st.d $r24, $r3, 24*8
    st.d $r25, $r3, 25*8
    st.d $r26, $r3, 26*8
    st.d $r27, $r3, 27*8
    st.d $r28, $r3, 28*8
    st.d $r29, $r3, 29*8
    st.d $r30, $r3, 30*8
    st.d $r31, $r3, 31*8

    addi.d $r12, $r3, 304
    st.d $r12, $r3, 3*8

    st.d $r0, $r3, 0*8

    csrrd $r12, 0x1
    st.d $r12, $r3, 32*8

    csrrd $r12, 0x6
    st.d $r12, $r3, 33*8

    csrrd $r12, 0x5
    st.d $r12, $r3, 34*8

    csrrd $r12, 0x7
    st.d $r12, $r3, 35*8

    csrrd $r12, 0x8
    st.d $r12, $r3, 36*8

    addi.d $r12, $r0, 0x5a5
    st.d $r12, $r3, 37*8

    ori $r4, $r3, 0
    bl kernel_loongarch_trap

    ld.d $r12, $r3, 37*8
    addi.d $r13, $r0, 0x5a5
    bne $r12, $r13, .Lloongarch_frame_corrupted

    ld.d $r12, $r3, 32*8
    csrwr $r12, 0x1

    ld.d $r12, $r3, 33*8
    csrwr $r12, 0x6

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

.Lloongarch_frame_corrupted:
    ori $r4, $r3, 0
    bl kernel_trap_frame_corrupted
    break 0

.size __loongarch_trap_entry, . - __loongarch_trap_entry

/* Returns 1 when register state survives a real break/ertn round trip. */
.globl __loongarch_trap_register_self_test
.type __loongarch_trap_register_self_test, @function
__loongarch_trap_register_self_test:
    addi.d $r3, $r3, -32
    st.d $r1, $r3, 0
    st.d $r23, $r3, 8
    st.d $r24, $r3, 16

    addi.d $r24, $r3, 0

    addi.d $r1, $r0, 0x101
    addi.d $r12, $r0, 0x111
    addi.d $r13, $r0, 0x122
    addi.d $r14, $r0, 0x133
    addi.d $r15, $r0, 0x144
    addi.d $r16, $r0, 0x155
    addi.d $r17, $r0, 0x166
    addi.d $r18, $r0, 0x177
    addi.d $r19, $r0, 0x188
    addi.d $r20, $r0, 0x199
    addi.d $r5, $r0, 0x211
    addi.d $r6, $r0, 0x222
    addi.d $r7, $r0, 0x233
    addi.d $r8, $r0, 0x244
    addi.d $r9, $r0, 0x255
    addi.d $r10, $r0, 0x266
    addi.d $r11, $r0, 0x277
    addi.d $r23, $r0, 0x311

    break 0

    bne $r3, $r24, .Lloongarch_register_test_failed

    addi.d $r4, $r0, 0x101
    bne $r1, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x111
    bne $r12, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x122
    bne $r13, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x133
    bne $r14, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x144
    bne $r15, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x155
    bne $r16, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x166
    bne $r17, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x177
    bne $r18, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x188
    bne $r19, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x199
    bne $r20, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x211
    bne $r5, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x222
    bne $r6, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x233
    bne $r7, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x244
    bne $r8, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x255
    bne $r9, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x266
    bne $r10, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x277
    bne $r11, $r4, .Lloongarch_register_test_failed
    addi.d $r4, $r0, 0x311
    bne $r23, $r4, .Lloongarch_register_test_failed

    addi.d $r4, $r0, 1
    b .Lloongarch_register_test_done

.Lloongarch_register_test_failed:
    addi.d $r4, $r0, 0

.Lloongarch_register_test_done:
    ld.d $r1, $r3, 0
    ld.d $r23, $r3, 8
    ld.d $r24, $r3, 16
    addi.d $r3, $r3, 32
    jirl $r0, $r1, 0

.size __loongarch_trap_register_self_test, . - __loongarch_trap_register_self_test
