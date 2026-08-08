.section .text.entry
.align 4

.globl _start
.type _start, @function

/*
 * DMW 寄存器：
 *
 * CSR.DMW0 = 0x180
 * CSR.DMW1 = 0x181
 * CSR.DMW2 = 0x182
 * CSR.DMW3 = 0x183
 *
 * CRMD = 0x0
 */
.equ CSR_CRMD, 0x0

.equ CSR_DMW0, 0x180
.equ CSR_DMW1, 0x181
.equ CSR_DMW2, 0x182
.equ CSR_DMW3, 0x183

/*
 * DA=0, PG=1, PLV=0, IE=0,
 * DATF=coherent cached, DATM=coherent cached。
 *
 * 0x10: PG
 * 0x20: DATF = 1
 * 0x80: DATM = 1
 */
.equ CRMD_MAPPED_KERNEL, 0xb0

/*
 * 临时窗口：
 *
 * VSEG=0
 * MAT=coherent cached
 * PLV0=1
 *
 * 开启 PG 后，保证当前低地址指令仍能执行到跳转指令。
 */
.equ DMW_TEMP_LOW_CACHED, 0x0000000000000011

/*
 * 最终窗口。
 */
.equ DMW_UNCACHED_PLV0, 0x8000000000000001
.equ DMW_CACHED_PLV0,   0x9000000000000011

.equ CACHED_BASE, 0x9000000000000000

_start:
    /*
     * 保留固件参数：
     *
     * a0 = $r4
     * a1 = $r5
     * a2 = $r6
     *
     * 后续只使用临时寄存器 $r12-$r14。
     */

    /*
     * 建立一个临时低地址 cached 窗口。
     *
     * 一旦 CRMD.PG=1，当前低地址 PC 仍然能命中该窗口。
     */
    li.d $r12, DMW_TEMP_LOW_CACHED
    csrwr $r12, CSR_DMW0

    /*
     * 正式 cached DMW。
     */
    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1

    /*
     * 提前建立只用于数据的低地址兼容窗口。
     */
    csrwr $r0, CSR_DMW2

    csrwr $r0, CSR_DMW3

    /*
     * 计算当前 continuation 的 cached 高地址别名。
     *
     * 因为 .boot 按照高地址链接、低地址加载，
     * PC-relative offset 在两个地址空间中完全相同。
     */
    la.pcrel $r12, .Lhigh_address
    li.d $r13, CACHED_BASE
    or $r12, $r12, $r13

    /*
     * 进入 mapped address translation mode。
     *
     * 当前低 PC 暂时由 DMW0(VSEG=0) 保证可取指。
     */
    li.w $r14, CRMD_MAPPED_KERNEL
    csrwr $r14, CSR_CRMD

    dbar 0
    ibar 0

    /*
     * 跳到 cached DMW 高地址。
     */
    jirl $r0, $r12, 0

.Lhigh_address:
    /*
     * 当前 PC 已位于 0x9000... cached DMW。
     *
     * 现在可以把 DMW0 替换成最终的 uncached 窗口。
     */
    li.d $r12, DMW_UNCACHED_PLV0
    csrwr $r12, CSR_DMW0

    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1

    csrwr $r0, CSR_DMW2

    csrwr $r0, CSR_DMW3

    dbar 0
    ibar 0

    /*
     * 正式内核栈符号本身是高 VMA。
     */
    la.pcrel $r3, __boot_stack_top

    /*
     * 清理高地址 BSS。
     */
    la.pcrel $r12, __bss_start
    la.pcrel $r13, __bss_end

.Lclear_bss:
    bgeu $r12, $r13, .Lenter_rust

    st.d $r0, $r12, 0
    addi.d $r12, $r12, 8

    b .Lclear_bss

.Lenter_rust:
    /*
     * a0/a1/a2 未被修改。
     *
     * rust_entry 现在也是高地址符号。
     */
    bl rust_entry

.Lunexpected_return:
    idle 0
    b .Lunexpected_return

.size _start, . - _start