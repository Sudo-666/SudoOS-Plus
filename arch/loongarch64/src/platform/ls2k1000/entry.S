/*
 * LS2K1000 主核入口 (final)。
 *
 * 兼容 bootm (PG=0, direct-address) 和 go (PG=1, DMW) 两种进入方式。
 * 策略：先无条件建立 DMW + 开启 PG，再用虚拟 UART 输出。
 * UART 物理基址: 0x1fe2_0000, 非缓存虚拟地址: 0x8000_0000_1fe2_0000
 */

.section .text.entry
.align 4
.globl _start
.type _start, @function

.equ CSR_CRMD,   0x0
.equ CSR_ECFG,   0x4
.equ CSR_DMW0,   0x180
.equ CSR_DMW1,   0x181
.equ CSR_DMW2,   0x182
.equ CSR_DMW3,   0x183
.equ CSR_CPUNUM, 0x20

.equ UART_BASE, 0x800000001fe20000
.equ UART_DAT,  0
.equ UART_LSR,  5
.equ LSR_TX_IDLE, 0x20

.macro PUTCHAR char
    li.w    $t4, \char
    li.d    $t5, UART_BASE
100:
    ld.bu   $t6, $t5, UART_LSR
    andi    $t6, $t6, LSR_TX_IDLE
    beqz    $t6, 100b
    st.b    $t4, $t5, UART_DAT
.endm

_start:
    /* ── 0. 诊断: 仅在 direct-address (PG=0) 模式下写物理 UART 'B' ──
     * go 路径进入时 U-Boot 保持 PG=1，此时写物理地址会触发 TLB 异常。
     * 先读 CRMD.PG，为 0 才直接写物理串口；PG=1 走后面 DMW 虚拟 UART。
     */
    csrrd   $t0, CSR_CRMD
    andi    $t0, $t0, 0x10          /* CRMD.PG */
    bnez    $t0, 1f
    li.w    $t0, 0x42               /* 'B' */
    li.d    $t1, 0x1fe20000
    st.b    $t0, $t1, 0
1:

    /* ── 1. 保存 U-Boot 参数到 callee-saved 寄存器 ── */
    or      $s0, $a0, $zero
    or      $s1, $a1, $zero
    or      $s2, $a2, $zero
    or      $s3, $a3, $zero

    /* ── 2. 关中断 ── */
    li.w    $t0, 0x4
    csrxchg $zero, $t0, CSR_CRMD
    csrwr   $zero, CSR_ECFG

    /* ── 2. CPUID 分流 ── */
    csrrd   $t0, CSR_CPUNUM
    andi    $t0, $t0, 0x3ff
    bnez    $t0, _start_secondary

    /* ── 3. 建立临时 DMW 窗口 ──
     * DMW0 = VSEG=0 临时 low cached 窗口
     *   PG=1 后当前低 PC 仍能通过此窗口取指
     * DMW1 = VSEG=0x9 正式 high cached 窗口
     */
    li.d    $r12, 0x0000000000000011   /* DMW0: temp low cached, VSEG=0 */
    csrwr   $r12, CSR_DMW0
    li.d    $r12, 0x9000000000000011   /* DMW1: high cached, VSEG=0x9 */
    csrwr   $r12, CSR_DMW1
    csrwr   $zero, CSR_DMW2
    csrwr   $zero, CSR_DMW3

    /* ── 4. 计算高地址别名并启用 PG ── */
    la.pcrel $r12, .Lhigh
    li.d    $r13, 0x9000000000000000
    or      $r12, $r12, $r13
    li.w    $r14, 0xb0
    csrwr   $r14, CSR_CRMD
    dbar 0
    ibar 0
    jirl    $zero, $r12, 0

.Lhigh:
    /* ── 5. 重配 DMW0 为最终 uncached 窗口 ── */
    li.d    $r12, 0x8000000000000001   /* DMW0: uncached, VSEG=0x8 */
    csrwr   $r12, CSR_DMW0
    li.d    $r12, 0x9000000000000011   /* DMW1: cached, VSEG=0x9 */
    csrwr   $r12, CSR_DMW1
    dbar 0
    ibar 0

    /* ── 6. 输出 'M' ── */
    PUTCHAR 0x4d

    /* ── 7. 保存参数到 BOOT_ARGS ── */
    la.global $t0, BOOT_ARGS
    st.d    $s0, $t0, 0
    st.d    $s1, $t0, 8
    st.d    $s2, $t0, 16
    st.d    $s3, $t0, 24

    /* ── 8. 设置栈指针 ── */
    la.global $sp, __boot_stack_top

    /* ── 9. 清空 BSS ── */
    la.global $t0, __bss_start
    la.global $t1, __bss_end
.Lclear_bss:
    bgeu    $t0, $t1, .Lenter_rust
    st.d    $zero, $t0, 0
    addi.d  $t0, $t0, 8
    b       .Lclear_bss

.Lenter_rust:
    PUTCHAR 0x79               /* 'y' */

    /* ── 10. 跳 rust_entry ── */
    or      $a0, $s0, $zero
    or      $a1, $s1, $zero
    or      $a2, $s2, $zero
    or      $a3, $s3, $zero
    bl      rust_entry

.Lunexpected_return:
    PUTCHAR 0x58               /* 'X' */
    idle 0
    b .Lunexpected_return

.size _start, . - _start