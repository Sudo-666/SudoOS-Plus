OUTPUT_ARCH(loongarch)

/*
 * ELF entry 使用低物理地址。
 *
 * .boot 本身按照高 DMW 地址链接，但通过 AT() 加载到
 * 0x0020_0000。这样启动汇编在物理地址执行时，
 * PC-relative relocation 依然正确。
 */
ENTRY(__kernel_entry_phys)

BOOT_PHYS_BASE   = 0x0000000000200000;
BOOT_VIRT_BASE   = 0x9000000000200000;

KERNEL_PHYS_BASE = 0x0000000000400000;
KERNEL_VIRT_BASE = 0x9000000000400000;

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
     * QEMU 应当跳到实际物理加载地址，而不是高 VMA。
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
     * - 低地址 .boot；
     * - 启动段和正式内核之间的保留间隔；
     * - 正式内核及启动栈。
     */
    __kernel_phys_start = BOOT_PHYS_BASE;
    __kernel_phys_end = __rw_phys_end;

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
    }
}