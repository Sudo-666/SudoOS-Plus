OUTPUT_ARCH(riscv)
ENTRY(_start)

BOOT_PHYS_BASE   = 0x80200000;
KERNEL_PHYS_BASE = 0x80400000;
KERNEL_VIRT_BASE = 0xffffffff80000000;

BOOT_STACK_SIZE = 64K;

SECTIONS
{
    /*
     * 第一阶段启动代码。
     *
     * VMA 和 LMA 都是低物理地址，因此 OpenSBI 可以直接跳入。
     */
    . = BOOT_PHYS_BASE;

    __boot_phys_start = .;

    .boot.text : ALIGN(4K)
{
    KEEP(*(.boot.text.entry))
    *(.boot.text .boot.text.*)
}

.boot.rodata : ALIGN(4K)
{
    KEEP(*(.boot.rodata .boot.rodata.*))
}

.boot.data : ALIGN(4K)
{
    *(.boot.data .boot.data.*)
}

.boot.bss (NOLOAD) : ALIGN(4K)
{
    __boot_bss_start = .;

    *(.boot.bss .boot.bss.*)

    . = ALIGN(4K);
    __boot_bss_end = .;
}

    __boot_phys_end = .;

    /*
     * 正式高半内核。
     *
     * VMA 从 0xffffffff80000000 开始，
     * LMA 从物理地址 0x80400000 开始。
     */
    . = KERNEL_VIRT_BASE;

    __kernel_start = .;

    .text : AT(
    KERNEL_PHYS_BASE
    + (ADDR(.text) - KERNEL_VIRT_BASE)
)
{
    __text_start = .;

    KEEP(*(.text.high_entry))
    *(.text .text.*)

    . = ALIGN(4K);
    __text_end = .;
}

    .rodata : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.rodata) - KERNEL_VIRT_BASE)
    )
    {
        __rodata_start = .;

        *(.rodata .rodata.*)

        . = ALIGN(4K);
        __rodata_end = .;
    }

    .data : AT(
        KERNEL_PHYS_BASE
        + (ADDR(.data) - KERNEL_VIRT_BASE)
    )
    {
        __data_start = .;

        *(.data .data.*)

        . = ALIGN(16);

        /*
         * RISC-V small-data global pointer。
         */
        PROVIDE(__global_pointer$ = . + 0x800);

        *(.sdata .sdata.*)
        *(.got .got.*)

        . = ALIGN(4K);
        __data_end = .;
    }

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

    . = ALIGN(4K);

    __kernel_end = .;

    /*
     * 正式高半段对应的低物理范围。
     */
    __text_phys_start = LOADADDR(.text);
    __text_phys_end =
        LOADADDR(.text) + SIZEOF(.text);

    __rodata_phys_start = LOADADDR(.rodata);
    __rodata_phys_end =
        LOADADDR(.rodata) + SIZEOF(.rodata);

    __rw_phys_start = LOADADDR(.data);
    __rw_phys_end =
        LOADADDR(.boot_stack)
        + SIZEOF(.boot_stack);

    /*
     * 内存分配器必须保留完整范围：
     *
     * .boot
     * 0x802... 到 0x804... 的间隙
     * 正式高半内核的所有 LMA
     */
    __kernel_phys_start = BOOT_PHYS_BASE;
    __kernel_phys_end = __rw_phys_end;

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
    }
}