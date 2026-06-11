use myos_fdt::{DeviceTree, FdtBlob, FdtError, MemoryRegion};

use myos_mm::{MemoryMap, MemoryMapError, PhysAddr, PhysRange};

use myos_mm::EarlyFrameAllocator;

const MEMORY_MAP_CAPACITY: usize = 64;

pub type BootMemoryMap = MemoryMap<MEMORY_MAP_CAPACITY>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootMemoryError {
    InvalidPhysicalRange,
    NoUsableMemory,

    Fdt(FdtError),
    MemoryMap(MemoryMapError),
}

impl From<FdtError> for BootMemoryError {
    fn from(error: FdtError) -> Self {
        Self::Fdt(error)
    }
}

impl From<MemoryMapError> for BootMemoryError {
    fn from(error: MemoryMapError) -> Self {
        Self::MemoryMap(error)
    }
}

pub fn build_boot_memory_map(
    fdt_address: usize,
    blob: &FdtBlob<'_>,
    tree: &DeviceTree<'_>,
) -> Result<BootMemoryMap, BootMemoryError> {
    let mut map = BootMemoryMap::new();

    /*
     * 1. 加入固件声明的普通 RAM。
     */
    for region in tree.memory_regions() {
        map.add_usable(to_phys_range(region)?)?;
    }

    /*
     * 2. 排除 FDT memory reservation block。
     */
    for reservation in blob.memory_reservations() {
        map.reserve(to_phys_range(reservation?)?)?;
    }

    /*
     * 3. 排除 /reserved-memory 静态区域。
     */
    let mut reserve_error = None;

    tree.for_each_reserved_memory_region(|_name, region| {
        if reserve_error.is_some() {
            return;
        }

        reserve_error = map
            .reserve(match to_phys_range(region) {
                Ok(range) => range,

                Err(error) => {
                    reserve_error = Some(error);
                    return;
                }
            })
            .err()
            .map(BootMemoryError::from);
    })?;

    if let Some(error) = reserve_error {
        return Err(error);
    }

    /*
     * 4. 排除架构/平台启动协议占用的内存。
     *
     * 例如：
     *
     * - LoongArch QEMU 的低端 1 MiB 启动表；
     * - 某些开发板的固件共享区；
     * - 未通过 FDT 描述的启动器保留区。
     */
    crate::arch::memory::reserve_early_platform_memory(&mut map)?;

    /*
     * 5. 排除内核镜像自身。
     */
    map.reserve(crate::linker::kernel_image_range())?;

    /*
     * 6. 排除 FDT blob 自身。
     */
    let fdt_range = PhysRange::from_start_size(PhysAddr::new(fdt_address), blob.total_size())
        .ok_or(BootMemoryError::InvalidPhysicalRange)?;

    map.reserve(fdt_range)?;

    if map.is_empty() {
        return Err(BootMemoryError::NoUsableMemory);
    }

    Ok(map)
}

pub fn print_boot_memory_map(map: &BootMemoryMap) {
    crate::println!("physical memory:");

    for range in map.iter() {
        crate::println!(
            "  free [{:#018x}, {:#018x})  {} KiB",
            range.start().get(),
            range.end().get(),
            range.size() / 1024,
        );
    }

    match map.total_bytes() {
        Some(bytes) => {
            crate::println!("  total usable: {} MiB", bytes / 1024 / 1024,);
        }

        None => {
            crate::println!("  total usable: overflow",);
        }
    }
}

fn to_phys_range(region: MemoryRegion) -> Result<PhysRange, BootMemoryError> {
    PhysRange::from_start_size(PhysAddr::new(region.start()), region.size())
        .ok_or(BootMemoryError::InvalidPhysicalRange)
}

pub fn print_virtual_layout() {
    crate::arch::memory::layout::validate().unwrap_or_else(|error| {
        panic!("invalid virtual memory layout: {error:?}",);
    });

    let user = crate::arch::memory::layout::USER_RANGE;

    crate::println!("virtual memory policy:");
    crate::println!(
        "  user [{:#018x}, {:#018x})",
        user.start().get(),
        user.end().get(),
    );

    for region in crate::arch::memory::layout::KERNEL_REGIONS {
        let range = region.range();

        crate::println!(
            "  {:<20} [{:#018x}, {:#018x})",
            region.name(),
            range.start().get(),
            range.end().get(),
        );
    }

    crate::println!(
        "  kernel link address: {:#018x}",
        crate::arch::memory::layout::KERNEL_LINK_BASE.get(),
    );
}

pub fn verify_early_frame_allocator(map: &BootMemoryMap) {
    let allocator = EarlyFrameAllocator::from_memory_map(map);

    let before = allocator
        .remaining_bytes()
        .expect("physical memory size overflow");

    /*
     * 使用副本测试，不消费真正准备交给页表代码的分配器。
     */
    let mut probe = allocator;

    let frame = probe
        .allocate_frame()
        .expect("unable to allocate an early test frame");

    let block = probe
        .allocate_contiguous(4)
        .expect("unable to allocate four contiguous early frames");

    let after = probe
        .remaining_bytes()
        .expect("physical memory size overflow");

    assert_eq!(before - after, 5 * myos_mm::PAGE_SIZE,);

    crate::println!("early frame allocator:");
    crate::println!("  single frame : {:#018x}", frame.start_address().get(),);
    crate::println!(
        "  four frames  : [{:#018x}, {:#018x})",
        block.range().start().get(),
        block.range().end().get(),
    );
    crate::println!("  total frames : {}", before / myos_mm::PAGE_SIZE,);
}

pub fn validate_paging_policy() {
    crate::arch::memory::paging::validate();

    myos_mm::MappingOptions::kernel_code()
        .validate()
        .expect("invalid kernel code mapping policy");

    myos_mm::MappingOptions::kernel_rodata()
        .validate()
        .expect("invalid kernel rodata mapping policy");

    myos_mm::MappingOptions::kernel_data()
        .validate()
        .expect("invalid kernel data mapping policy");

    crate::println!("paging policy:");
    crate::println!(
        "  levels            : {}",
        crate::arch::memory::paging::LEVELS,
    );
    crate::println!(
        "  entries per table : {}",
        crate::arch::memory::paging::ENTRIES_PER_TABLE,
    );
    crate::println!(
        "  virtual bits      : {}",
        crate::arch::memory::paging::VIRTUAL_ADDRESS_BITS,
    );
    crate::println!("  write xor execute : enforced",);
}

pub struct EarlyMemoryState {
    frame_allocator: EarlyFrameAllocator<MEMORY_MAP_CAPACITY>,

    boot_page_table: crate::arch::memory::paging::BootPageTable,
}

impl EarlyMemoryState {
    pub const fn frame_allocator(&self) -> &EarlyFrameAllocator<MEMORY_MAP_CAPACITY> {
        &self.frame_allocator
    }

    pub const fn boot_page_table(&self) -> &crate::arch::memory::paging::BootPageTable {
        &self.boot_page_table
    }

    pub fn parts_mut(
        &mut self,
    ) -> (
        &mut EarlyFrameAllocator<MEMORY_MAP_CAPACITY>,
        &mut crate::arch::memory::paging::BootPageTable,
    ) {
        (&mut self.frame_allocator, &mut self.boot_page_table)
    }
}

pub fn initialize_early_memory(map: &BootMemoryMap) -> EarlyMemoryState {
    let mut frame_allocator = EarlyFrameAllocator::from_memory_map(map);

    let before = frame_allocator
        .remaining_frames()
        .expect("physical frame count overflow");

    let boot_page_table = crate::arch::memory::paging::BootPageTable::new(&mut frame_allocator)
        .unwrap_or_else(|error| {
            panic!(
                "unable to allocate boot page tables: \
                 {error:?}",
            );
        });

    let after = frame_allocator
        .remaining_frames()
        .expect("physical frame count overflow");

    assert_eq!(before - after, boot_page_table.allocated_table_pages(),);

    crate::println!("boot page tables:");
    crate::println!(
        "  root frame      : {:#018x}",
        boot_page_table.root_frame().start_address().get(),
    );
    crate::println!(
        "  allocated pages : {}",
        boot_page_table.allocated_table_pages(),
    );
    crate::println!("  remaining frames: {}", after,);

    EarlyMemoryState {
        frame_allocator,
        boot_page_table,
    }
}

pub fn map_boot_fdt_page(state: &mut EarlyMemoryState, fdt_address: usize) {
    let virtual_page =
        myos_mm::VirtPage::from_start_address(crate::arch::memory::layout::FIXMAP.start())
            .expect("FIXMAP start is not page aligned");

    let physical_frame =
        myos_mm::PhysFrame::containing_address(myos_mm::PhysAddr::new(fdt_address));

    let offset = fdt_address & (myos_mm::PAGE_SIZE - 1);

    let (allocator, page_table) = state.parts_mut();

    page_table
        .map_page(
            allocator,
            virtual_page,
            physical_frame,
            myos_mm::MappingOptions::kernel_rodata(),
        )
        .unwrap_or_else(|error| {
            panic!("unable to map FDT fixmap page: {error:?}",);
        });

    let test_virtual = virtual_page
        .start_address()
        .checked_add(offset)
        .expect("FDT fixmap address overflow");

    let translated = page_table
        .translate(test_virtual)
        .unwrap_or_else(|error| {
            panic!("unable to translate FDT fixmap: {error:?}",);
        })
        .expect("FDT fixmap translation is missing");

    assert_eq!(translated.get(), fdt_address,);

    crate::println!("software page-table test:");
    crate::println!("  virtual : {:#018x}", test_virtual.get(),);
    crate::println!("  physical: {:#018x}", translated.get(),);
    crate::println!("  tables  : {} pages", page_table.allocated_table_pages(),);
}

#[cfg(target_arch = "riscv64")]
pub fn prepare_kernel_image(
    state: &mut EarlyMemoryState,
) {
    let image =
        crate::linker::kernel_image_layout();

    crate::println!("kernel image mapping:");

    for segment in image.segments() {
        let physical = segment.physical();

        let virtual_start =
            crate::arch::memory::layout::
                kernel_image_virtual_address(
                    physical.start(),
                )
                .unwrap_or_else(|| {
                    panic!(
                        "unable to calculate virtual address \
                         for kernel segment {}",
                        segment.name(),
                    );
                });

        crate::println!(
            "  {:<16} phys [{:#018x}, {:#018x}) \
             -> virt {:#018x}",
            segment.name(),
            physical.start().get(),
            physical.end().get(),
            virtual_start.get(),
        );

        map_riscv_segment(
            state,
            *segment,
            virtual_start,
        );
    }
}

#[cfg(target_arch = "loongarch64")]
pub fn prepare_kernel_image(
    _state: &mut EarlyMemoryState,
) {
    let image =
        crate::linker::kernel_image_layout();

    crate::println!("kernel image mapping:");

    for segment in image.segments() {
        let physical = segment.physical();

        let virtual_start =
            crate::arch::memory::layout::phys_to_cached(
                physical.start(),
            )
            .unwrap_or_else(|| {
                panic!(
                    "kernel segment {} is outside \
                     the cached DMW",
                    segment.name(),
                );
            });

        let physical_last = physical
            .end()
            .checked_sub(1)
            .expect("kernel segment is empty");

        let virtual_last =
            crate::arch::memory::layout::phys_to_cached(
                physical_last,
            )
            .expect(
                "kernel segment end is outside cached DMW",
            );

        assert_eq!(
            crate::arch::memory::layout::cached_to_phys(
                virtual_start,
            ),
            Some(physical.start()),
        );

        assert_eq!(
            crate::arch::memory::layout::cached_to_phys(
                virtual_last,
            ),
            Some(physical_last),
        );

        crate::println!(
            "  {:<16} phys [{:#018x}, {:#018x}) \
             -> virt {:#018x}",
            segment.name(),
            physical.start().get(),
            physical.end().get(),
            virtual_start.get(),
        );
    }
}

#[cfg(target_arch = "riscv64")]
fn map_riscv_segment(
    state: &mut EarlyMemoryState,
    segment: crate::linker::KernelSegment,
    virtual_start: myos_mm::VirtAddr,
) {
    use myos_mm::{PAGE_SIZE, PhysFrame, VirtPage};

    let physical = segment.physical();

    let (allocator, page_table) = state.parts_mut();

    let mut offset = 0;

    while offset < physical.size() {
        let physical_address = physical
            .start()
            .checked_add(offset)
            .expect("kernel physical address overflow");

        let virtual_address = virtual_start
            .checked_add(offset)
            .expect("kernel virtual address overflow");

        let frame = PhysFrame::from_start_address(physical_address)
            .expect("kernel segment is not page aligned");

        let page = VirtPage::from_start_address(virtual_address)
            .expect("kernel virtual segment is not page aligned");

        page_table
            .map_page(allocator, page, frame, segment.options())
            .unwrap_or_else(|error| {
                panic!(
                    "unable to map kernel segment {} \
                     at offset {offset:#x}: {error:?}",
                    segment.name(),
                );
            });

        offset += PAGE_SIZE;
    }
}

#[cfg(target_arch = "loongarch64")]
fn prepare_kernel_segment(
    _state: &mut EarlyMemoryState,
    segment: crate::linker::KernelSegment,
    virtual_start: myos_mm::VirtAddr,
) {
    let physical = segment.physical();

    let expected_start = crate::arch::memory::layout::phys_to_cached(physical.start())
        .expect("kernel segment is outside cached DMW");

    assert_eq!(virtual_start, expected_start,);

    let physical_last = physical.end().checked_sub(1).expect("empty kernel segment");

    let virtual_last = crate::arch::memory::layout::phys_to_cached(physical_last)
        .expect("kernel segment end is outside cached DMW");

    let translated_last = crate::arch::memory::layout::cached_to_phys(virtual_last)
        .expect("cached DMW reverse translation failed");

    assert_eq!(translated_last, physical_last,);

    /*
     * LoongArch 内核镜像通过 cached DMW 访问，
     * 不消耗页表页面。
     */
}

#[cfg(target_arch = "riscv64")]
pub fn prepare_riscv_early_uart_mapping(
    state: &mut EarlyMemoryState,
) {
    use myos_mm::{
        MappingOptions,
        PhysAddr,
        PhysFrame,
        VirtAddr,
        VirtPage,
    };

    let physical =
        PhysAddr::new(
            crate::arch::early_console::MMIO_BASE,
        );

    let frame =
        PhysFrame::from_start_address(physical)
            .expect("RISC-V UART base is unaligned");

    let page =
        VirtPage::from_start_address(
            VirtAddr::new(physical.get()),
        )
        .expect("RISC-V UART VA is unaligned");

    let (allocator, page_table) =
        state.parts_mut();

    page_table
        .map_page(
            allocator,
            page,
            frame,
            MappingOptions::kernel_device(),
        )
        .unwrap_or_else(|error| {
            panic!(
                "unable to map early RISC-V UART: \
                 {error:?}",
            );
        });

    crate::println!("RISC-V final early MMIO:");
    crate::println!(
        "  uart identity: [{:#018x}, {:#018x})",
        physical.get(),
        physical.get()
            + crate::arch::early_console::MMIO_SIZE,
    );
}

#[cfg(target_arch = "riscv64")]
pub fn install_riscv_final_page_table(
    state: &EarlyMemoryState,
) {
    use core::{
        arch::asm,
        ptr::read_volatile,
    };

    let root =
        state
            .boot_page_table()
            .root_frame();

    /*
     * 进入 Rust 前静态 Sv39 已经开启。
     */
    assert!(
        crate::arch::memory::paging::
            translation_is_enabled(),
    );

    unsafe {
        crate::arch::memory::paging::
            switch_sv39_root(root)
    }
    .unwrap_or_else(|error| {
        panic!(
            "failed to install final RISC-V page table: \
             {error:?}",
        );
    });

    let current_pc: usize;

    unsafe {
        asm!(
            "auipc {pc}, 0",
            pc = out(reg) current_pc,
            options(nomem, nostack),
        );
    }

    let current_pc =
        myos_mm::VirtAddr::new(current_pc);

    assert!(
        crate::arch::memory::layout::
            KERNEL_IMAGE.contains(current_pc),
        "RISC-V is not executing in the high kernel image",
    );

    let image =
        crate::linker::kernel_image_layout();

    let text =
        image.segments()[0];

    let high_address =
        crate::arch::memory::layout::
            kernel_image_virtual_address(
                text.physical().start(),
            )
            .expect("unable to calculate high text address");

    let direct_pointer =
        crate::arch::memory::phys_access::
            ram_ptr::<u8>(
                text.physical().start(),
            )
            .expect("kernel text is absent from direct map");

    let high_byte = unsafe {
        read_volatile(
            high_address.get() as *const u8,
        )
    };

    let direct_byte = unsafe {
        read_volatile(direct_pointer)
    };

    assert_eq!(
        high_byte,
        direct_byte,
    );

    /*
     * 正式页表不应再包含低地址内核映射。
     */
    let low_boot =
        myos_mm::VirtAddr::new(
            crate::arch::memory::layout::
                BOOT_PHYS_BASE.get(),
        );

    assert_eq!(
        state
            .boot_page_table()
            .translate(low_boot)
            .expect(
                "failed to inspect low boot mapping",
            ),
        None,
        "final page table still maps the low boot image",
    );

    crate::println!("RISC-V final address space:");
    crate::println!(
        "  current PC      : {:#018x}",
        current_pc.get(),
    );
    crate::println!(
        "  high text       : {:#018x}",
        high_address.get(),
    );
    crate::println!(
        "  direct map      : verified",
    );
    crate::println!(
        "  low boot mapping: removed",
    );
}

#[cfg(target_arch = "loongarch64")]
pub fn verify_loongarch_high_mapping() {
    use core::ptr::read_volatile;

    crate::arch::memory::dmw::assert_configured();

    let cached_pc = crate::arch::memory::dmw::current_pc();

    let physical = crate::arch::memory::layout::cached_to_phys(cached_pc)
        .expect("current LoongArch PC is not in cached DMW");

    let uncached_alias = crate::arch::memory::layout::phys_to_uncached(physical)
        .expect("current code is outside uncached DMW range");

    /*
     * 同一物理代码字节分别通过 cached 和 uncached
     * 高地址别名读取。
     */
    let cached_byte = unsafe { read_volatile(cached_pc.get() as *const u8) };

    let uncached_byte = unsafe { read_volatile(uncached_alias.get() as *const u8) };

    assert_eq!(
        cached_byte, uncached_byte,
        "LoongArch cached and uncached DMW aliases disagree",
    );

    crate::println!("LoongArch DMW:");
    crate::println!("  current PC      : {:#018x}", cached_pc.get(),);
    crate::println!("  physical alias  : {:#018x}", physical.get(),);
    crate::println!("  uncached alias  : {:#018x}", uncached_alias.get(),);
    crate::println!(
        "  DMW0            : {:#018x}",
        crate::arch::memory::dmw::dmw0(),
    );
    crate::println!(
        "  DMW1            : {:#018x}",
        crate::arch::memory::dmw::dmw1(),
    );
    crate::println!("  high execution  : verified",);
}

#[cfg(target_arch = "riscv64")]
pub fn prepare_riscv_direct_map(
    state: &mut EarlyMemoryState,
    tree: &DeviceTree<'_>,
) {
    use myos_mm::{
        MappingOptions,
        MemoryMap,
        PhysAddr,
        PhysFrame,
        PhysRange,
        VirtPage,
        PAGE_SIZE,
    };

    /*
     * direct map 应覆盖固件报告的全部普通 RAM，而不是只覆盖
     * 当前 free list。内核、FDT 和固件保留区仍属于 RAM，
     * 只是不能交给页分配器。
     */
    let mut ram =
        MemoryMap::<MEMORY_MAP_CAPACITY>::new();

    for region in tree.memory_regions() {
        let range = PhysRange::from_start_size(
            PhysAddr::new(region.start()),
            region.size(),
        )
        .expect("invalid FDT RAM range");

        ram.add_usable(range)
            .expect("too many physical RAM ranges");
    }

    let image =
        crate::linker::kernel_image_layout();

    let mut mapped_pages = 0_usize;

    let (allocator, page_table) =
        state.parts_mut();

    for range in ram.iter() {
        let mut physical = range.start();

        while physical < range.end() {
            let virtual_address =
                crate::arch::memory::layout::phys_to_direct(
                    physical,
                )
                .unwrap_or_else(|| {
                    panic!(
                        "RAM address is outside RISC-V \
                         direct map: {:#x}",
                        physical.get(),
                    );
                });

            let page =
                VirtPage::from_start_address(
                    virtual_address,
                )
                .expect("direct-map VA is unaligned");

            let frame =
                PhysFrame::from_start_address(
                    physical,
                )
                .expect("RAM frame is unaligned");

            let options =
                direct_map_options(
                    physical,
                    &image,
                );

            page_table
                .map_page(
                    allocator,
                    page,
                    frame,
                    options,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "unable to map direct-map page \
                         {:#x}: {error:?}",
                        physical.get(),
                    );
                });

            physical = physical
                .checked_add(PAGE_SIZE)
                .expect("RAM iterator overflow");

            mapped_pages += 1;
        }
    }

    crate::println!("RISC-V direct map:");
    crate::println!(
        "  mapped pages : {}",
        mapped_pages,
    );
    crate::println!(
        "  table pages  : {}",
        page_table.allocated_table_pages(),
    );
}

#[cfg(target_arch = "riscv64")]
fn direct_map_options(
    physical: myos_mm::PhysAddr,
    image: &crate::linker::KernelImageLayout,
) -> myos_mm::MappingOptions {
    for segment in image.segments() {
        if !segment.physical().contains(physical) {
            continue;
        }

        /*
         * text 的 direct-map 别名只读且不可执行。
         *
         * 正式高半映像仍然是 RX，避免出现额外的可执行别名。
         */
        if segment
            .options()
            .permissions()
            .is_executable()
        {
            return myos_mm::MappingOptions::
                kernel_rodata();
        }

        return segment.options();
    }

    myos_mm::MappingOptions::kernel_data()
}

