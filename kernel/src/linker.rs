use myos_mm::{MappingOptions, PhysRange};

#[cfg(target_arch = "loongarch64")]
unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;

    static __rodata_start: u8;
    static __rodata_end: u8;

    static __data_start: u8;
    static __smp_boot_stack_top: u8;
}

#[cfg(target_arch = "riscv64")]
unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;

    static __rodata_start: u8;
    static __rodata_end: u8;

    static __data_start: u8;
    static __smp_boot_stack_top: u8;
}

#[derive(Clone, Copy, Debug)]
pub struct KernelSegment {
    name: &'static str,
    physical: PhysRange,
    options: MappingOptions,
}

impl KernelSegment {
    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn physical(self) -> PhysRange {
        self.physical
    }

    pub const fn options(self) -> MappingOptions {
        self.options
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KernelImageLayout {
    physical: PhysRange,
    segments: [KernelSegment; 3],
}

impl KernelImageLayout {
    pub const fn physical(self) -> PhysRange {
        self.physical
    }

    pub const fn segments(&self) -> &[KernelSegment; 3] {
        &self.segments
    }
}

#[cfg(target_arch = "riscv64")]
pub fn kernel_image_layout() -> KernelImageLayout {
    let text = riscv_kernel_symbol_range(
        core::ptr::addr_of!(__text_start),
        core::ptr::addr_of!(__text_end),
        "text",
    );

    let rodata = riscv_kernel_symbol_range(
        core::ptr::addr_of!(__rodata_start),
        core::ptr::addr_of!(__rodata_end),
        "rodata",
    );

    let read_write = riscv_kernel_symbol_range(
        core::ptr::addr_of!(__data_start),
        core::ptr::addr_of!(__smp_boot_stack_top),
        "data/bss/boot-stacks",
    );

    {
        const TEMPORARY_KERNEL_WINDOW_SIZE: usize = 64 * 1024 * 1024;

        let main_kernel_size = read_write.end().get() - text.start().get();

        assert!(
            main_kernel_size <= TEMPORARY_KERNEL_WINDOW_SIZE,
            "RISC-V high kernel exceeds the temporary \
             64 MiB startup mapping",
        );
    }

    assert_eq!(text.start(), crate::arch::memory::layout::KERNEL_PHYS_BASE,);

    let physical = PhysRange::new(
        crate::arch::memory::layout::BOOT_PHYS_BASE,
        read_write.end(),
    )
    .expect("invalid RISC-V kernel physical range");

    validate_layout(text, rodata, read_write, physical);

    KernelImageLayout {
        physical,

        segments: make_segments(text, rodata, read_write),
    }
}

#[cfg(target_arch = "riscv64")]
fn riscv_kernel_symbol_range(start: *const u8, end: *const u8, name: &'static str) -> PhysRange {
    let virtual_start = myos_mm::VirtAddr::new(start as usize);

    let virtual_end = myos_mm::VirtAddr::new(end as usize);

    let physical_start = crate::arch::memory::layout::kernel_image_physical_address(virtual_start)
        .unwrap_or_else(|| {
            panic!(
                "RISC-V {name} start is outside \
                     the high kernel image: {:#018x}",
                virtual_start.get(),
            );
        });

    let physical_end = crate::arch::memory::layout::kernel_image_physical_address(virtual_end)
        .unwrap_or_else(|| {
            panic!(
                "RISC-V {name} end is outside \
                     the high kernel image: {:#018x}",
                virtual_end.get(),
            );
        });

    PhysRange::new(physical_start, physical_end).unwrap_or_else(|| {
        panic!("invalid RISC-V physical range for {name}",);
    })
}

#[cfg(target_arch = "loongarch64")]
pub fn kernel_image_layout() -> KernelImageLayout {
    /*
     * 这些 linker symbols 都是正式内核的高 DMW VMA。
     * 不再把它们直接当物理地址使用。
     */
    let text = cached_symbol_range(
        core::ptr::addr_of!(__text_start),
        core::ptr::addr_of!(__text_end),
        "text",
    );

    let rodata = cached_symbol_range(
        core::ptr::addr_of!(__rodata_start),
        core::ptr::addr_of!(__rodata_end),
        "rodata",
    );

    /*
     * data、bss 和启动栈作为统一 RW 物理范围。
     */
    let read_write = cached_symbol_range(
        core::ptr::addr_of!(__data_start),
        core::ptr::addr_of!(__smp_boot_stack_top),
        "data/bss/boot-stacks",
    );

    assert_eq!(
        text.start(),
        crate::arch::memory::layout::KERNEL_PHYS_BASE,
        "LoongArch linker KERNEL_PHYS_BASE does not match layout",
    );

    /*
     * 整体物理保留区还要包含：
     *
     * .boot @ 0x0020_0000
     * 启动段到主内核间的保留间隙
     * 正式内核直到 SMP bootstrap stacks 末尾
     */
    let physical = PhysRange::new(
        crate::arch::memory::layout::BOOT_PHYS_BASE,
        read_write.end(),
    )
    .expect("invalid LoongArch kernel physical range");

    validate_layout(text, rodata, read_write, physical);

    KernelImageLayout {
        physical,
        segments: make_segments(text, rodata, read_write),
    }
}

pub fn kernel_image_range() -> PhysRange {
    kernel_image_layout().physical()
}

fn make_segments(text: PhysRange, rodata: PhysRange, read_write: PhysRange) -> [KernelSegment; 3] {
    [
        KernelSegment {
            name: "text",
            physical: text,
            options: MappingOptions::kernel_code(),
        },
        KernelSegment {
            name: "rodata",
            physical: rodata,
            options: MappingOptions::kernel_rodata(),
        },
        KernelSegment {
            name: "data/bss/boot-stacks",
            physical: read_write,
            options: MappingOptions::kernel_data(),
        },
    ]
}

fn validate_layout(text: PhysRange, rodata: PhysRange, read_write: PhysRange, physical: PhysRange) {
    assert!(text.is_page_aligned());
    assert!(rodata.is_page_aligned());
    assert!(
        read_write.start().is_aligned(myos_mm::PAGE_SIZE),
        "RW segment start is not page aligned: {:#018x}",
        read_write.start().get(),
    );

    assert!(
        read_write.end().is_aligned(myos_mm::PAGE_SIZE),
        "RW segment end is not page aligned: {:#018x}",
        read_write.end().get(),
    );
    assert!(physical.is_page_aligned());

    assert!(!text.is_empty());
    assert!(!rodata.is_empty());
    assert!(!read_write.is_empty());

    assert!(!text.overlaps(rodata));
    assert!(!text.overlaps(read_write));
    assert!(!rodata.overlaps(read_write));

    assert!(physical.contains_range(text));
    assert!(physical.contains_range(rodata));
    assert!(physical.contains_range(read_write));
}

#[cfg(target_arch = "loongarch64")]
fn cached_symbol_range(start: *const u8, end: *const u8, name: &'static str) -> PhysRange {
    let virtual_start = myos_mm::VirtAddr::new(start as usize);

    let virtual_end = myos_mm::VirtAddr::new(end as usize);

    let physical_start =
        crate::arch::memory::layout::cached_to_phys(virtual_start).unwrap_or_else(|| {
            panic!(
                "linker symbol for {name} start is not \
                 inside cached DMW: {:#018x}",
                virtual_start.get(),
            );
        });

    let physical_end =
        crate::arch::memory::layout::cached_to_phys(virtual_end).unwrap_or_else(|| {
            panic!(
                "linker symbol for {name} end is not \
                 inside cached DMW: {:#018x}",
                virtual_end.get(),
            );
        });

    PhysRange::new(physical_start, physical_end).unwrap_or_else(|| {
        panic!(
            "linker produced an invalid physical range \
             for {name}",
        );
    })
}
