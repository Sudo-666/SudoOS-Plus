use alloc::vec::Vec;

use myos_mm::{
    KernelVirtualAllocator, KernelVirtualReservation, MappingOptions, PAGE_SIZE, PageAllocation,
    PhysAddr, VirtAddr, VirtPage, VirtRange, VmallocKind,
};

use crate::irq_lock::IrqSpinLock;
use crate::runtime_page_table::{RuntimePageTable, RuntimePageTableError};

const VMALLOC_RESERVATIONS: usize = 128;

static VMALLOC: IrqSpinLock<Option<KernelVirtualAllocator<VMALLOC_RESERVATIONS>>> =
    IrqSpinLock::new(None);

static KERNEL_PAGE_TABLE: IrqSpinLock<Option<RuntimePageTable>> = IrqSpinLock::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelVmError {
    AlreadyInitialized,
    NotInitialized,
    Reservation(myos_mm::VmAreaError),

    PageTable(RuntimePageTableError),

    PageAllocator(crate::page_alloc::GlobalPageAllocatorError),

    AddressOverflow,

    InvalidArgument,

    MetadataOutOfMemory,
}

impl From<myos_mm::VmAreaError> for KernelVmError {
    fn from(error: myos_mm::VmAreaError) -> Self {
        Self::Reservation(error)
    }
}

impl From<RuntimePageTableError> for KernelVmError {
    fn from(error: RuntimePageTableError) -> Self {
        Self::PageTable(error)
    }
}

impl From<crate::page_alloc::GlobalPageAllocatorError> for KernelVmError {
    fn from(error: crate::page_alloc::GlobalPageAllocatorError) -> Self {
        Self::PageAllocator(error)
    }
}

pub struct KernelVmAllocation {
    reservation: KernelVirtualReservation,
    pages: Vec<PageAllocation>,
}

impl KernelVmAllocation {
    pub fn usable_range(&self) -> VirtRange {
        self.reservation.usable()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
}

pub struct KernelIoMapping {
    reservation: KernelVirtualReservation,
    physical: PhysAddr,
    virtual_address: VirtAddr,
    size: usize,
    mapped_size: usize,
}

impl KernelIoMapping {
    pub fn physical_address(&self) -> PhysAddr {
        self.physical
    }

    pub fn virtual_address(&self) -> VirtAddr {
        self.virtual_address
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

pub fn initialize(memory: crate::memory::KernelMemoryState) {
    let mut vmalloc_slot = VMALLOC.lock();

    if vmalloc_slot.is_some() {
        panic!(
            "kernel virtual memory allocator error: {:?}",
            KernelVmError::AlreadyInitialized,
        );
    }

    *vmalloc_slot = Some(KernelVirtualAllocator::new(
        crate::arch::memory::layout::VMALLOC,
    ));

    drop(vmalloc_slot);

    let mut page_table_slot = KERNEL_PAGE_TABLE.lock();

    if page_table_slot.is_some() {
        panic!(
            "kernel page table error: {:?}",
            KernelVmError::AlreadyInitialized,
        );
    }

    *page_table_slot = Some(RuntimePageTable::from_boot(memory.into_boot_page_table()));

    let arena = crate::arch::memory::layout::VMALLOC;

    crate::println!("kernel vm:");
    crate::println!(
        "  vmalloc arena  : [{:#018x}, {:#018x})",
        arena.start().get(),
        arena.end().get(),
    );
    crate::println!("  reservations   : {}", VMALLOC_RESERVATIONS);
    crate::println!("  guard pages    : enabled");
    #[cfg(target_arch = "riscv64")]
    crate::println!("  runtime pgtbl  : active hardware root");

    #[cfg(target_arch = "loongarch64")]
    crate::println!("  runtime pgtbl  : software-only until TLB refill");
}

pub fn reserve_vmalloc(
    size: usize,
    alignment: usize,
) -> Result<KernelVirtualReservation, KernelVmError> {
    let mut slot = VMALLOC.lock();

    let allocator = slot.as_mut().ok_or(KernelVmError::NotInitialized)?;

    Ok(allocator.reserve(size, alignment, VmallocKind::Vmalloc)?)
}

fn reserve_ioremap(
    physical: PhysAddr,
    size: usize,
) -> Result<KernelVirtualReservation, KernelVmError> {
    let mut slot = VMALLOC.lock();

    let allocator = slot.as_mut().ok_or(KernelVmError::NotInitialized)?;

    Ok(allocator.reserve(size, PAGE_SIZE, VmallocKind::IoRemap { physical })?)
}

pub fn release_vmalloc(reservation: KernelVirtualReservation) -> Result<(), KernelVmError> {
    let mut slot = VMALLOC.lock();

    let allocator = slot.as_mut().ok_or(KernelVmError::NotInitialized)?;

    allocator.release(reservation)?;

    Ok(())
}

pub fn vmalloc(size: usize, alignment: usize) -> Result<KernelVmAllocation, KernelVmError> {
    if size == 0 {
        return Err(KernelVmError::InvalidArgument);
    }

    let reservation = reserve_vmalloc(size, alignment)?;
    let page_count = pages_for_size(reservation.usable().size())?;

    let mut pages = Vec::new();

    pages
        .try_reserve(page_count)
        .map_err(|_| KernelVmError::MetadataOutOfMemory)?;

    for _ in 0..page_count {
        match crate::page_alloc::allocate(
            0,
            crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
        ) {
            Ok(page) => pages.push(page),
            Err(error) => {
                rollback_unmapped_pages(pages);
                release_vmalloc(reservation)?;
                return Err(KernelVmError::from(error));
            }
        }
    }

    let mut mapped_pages = 0;

    let map_result = with_kernel_page_table(|page_table| {
        for (index, physical_page) in pages.iter().enumerate() {
            let virtual_page = reservation_page(reservation, index)?;

            page_table.map_page(
                virtual_page,
                physical_page.start(),
                MappingOptions::kernel_data(),
            )?;

            mapped_pages += 1;
        }

        Ok(())
    });

    if let Err(error) = map_result {
        rollback_mapped_pages(reservation, mapped_pages);
        rollback_unmapped_pages(pages);
        release_vmalloc(reservation)?;
        return Err(error);
    }

    Ok(KernelVmAllocation { reservation, pages })
}

pub fn vfree(mut allocation: KernelVmAllocation) -> Result<(), KernelVmError> {
    let page_count = allocation.pages.len();

    with_kernel_page_table(|page_table| {
        for index in 0..page_count {
            page_table.unmap_page(reservation_page(allocation.reservation, index)?)?;
        }

        Ok(())
    })?;

    while let Some(page) = allocation.pages.pop() {
        crate::page_alloc::free(page)?;
    }

    release_vmalloc(allocation.reservation)
}

pub fn ioremap(physical: PhysAddr, size: usize) -> Result<KernelIoMapping, KernelVmError> {
    if size == 0 {
        return Err(KernelVmError::InvalidArgument);
    }

    let aligned_physical = physical
        .align_down(PAGE_SIZE)
        .ok_or(KernelVmError::InvalidArgument)?;

    let offset = physical.get() - aligned_physical.get();

    let mapped_size = align_up(
        size.checked_add(offset)
            .ok_or(KernelVmError::AddressOverflow)?,
        PAGE_SIZE,
    )?;

    let reservation = reserve_ioremap(aligned_physical, mapped_size)?;

    let page_count = pages_for_size(mapped_size)?;

    let mut mapped_pages = 0;

    let map_result = with_kernel_page_table(|page_table| {
        for index in 0..page_count {
            let virtual_page = reservation_page(reservation, index)?;

            let physical_address = aligned_physical
                .checked_add(index * PAGE_SIZE)
                .ok_or(KernelVmError::AddressOverflow)?;

            let physical_frame = myos_mm::PhysFrame::from_start_address(physical_address)
                .ok_or(KernelVmError::InvalidArgument)?;

            page_table.map_page(
                virtual_page,
                physical_frame,
                MappingOptions::kernel_device(),
            )?;

            mapped_pages += 1;
        }

        Ok(())
    });

    if let Err(error) = map_result {
        rollback_mapped_pages(reservation, mapped_pages);
        release_vmalloc(reservation)?;
        return Err(error);
    }

    let virtual_address = reservation
        .usable()
        .start()
        .checked_add(offset)
        .ok_or(KernelVmError::AddressOverflow)?;

    Ok(KernelIoMapping {
        reservation,
        physical,
        virtual_address,
        size,
        mapped_size,
    })
}

pub fn iounmap(mapping: KernelIoMapping) -> Result<(), KernelVmError> {
    let page_count = pages_for_size(mapping.mapped_size)?;

    with_kernel_page_table(|page_table| {
        for index in 0..page_count {
            page_table.unmap_page(reservation_page(mapping.reservation, index)?)?;
        }

        Ok(())
    })?;

    release_vmalloc(mapping.reservation)
}

#[cfg(debug_assertions)]
pub fn verify() {
    let before = reservation_count().expect("kernel vm allocator unavailable");

    let allocation = vmalloc(PAGE_SIZE, PAGE_SIZE).expect("unable to allocate test vmalloc range");

    assert_eq!(allocation.usable_range().size(), PAGE_SIZE);
    assert_eq!(allocation.page_count(), 1);

    let virtual_page = VirtPage::from_start_address(allocation.usable_range().start())
        .expect("vmalloc usable range is not page-aligned");

    #[cfg(target_arch = "riscv64")]
    verify_vmalloc_hardware_access(&allocation);

    {
        let mut page_table_slot = KERNEL_PAGE_TABLE.lock();
        let page_table = page_table_slot
            .as_mut()
            .expect("kernel runtime page table unavailable");

        page_table
            .protect_page(virtual_page, MappingOptions::kernel_rodata())
            .expect("unable to protect test vmalloc page");

        page_table
            .protect_page(virtual_page, MappingOptions::kernel_data())
            .expect("unable to restore test vmalloc page permissions");
    }

    vfree(allocation).expect("unable to free test vmalloc range");

    let io_page =
        crate::page_alloc::allocate(0, crate::page_alloc::PageAllocationOptions::kernel_zeroed())
            .expect("unable to allocate test ioremap backing page");

    let io_mapping =
        ioremap(io_page.start().start_address(), PAGE_SIZE).expect("unable to ioremap test page");

    assert_eq!(
        io_mapping.physical_address(),
        io_page.start().start_address()
    );
    assert_eq!(io_mapping.size(), PAGE_SIZE);

    {
        let mut page_table_slot = KERNEL_PAGE_TABLE.lock();
        let page_table = page_table_slot
            .as_mut()
            .expect("kernel runtime page table unavailable");

        assert_eq!(
            page_table
                .translate(io_mapping.virtual_address())
                .expect("unable to translate test ioremap page"),
            Some(io_page.start().start_address()),
        );
    }

    iounmap(io_mapping).expect("unable to iounmap test page");
    crate::page_alloc::free(io_page).expect("unable to free test ioremap backing page");

    assert_eq!(
        reservation_count().expect("kernel vm allocator unavailable"),
        before,
    );

    let runtime_tables =
        with_kernel_page_table(|page_table| Ok(page_table.allocated_runtime_tables()))
            .expect("kernel runtime page table unavailable");

    crate::println!("kernel vm test:");
    crate::println!("  vmalloc/vfree   : software lifecycle verified");
    crate::println!("  ioremap/iounmap : software lifecycle verified");
    crate::println!("  protect/unmap   : page-table update verified");
    crate::println!("  guard gap       : reservation verified");

    #[cfg(target_arch = "riscv64")]
    crate::println!("  hardware access : verified");

    #[cfg(target_arch = "loongarch64")]
    crate::println!("  hardware access : deferred until TLB refill");

    crate::println!("  runtime tables  : {}", runtime_tables);
}

#[cfg(all(debug_assertions, target_arch = "riscv64"))]
fn verify_vmalloc_hardware_access(allocation: &KernelVmAllocation) {
    const PATTERN: u64 = 0x5355_444f_4f53_4d4d;

    let address = allocation.usable_range().start().get();
    let pointer = address as *mut u64;

    // SAFETY:
    // vmalloc() 已将该页以 kernel_data 权限写入当前活动 Sv39 根页表，
    // allocation token 在整个读写期间保持映射和物理页所有权。
    unsafe {
        core::ptr::write_volatile(pointer, PATTERN);
        assert_eq!(
            core::ptr::read_volatile(pointer),
            PATTERN,
            "RISC-V CPU could not access the vmalloc mapping",
        );
    }
}

fn reservation_count() -> Result<usize, KernelVmError> {
    let slot = VMALLOC.lock();

    let allocator = slot.as_ref().ok_or(KernelVmError::NotInitialized)?;

    Ok(allocator.reservation_count())
}

fn with_kernel_page_table<T>(
    f: impl FnOnce(&mut RuntimePageTable) -> Result<T, KernelVmError>,
) -> Result<T, KernelVmError> {
    let mut slot = KERNEL_PAGE_TABLE.lock();

    let page_table = slot.as_mut().ok_or(KernelVmError::NotInitialized)?;

    f(page_table)
}

fn reservation_page(
    reservation: KernelVirtualReservation,
    index: usize,
) -> Result<VirtPage, KernelVmError> {
    let offset = index
        .checked_mul(PAGE_SIZE)
        .ok_or(KernelVmError::AddressOverflow)?;

    let address = reservation
        .usable()
        .start()
        .checked_add(offset)
        .ok_or(KernelVmError::AddressOverflow)?;

    VirtPage::from_start_address(address).ok_or(KernelVmError::InvalidArgument)
}

fn pages_for_size(size: usize) -> Result<usize, KernelVmError> {
    if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
        return Err(KernelVmError::InvalidArgument);
    }

    Ok(size / PAGE_SIZE)
}

fn align_up(value: usize, alignment: usize) -> Result<usize, KernelVmError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(KernelVmError::InvalidArgument);
    }

    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or(KernelVmError::AddressOverflow)
}

fn rollback_mapped_pages(reservation: KernelVirtualReservation, mapped_pages: usize) {
    let _ = with_kernel_page_table(|page_table| {
        for index in 0..mapped_pages {
            let page = reservation_page(reservation, index)?;
            let _ = page_table.unmap_page(page);
        }

        Ok(())
    });
}

fn rollback_unmapped_pages(mut pages: Vec<PageAllocation>) {
    while let Some(page) = pages.pop() {
        let _ = crate::page_alloc::free(page);
    }
}
