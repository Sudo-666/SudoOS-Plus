/*
 * RISC-V 第一阶段启动代码。
 *
 * 执行环境：
 *
 * - OpenSBI 已进入 S-mode；
 * - satp.MODE = Bare；
 * - a0 = boot hart ID；
 * - a1 = FDT 物理地址；
 * - 当前 PC 位于 BOOT_PHYS_BASE。
 */

.section .boot.text.entry, "ax", @progbits
.align 2

.globl _start
.type _start, @function

.equ SATP_MODE_SV39, 0x8000000000000000

.equ PTE_V, 0x001
.equ PTE_R, 0x002
.equ PTE_W, 0x004
.equ PTE_X, 0x008
.equ PTE_A, 0x040
.equ PTE_D, 0x080

.equ PTE_TABLE, PTE_V

.equ PTE_RWAD, (PTE_V | PTE_R | PTE_W | PTE_A | PTE_D)

.equ PTE_RWXAD, (PTE_V | PTE_R | PTE_W | PTE_X | PTE_A | PTE_D)

.equ BOOT_PHYS_BASE,   0x80200000
.equ KERNEL_PHYS_BASE, 0x80400000

/*
 * 临时高半映射覆盖 64 MiB：
 *
 * 32 × 2 MiB = 64 MiB。
 */
.equ TEMP_KERNEL_LARGE_PAGES, 32

_start:
    /*
     * 在完整 trap 子系统建立之前禁止 supervisor interrupt。
     *
     * SSTATUS.SIE = bit 1。
     */
    csrci sstatus, 2

    /*
     * 所有以下 lla 都只引用低物理 boot 区域中的符号。
     *
     * 禁止 relaxation，避免链接器将地址计算改成依赖 gp
     * 的形式。此时 gp 尚未初始化。
     */
    .option push
    .option norelax

    lla t0, .Lboot_fault
    csrw stvec, t0

    lla s0, __boot_page_tables_start
    lla s1, __boot_page_tables_end

    .option pop

    /*
     * 清空所有临时页表。
     *
     * s0 在整个第一阶段中保存页表区域基址。
     */
    mv t0, s0

.Lclear_boot_tables:
    bgeu t0, s1, .Lbuild_identity_mapping

    sd zero, 0(t0)
    addi t0, t0, 8

    j .Lclear_boot_tables

.Lbuild_identity_mapping:
    /*
     * root[2] -> identity_l1
     *
     * 0x80200000 的 VPN[2] = 2。
     */
    .option push
    .option norelax

    lla t0, __boot_root_page_table
    lla t1, __boot_identity_l1

    .option pop

    srli t1, t1, 2
    ori t1, t1, PTE_TABLE

    sd t1, (2 * 8)(t0)

    /*
     * identity_l1[1]：
     *
     * VA 0x80200000..0x803fffff
     * PA 0x80200000..0x803fffff
     *
     * 这是一个 2 MiB 临时叶项。
     */
    .option push
    .option norelax

    lla t0, __boot_identity_l1

    .option pop

    li t1, BOOT_PHYS_BASE
    srli t1, t1, 2
    ori t1, t1, PTE_RWXAD

    sd t1, (1 * 8)(t0)

.Lbuild_high_kernel_mapping:
    /*
     * root[510] -> kernel_l1
     *
     * 高地址 0xffffffff80000000 的 VPN[2] = 510。
     */
    .option push
    .option norelax

    lla t0, __boot_root_page_table
    lla t1, __boot_kernel_l1

    .option pop

    srli t1, t1, 2
    ori t1, t1, PTE_TABLE

    li t2, 510*8
    add t0, t0, t2
    sd t1, 0(t0)

    /*
     * kernel_l1[0..31]：
     *
     * VA 0xffffffff80000000
     *      ..
     * VA 0xffffffff83ffffff
     *
     * 映射到：
     *
     * PA 0x80400000
     *      ..
     * PA 0x843fffff
     */
    .option push
    .option norelax

    lla t0, __boot_kernel_l1

    .option pop

    li t1, KERNEL_PHYS_BASE
    li t2, TEMP_KERNEL_LARGE_PAGES
    li t3, 0x200000

.Lmap_temporary_kernel:
    srli t4, t1, 2
    ori t4, t4, PTE_RWXAD

    sd t4, 0(t0)

    addi t0, t0, 8
    add t1, t1, t3
    addi t2, t2, -1

    bnez t2, .Lmap_temporary_kernel

.Lbuild_direct_map:
    /*
     * 临时 RAM direct map。
     *
     * PA 0x80000000..0xbfffffff
     *
     * 映射到根索引 346 对应的 1 GiB 高地址区域。
     *
     * 这是启动期粗粒度映射。正式页表会根据实际 FDT RAM
     * 范围重新建立映射。
     */
    .option push
    .option norelax

    lla t0, __boot_root_page_table

    .option pop

    li t1, 0x80000000
    srli t1, t1, 2
    ori t1, t1, PTE_RWAD

    li t2, 346*8
    add t0, t0, t2
    sd t1, 0(t0)

.Lbuild_uart_mapping:
    /*
     * UART：
     *
     * VA 0x10000000 -> PA 0x10000000
     *
     * root[0] -> uart_l1
     * uart_l1[128] -> uart_l0
     * uart_l0[0] -> UART 4 KiB 页面
     */

    .option push
    .option norelax

    lla t0, __boot_root_page_table
    lla t1, __boot_uart_l1

    .option pop

    srli t1, t1, 2
    ori t1, t1, PTE_TABLE

    sd t1, (0 * 8)(t0)

    .option push
    .option norelax

    lla t0, __boot_uart_l1
    lla t1, __boot_uart_l0

    .option pop

    srli t1, t1, 2
    ori t1, t1, PTE_TABLE

    sd t1, (128 * 8)(t0)

    .option push
    .option norelax

    lla t0, __boot_uart_l0

    .option pop

    li t1, 0x10000000
    srli t1, t1, 2
    ori t1, t1, PTE_RWAD

    sd t1, (0 * 8)(t0)

.Lenable_sv39:
    /*
     * 高地址跳转目标从低地址只读数据槽读取。
     *
     * 不再假设 __riscv_high_entry 精确等于某个硬编码常量。
     */
    .option push
    .option norelax

    lla t5, __boot_high_entry_address

    .option pop

    ld t5, 0(t5)

    /*
     * satp.PPN 必须是根页表物理地址除以 4096。
     */
    .option push
    .option norelax

    lla t0, __boot_root_page_table

    .option pop

    srli t0, t0, 12

    li t1, SATP_MODE_SV39
    or t0, t0, t1

    /*
     * 页表写入必须在硬件 walker 使用新根页表前有序可见。
     *
     * 切换后再次执行全局 SFENCE.VMA，清除旧地址空间留下的
     * 地址翻译缓存。
     */
    sfence.vma zero, zero
    csrw satp, t0
    sfence.vma zero, zero
    fence.i

    /*
     * t5 已保存精确的高地址入口。
     */
    jr t5

.Lboot_fault:
    /*
     * 此标签仍位于低地址 identity mapping 覆盖范围。
     */
    wfi
    j .Lboot_fault

.size _start, . - _start

/*
 * 链接器会在这里写入 __riscv_high_entry 的完整高虚拟地址。
 *
 * 第一阶段代码通过低地址 identity mapping 读取该指针，
 * 然后在开启 Sv39 后跳转。
 */
.section .boot.rodata.high_entry, "a", @progbits
.balign 8

.globl __boot_high_entry_address
__boot_high_entry_address:
    .dword __riscv_high_entry

/*
 * 临时页表必须是可写、不可执行的 NOBITS 数据。
 *
 * 它们不应位于 .boot.text 中。
 */
.section .boot.bss.page_tables, "aw", @nobits
.balign 4096

.globl __boot_page_tables_start
__boot_page_tables_start:

.globl __boot_root_page_table
__boot_root_page_table:
    .space 4096

.globl __boot_identity_l1
__boot_identity_l1:
    .space 4096

.globl __boot_kernel_l1
__boot_kernel_l1:
    .space 4096

.globl __boot_uart_l1
__boot_uart_l1:
    .space 4096

.globl __boot_uart_l0
__boot_uart_l0:
    .space 4096

.globl __boot_page_tables_end
__boot_page_tables_end:

/*
 * 第二阶段：高半内核入口。
 */
.section .text.high_entry, "ax", @progbits
.align 2

.globl __riscv_high_entry
.type __riscv_high_entry, @function

__riscv_high_entry:
    /*
     * 现在 PC 已经位于高半内核。
     */
    .option push
    .option norelax

    lla t0, .Lhigh_fault

    .option pop

    csrw stvec, t0

    /*
     * 重新加载高地址 global pointer。
     */
    .option push
    .option norelax

    lla gp, __global_pointer$

    .option pop

    /*
     * 切换到高地址启动栈。
     */
    .option push
    .option norelax

    lla sp, __boot_stack_top

    .option pop

    andi sp, sp, -16

    /*
     * 清空正式高半内核 BSS。
     */
    .option push
    .option norelax

    lla t0, __bss_start
    lla t1, __bss_end

    .option pop

.Lclear_bss:
    bgeu t0, t1, .Lenter_rust

    sd zero, 0(t0)
    addi t0, t0, 8

    j .Lclear_bss

.Lenter_rust:
    /*
     * OpenSBI 参数：
     *
     * a0 = hart ID
     * a1 = FDT 物理地址
     *
     * 当前接口仍保留第三个参数。
     */
    li a2, 0

    call rust_entry

.Lhigh_fault:
    wfi
    j .Lhigh_fault

.size __riscv_high_entry, . - __riscv_high_entry