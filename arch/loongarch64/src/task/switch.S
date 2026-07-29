.section .text
.align 4

.globl __loongarch_switch_context
.type __loongarch_switch_context, @function
/*
 * r4 = previous Context*
 * r5 = next Context*
 *
 * Context layout:
 *   0..95    ra(r1), sp(r3), fp(r22), s0(r23)..s8(r31)
 *   96..607  32 x 128-bit FP/LSX register backing slots
 *   608      FCSR0
 *   616..672 FCC0..FCC7 (one machine word each)
 *
 * The final-2026 LoongArch userland executes LSX in ld-linux before its first
 * syscall.  Enable FPE+SXE on every CPU at the context-switch boundary and
 * always carry the full 128-bit state.  This also makes task migration safe:
 * an incoming task never depends on the target CPU having trapped on SXD.
 */
__loongarch_switch_context:
    st.d $r1,  $r4,  0*8
    st.d $r3,  $r4,  1*8
    st.d $r22, $r4,  2*8
    st.d $r23, $r4,  3*8
    st.d $r24, $r4,  4*8
    st.d $r25, $r4,  5*8
    st.d $r26, $r4,  6*8
    st.d $r27, $r4,  7*8
    st.d $r28, $r4,  8*8
    st.d $r29, $r4,  9*8
    st.d $r30, $r4, 10*8
    st.d $r31, $r4, 11*8

    /* CSR.EUEN: bit0=FPE, bit1=SXE. csrwr returns the old value in r12. */
    csrrd $r12, 0x2
    ori $r12, $r12, 0x3
    csrwr $r12, 0x2

    vst $vr0, $r4, 96
    vst $vr1, $r4, 112
    vst $vr2, $r4, 128
    vst $vr3, $r4, 144
    vst $vr4, $r4, 160
    vst $vr5, $r4, 176
    vst $vr6, $r4, 192
    vst $vr7, $r4, 208
    vst $vr8, $r4, 224
    vst $vr9, $r4, 240
    vst $vr10, $r4, 256
    vst $vr11, $r4, 272
    vst $vr12, $r4, 288
    vst $vr13, $r4, 304
    vst $vr14, $r4, 320
    vst $vr15, $r4, 336
    vst $vr16, $r4, 352
    vst $vr17, $r4, 368
    vst $vr18, $r4, 384
    vst $vr19, $r4, 400
    vst $vr20, $r4, 416
    vst $vr21, $r4, 432
    vst $vr22, $r4, 448
    vst $vr23, $r4, 464
    vst $vr24, $r4, 480
    vst $vr25, $r4, 496
    vst $vr26, $r4, 512
    vst $vr27, $r4, 528
    vst $vr28, $r4, 544
    vst $vr29, $r4, 560
    vst $vr30, $r4, 576
    vst $vr31, $r4, 592
    movfcsr2gr $r12, $fcsr0
    st.d $r12, $r4, 608
    movcf2gr $r12, $fcc0
    st.d $r12, $r4, 616
    movcf2gr $r12, $fcc1
    st.d $r12, $r4, 624
    movcf2gr $r12, $fcc2
    st.d $r12, $r4, 632
    movcf2gr $r12, $fcc3
    st.d $r12, $r4, 640
    movcf2gr $r12, $fcc4
    st.d $r12, $r4, 648
    movcf2gr $r12, $fcc5
    st.d $r12, $r4, 656
    movcf2gr $r12, $fcc6
    st.d $r12, $r4, 664
    movcf2gr $r12, $fcc7
    st.d $r12, $r4, 672

    vld $vr0, $r5, 96
    vld $vr1, $r5, 112
    vld $vr2, $r5, 128
    vld $vr3, $r5, 144
    vld $vr4, $r5, 160
    vld $vr5, $r5, 176
    vld $vr6, $r5, 192
    vld $vr7, $r5, 208
    vld $vr8, $r5, 224
    vld $vr9, $r5, 240
    vld $vr10, $r5, 256
    vld $vr11, $r5, 272
    vld $vr12, $r5, 288
    vld $vr13, $r5, 304
    vld $vr14, $r5, 320
    vld $vr15, $r5, 336
    vld $vr16, $r5, 352
    vld $vr17, $r5, 368
    vld $vr18, $r5, 384
    vld $vr19, $r5, 400
    vld $vr20, $r5, 416
    vld $vr21, $r5, 432
    vld $vr22, $r5, 448
    vld $vr23, $r5, 464
    vld $vr24, $r5, 480
    vld $vr25, $r5, 496
    vld $vr26, $r5, 512
    vld $vr27, $r5, 528
    vld $vr28, $r5, 544
    vld $vr29, $r5, 560
    vld $vr30, $r5, 576
    vld $vr31, $r5, 592
    ld.d $r12, $r5, 608
    movgr2fcsr $fcsr0, $r12
    ld.d $r12, $r5, 616
    movgr2cf $fcc0, $r12
    ld.d $r12, $r5, 624
    movgr2cf $fcc1, $r12
    ld.d $r12, $r5, 632
    movgr2cf $fcc2, $r12
    ld.d $r12, $r5, 640
    movgr2cf $fcc3, $r12
    ld.d $r12, $r5, 648
    movgr2cf $fcc4, $r12
    ld.d $r12, $r5, 656
    movgr2cf $fcc5, $r12
    ld.d $r12, $r5, 664
    movgr2cf $fcc6, $r12
    ld.d $r12, $r5, 672
    movgr2cf $fcc7, $r12

    ld.d $r1,  $r5,  0*8
    ld.d $r3,  $r5,  1*8
    ld.d $r22, $r5,  2*8
    ld.d $r23, $r5,  3*8
    ld.d $r24, $r5,  4*8
    ld.d $r25, $r5,  5*8
    ld.d $r26, $r5,  6*8
    ld.d $r27, $r5,  7*8
    ld.d $r28, $r5,  8*8
    ld.d $r29, $r5,  9*8
    ld.d $r30, $r5, 10*8
    ld.d $r31, $r5, 11*8

    jirl $r0, $r1, 0

.size __loongarch_switch_context, . - __loongarch_switch_context
    .section .text
    .align 4
    .globl __loongarch_fresh_context_entry
    .type __loongarch_fresh_context_entry, @function
__loongarch_fresh_context_entry:
    # s0 ($r23) carries the Rust bootstrap entry. Build a mapped ABI frame.
    # Fresh SP was validated before publication; touch no stack here.
    jirl $r1, $r23, 0
    bl kernel_fresh_context_returned
    break 0
    .size __loongarch_fresh_context_entry, .-__loongarch_fresh_context_entry
