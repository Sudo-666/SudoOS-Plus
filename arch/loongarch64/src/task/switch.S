.section .text
.align 4

.globl __loongarch_switch_context
.type __loongarch_switch_context, @function

/*
 * r4 = previous Context*
 * r5 = next Context*
 *
 * Context layout:
 *   ra(r1), sp(r3), fp(r22), s0(r23)..s8(r31)
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
    addi.d $r3, $r3, -16
    st.d $r0, $r3, 0
    st.d $r0, $r3, 8
    jirl $r1, $r23, 0
    bl kernel_fresh_context_returned
    break 0
    .size __loongarch_fresh_context_entry, .-__loongarch_fresh_context_entry
