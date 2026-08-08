/*
 * LS2K1000 副核入口。
 *
 * 有两个入口点：
 * 1. _start_secondary —— U-Boot 启动时由 entry.S 的 CPUID 分流到达
 * 2. __loongarch_secondary_entry —— 内核通过 BootSlot + IOCSR IPI 唤醒到达
 *
 * 入口点 2 使用与 QEMU virt 相同的 BootSlot 机制：
 * 内核填写 __loongarch_secondary_boot_data[hardware_id] 后发送 IPI，
 * 副核在此读取启动数据并跳转到内核指定的入口。
 */

.section .text.secondary_entry, "ax", @progbits
.align 4

.globl _start_secondary
.type _start_secondary, @function

.globl __loongarch_secondary_entry
.type __loongarch_secondary_entry, @function

.equ CSR_CRMD, 0x0
.equ CSR_ECFG, 0x4
.equ CSR_DMW0, 0x180
.equ CSR_DMW1, 0x181
.equ CSR_DMW2, 0x182
.equ CSR_DMW3, 0x183
.equ CSR_CPUNUM, 0x20

.equ CRMD_MAPPED_KERNEL, 0xb0
.equ DMW_TEMP_LOW_CACHED, 0x0000000000000011
.equ DMW_UNCACHED_PLV0, 0x8000000000000001
.equ DMW_CACHED_PLV0,   0x9000000000000011
.equ CACHED_BASE, 0x9000000000000000

_start_secondary:
__loongarch_secondary_entry:
    # 禁用本地中断和 ROM 阶段遗留的异常源
    li.w $r12, 0x4
    csrxchg $r0, $r12, CSR_CRMD
    csrwr $r0, CSR_ECFG

    # 读取硬件 CPU ID
    csrrd $r22, CSR_CPUNUM
    andi $r22, $r22, 0x3ff

    # 配置 DMW 窗口以建立 XKPRANGE 地址映射
    li.d $r12, DMW_TEMP_LOW_CACHED
    csrwr $r12, CSR_DMW0
    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1
    csrwr $r0, CSR_DMW2
    csrwr $r0, CSR_DMW3

    # 计算并跳转到高地址 cached 别名
    la.pcrel $r12, .Lsecondary_high
    li.d $r13, CACHED_BASE
    or $r12, $r12, $r13

    li.w $r14, CRMD_MAPPED_KERNEL
    csrwr $r14, CSR_CRMD
    dbar 0
    ibar 0
    jirl $r0, $r12, 0

.Lsecondary_high:
    # 重写 DMW 窗口为最终配置
    li.d $r12, DMW_UNCACHED_PLV0
    csrwr $r12, CSR_DMW0
    li.d $r12, DMW_CACHED_PLV0
    csrwr $r12, CSR_DMW1
    csrwr $r0, CSR_DMW2
    csrwr $r0, CSR_DMW3
    dbar 0
    ibar 0

    # 读取 BootSlot（每个槽位 64 字节，repr(align(64))）
    la.pcrel $r12, __loongarch_secondary_boot_data
    slli.d $r13, $r22, 6
    add.d $r12, $r12, $r13

    ld.d $r3,  $r12, 0   # 高地址 bootstrap stack top
    ld.d $r21, $r12, 8   # 逻辑 CPU ID
    ld.d $r13, $r12, 16  # 高地址 Rust secondary entry
    ld.d $r14, $r12, 24  # publication marker

    # 如果 BootSlot 尚未发布（marker == 0），说明是通过
    # U-Boot 冷启动到达（_start_secondary 路径），
    # 退化为简单的 per-core 栈并调用存根。
    beqz $r14, .Lcold_boot_fallback

    # 内核已发布 BootSlot：通过 kernel-provided entry 启动
    or $r4, $r21, $r0  # 逻辑 CPU ID
    or $r5, $r22, $r0  # 硬件 CPU ID
    jirl $r0, $r13, 0

.Lcold_boot_fallback:
    # U-Boot 冷启动路径：为每个核心分配 16KB (0x4000) 栈
    la.global $sp, __boot_stack_top
    li.d $t1, 0x4000
    mul.d $t1, $r22, $t1
    sub.d $sp, $sp, $t1

    # 调用 Rust 副核存根
    bl rust_main_secondary

.Lsecondary_fault:
    idle 0
    b .Lsecondary_fault

.size _start_secondary, . - _start_secondary
.size __loongarch_secondary_entry, . - __loongarch_secondary_entry