OUTPUT_ARCH(loongarch)

/*
 * LS2K1000 内核链接脚本 (DDR 基址 + AT() 加载)。
 *
 * 2K1000 的 1GB DDR 物理基址是 0x9000_0000（与 QEMU virt 约定一致）。
 * 内核以高 DMW 虚拟地址 (0x9000_0000_...) 链接，通过 AT() 加载到
 * DDR 物理地址。U-Boot 的 bootm/go 直接跳到低物理入口 __kernel_entry_phys
 * (0x9000_0000)，entry.S 再切换到高 DMW 地址执行。
 *
 * uImage load/entry 均为 0x9000_0000，32 位字段可容纳。
 */

ENTRY(__kernel_entry_phys)

BOOT_PHYS_BASE   = 0x0000000090000000;
BOOT_VIRT_BASE   = 0x9000000090000000;

KERNEL_PHYS_BASE = 0x0000000090200000;
KERNEL_VIRT_BASE = 0x9000000090200000;

BOOT_STACK_SIZE = 64K;
SMP_BOOT_STACKS_SIZE = 512K;

SECTIONS
{
    /*
     * 低物理启动段，高 DMW 链接地址。
     */
    . = BOOT_VIRT_BASE;

    .boot : AT(BOOT_PHYS_BASE)
    {
        __boot_virt_start = .;

        KEEP(*(.text.entry))
        KEEP(*(.text.secondary_entry))

        . = ALIGN(4K);
        __boot_virt_end = .;
    }

    __boot_phys_start = LOADADDR(.boot);
    __boot_phys_end =
        LOADADDR(.boot) + SIZEOF(.boot);

    /*
     * U-Boot 应当跳到实际物理加载地址，而不是高 VMA。
     */
    __kernel_entry_phys =
        LOADADDR(.boot) + (_start - ADDR(.boot));

    . = KERNEL_VIRT_BASE;

    __kernel_start = .;

    /*
     * ============================================================
     * Text: RX
     * ============================================================
     */

    . = ALIGN(4K);

    .text : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.text) - KERNEL_VIRT_BASE)
    )
    {
        __text_start = .;

        /*
         * TLB refill executes in direct-address mode.  Keep it in a dedicated
         * physical 4 KiB page and never let ordinary text share that page.
         */
        KEEP(*(.text.tlb_refill))
        ASSERT(. - __text_start <= 4K, "LoongArch TLB refill exceeds one page")
        . = __text_start + 4K;

        /*
         * 板级异常向量 (EENTRY) 独占一个 4 KiB 页。LoongArch 硬件把
         * EENTRY[11:0] 清零，异常入口必须页对齐；否则第一个 timer IRQ 会
         * 跳进本页页首 —— 此前恰好是 allocator shim，打印出伪 OOM。
         * trampoline 定义于 platform/ls2k1000/entry.S 的 .text.trap_entry。
         */
        . = ALIGN(4K);
        __trap_vector_start = .;
        KEEP(*(.text.trap_entry))
        __trap_vector_end = .;
        ASSERT(
            __loongarch_trap_entry_ls2k == __trap_vector_start,
            "board trap entry is not page-aligned"
        )
        ASSERT(
            __trap_vector_end - __trap_vector_start <= 4K,
            "board trap entry exceeds one page"
        )

        . = ALIGN(4K);
        *(.text .text.*)

        . = ALIGN(4K);
        __text_end = .;
    }

    /*
     * ============================================================
     * Read-only data: R
     * ============================================================
     */

    . = ALIGN(4K);

    .rodata : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.rodata) - KERNEL_VIRT_BASE)
    )
    {
        __rodata_start = .;

        *(.rodata .rodata.*)
        *(.srodata .srodata.*)

        *(.rodata.cst4)
        *(.rodata.cst8)
        *(.rodata.cst16)
        *(.rodata.cst32)

        . = ALIGN(4K);
        __rodata_end = .;
    }

    /*
     * ============================================================
     * Writable initialized data: RW
     * ============================================================
     */

    . = ALIGN(4K);

    .data : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.data) - KERNEL_VIRT_BASE)
    )
    {
        __data_start = .;

        *(.data .data.*)

        *(.data.rel.ro.local .data.rel.ro.local.*)
        *(.data.rel.ro .data.rel.ro.*)

        *(.sdata .sdata.*)

        *(.got .got.*)
        *(.got.plt)

        *(.init_array .init_array.*)
        *(.fini_array .fini_array.*)

        . = ALIGN(4K);
        __data_end = .;
    }

    /*
     * ============================================================
     * Zero-initialized data: RW, NOBITS
     * ============================================================
     */

    . = ALIGN(4K);

    .bss (NOLOAD) : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.bss) - KERNEL_VIRT_BASE)
    )
    {
        __bss_start = .;

        *(.sbss .sbss.*)
        *(.bss .bss.*)
        *(COMMON)

        . = ALIGN(4K);
        __bss_end = .;
    }

    /*
     * ============================================================
     * Boot stack: RW, NOBITS
     * ============================================================
     */

    . = ALIGN(4K);

    .boot_stack (NOLOAD) : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.boot_stack) - KERNEL_VIRT_BASE)
    )
    {
        __boot_stack_bottom = .;

        . += BOOT_STACK_SIZE;

        . = ALIGN(4K);
        __boot_stack_top = .;
    }

    .smp_boot_stacks (NOLOAD) : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.smp_boot_stacks) - KERNEL_VIRT_BASE)
    )
    {
        __smp_boot_stack_bottom = .;

        . += SMP_BOOT_STACKS_SIZE;

        . = ALIGN(4K);
        __smp_boot_stack_top = .;
    }

    . = ALIGN(4K);
    __kernel_end = .;

    /*
     * ============================================================
     * USB EHCI uncached DMA 池 (.nocache_ram)
     * ============================================================
     *
     * EHCI 的 QH/qTD 描述符、async/periodic 队列头和 frame list 必须落在
     * 控制器可见且无缓存陈旧问题的物理内存。LS2K1000 上把它们链接到强序
     * 非缓存 DMW 窗口 (0x8000_0000_0000_0000 | phys) 的 VMA：所有 CPU
     * 访问经该窗口直达物理内存，控制器读到最新值，无需 `cache` 指令维护
     * （binutils 无法汇编该指令，见 docs/decisions/ADR-001）。
     *
     * 物理紧随内核镜像（__kernel_end）之后，由 memory.rs 用
     * __nocache_ram_start/__nocache_ram_end 保留，防止页分配器复用。
     * NOLOAD：全零初始化，USB 胶水在 usb_hc_init 前 memset。
     */
    __nocache_phys_base = ALIGN(__kernel_end & 0x0FFFFFFFFFFFFFFF, 4K);

    . = 0x8000000000000000 + __nocache_phys_base;

    .nocache_ram (NOLOAD) :
    {
        __nocache_ram_start = .;
        *(.nocache_ram .nocache_ram.*)
        . = ALIGN(4K);

        /*
         * 动态 DMA 缓冲 bump 池：给枚举/传输的临时缓冲（ep0 setup、
         * 描述符数据、MSC 块读）预留。Rust 侧 free-list 分配器从
         * __nocache_ram_start..__nocache_ram_end 切块。
         */
        __nocache_dyn_start = .;
        . += 48K;
        __nocache_dyn_end = .;

        . = ALIGN(4K);
        __nocache_ram_end = .;
    }

    /*
     * 权限分段对应的物理范围。
     */
    __text_phys_start = LOADADDR(.text);
    __text_phys_end =
        LOADADDR(.text) + SIZEOF(.text);

    __rodata_phys_start = LOADADDR(.rodata);
    __rodata_phys_end =
        LOADADDR(.rodata) + SIZEOF(.rodata);

    __rw_phys_start = LOADADDR(.data);
    __rw_phys_end =
        LOADADDR(.smp_boot_stacks)
        + SIZEOF(.smp_boot_stacks);

    /*
     * 整个内核物理占用范围包含：
     *
     * - DDR 基址处的 .boot；
     * - 启动段和正式内核之间的保留间隔；
     * - 正式内核及启动栈。
     */
    __kernel_phys_start = BOOT_PHYS_BASE;
    __kernel_phys_end = __rw_phys_end;

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
        *(.debug_info)
    }
}
