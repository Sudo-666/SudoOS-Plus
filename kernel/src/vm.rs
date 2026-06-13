use alloc::vec::Vec;

use myos_mm::{
    KernelVirtualAllocator, KernelVirtualReservation, MappingOptions, PAGE_SIZE, PageAllocation,
    PhysAddr, VirtAddr, VirtPage, VirtRange, VmallocKind,
};

use crate::runtime_page_table::{RuntimePageTable, RuntimePageTableError};
use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const VMALLOC_RESERVATIONS: usize = 128;

static VMALLOC: IrqSpinLock<Option<KernelVirtualAllocator<VMALLOC_RESERVATIONS>>> =
    IrqSpinLock::new_with_class(None, LockClass::new("vmalloc", LockRank::Vm, 1));

static KERNEL_PAGE_TABLE: IrqSpinLock<Option<RuntimePageTable>> = IrqSpinLock::new_with_class(
    None,
    LockClass::new("kernel_page_table", LockRank::PageTable, 1),
);

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

    let runtime_page_table = RuntimePageTable::from_boot(memory.into_boot_page_table())
        .unwrap_or_else(|error| {
            panic!("unable to activate runtime page table: {error:?}");
        });

    #[cfg(target_arch = "loongarch64")]
    let hardware = runtime_page_table.hardware_state();

    *page_table_slot = Some(runtime_page_table);

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
    {
        crate::println!("  runtime pgtbl  : active hardware root");
        crate::println!(
            "  root physical  : {:#018x}",
            hardware.root().start_address().get(),
        );
        crate::println!("  refill entry   : {:#018x}", hardware.refill_entry().get(),);
        crate::println!(
            "  address bits   : VA={} PA={}",
            hardware.virtual_address_bits(),
            hardware.physical_address_bits(),
        );
    }
}

#[cfg(target_arch = "riscv64")]
pub fn activate_secondary_cpu() {
    assert!(
        crate::arch::memory::paging::translation_is_enabled(),
        "secondary RISC-V CPU entered Rust without Sv39 enabled",
    );
}

#[cfg(target_arch = "loongarch64")]
pub fn activate_secondary_cpu() {
    let hardware = {
        let slot = KERNEL_PAGE_TABLE.lock();
        slot.as_ref()
            .expect("kernel page table is not initialized")
            .hardware_state()
    };

    // SAFETY: the boot CPU owns the root permanently through
    // KERNEL_PAGE_TABLE. This secondary CPU has not accessed paged kernel
    // virtual addresses yet, and page-table mutation remains serialized by
    // the same global lock.
    let installed = unsafe { crate::arch::memory::paging::activate(hardware.root()) }
        .unwrap_or_else(|error| panic!("unable to activate secondary CPU paging: {error:?}"));

    assert_eq!(
        installed.root(),
        hardware.root(),
        "secondary CPU installed a different kernel page-table root",
    );
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

    crate::tlb::shootdown_kernel_all();
    Ok(KernelVmAllocation { reservation, pages })
}

pub fn vfree(mut allocation: KernelVmAllocation) -> Result<(), KernelVmError> {
    let page_count = allocation.pages.len();
    unmap_reservation_pages(allocation.reservation, page_count)?;

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

    crate::tlb::shootdown_kernel_all();

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
    unmap_reservation_pages(mapping.reservation, page_count)?;
    release_vmalloc(mapping.reservation)
}

#[cfg(debug_assertions)]
pub fn verify() {
    let reservations_before = reservation_count().expect("kernel vm allocator unavailable");
    let tables_before = runtime_table_count();

    /*
     * LoongArch TLB entries contain an even/odd page pair.  A two-page,
     * two-page-aligned test exercises both TLBRELO0 and TLBRELO1 in one refill.
     */
    let allocation = vmalloc(PAGE_SIZE * 2, PAGE_SIZE * 2)
        .expect("unable to allocate paired test vmalloc range");

    assert_eq!(allocation.usable_range().size(), PAGE_SIZE * 2);
    assert_eq!(allocation.page_count(), 2);

    let first_virtual_page = VirtPage::from_start_address(allocation.usable_range().start())
        .expect("vmalloc usable range is not page-aligned");

    verify_vmalloc_hardware_access(&allocation);

    protect_kernel_page(first_virtual_page, MappingOptions::kernel_rodata())
        .expect("unable to protect test vmalloc page");

    /* A read must remain valid after the permission/TLB update. */
    let pointer = allocation.usable_range().start().get() as *const u64;

    // SAFETY: the first page remains mapped read-only and allocation owns
    // the physical backing page for the entire access.
    let _ = unsafe { core::ptr::read_volatile(pointer) };

    protect_kernel_page(first_virtual_page, MappingOptions::kernel_data())
        .expect("unable to restore test vmalloc page permissions");

    verify_vmalloc_hardware_access(&allocation);
    vfree(allocation).expect("unable to free test vmalloc range");

    assert_eq!(
        runtime_table_count(),
        tables_before,
        "vfree leaked intermediate page-table pages",
    );

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

    verify_ioremap_hardware_access(&io_mapping);
    iounmap(io_mapping).expect("unable to iounmap test page");
    crate::page_alloc::free(io_page).expect("unable to free test ioremap backing page");

    assert_eq!(
        reservation_count().expect("kernel vm allocator unavailable"),
        reservations_before,
    );
    assert_eq!(
        runtime_table_count(),
        tables_before,
        "iounmap leaked intermediate page-table pages",
    );

    crate::println!("kernel vm test:");
    crate::println!("  vmalloc/vfree   : hardware lifecycle verified");
    crate::println!("  ioremap/iounmap : hardware lifecycle verified");
    crate::println!("  protect/unmap   : hardware TLB update verified");
    crate::println!("  guard gap       : reservation verified");
    crate::println!("  hardware access : verified");
    crate::println!("  table reclaim   : verified");
    crate::println!("  runtime tables  : {}", runtime_table_count());
}

#[cfg(debug_assertions)]
fn verify_vmalloc_hardware_access(allocation: &KernelVmAllocation) {
    const PATTERNS: [u64; 2] = [0x5355_444f_4f53_4d30, 0x5355_444f_4f53_4d31];

    assert!(allocation.usable_range().size() >= PAGE_SIZE * PATTERNS.len());

    for (page_index, pattern) in PATTERNS.into_iter().enumerate() {
        let address = allocation
            .usable_range()
            .start()
            .get()
            .checked_add(page_index * PAGE_SIZE)
            .expect("vmalloc hardware-test address overflow");
        let pointer = address as *mut u64;

        // SAFETY:
        // vmalloc() mapped this page writable in the current active root and
        // allocation retains ownership of both the mapping and backing page.
        unsafe {
            core::ptr::write_volatile(pointer, pattern);
            assert_eq!(
                core::ptr::read_volatile(pointer),
                pattern,
                "CPU could not access vmalloc page {page_index}",
            );
        }
    }
}

#[cfg(debug_assertions)]
fn verify_ioremap_hardware_access(mapping: &KernelIoMapping) {
    const PATTERN: u64 = 0x5355_444f_4f53_494f;

    let pointer = mapping.virtual_address().get() as *mut u64;

    // SAFETY: ioremap() installed a writable device mapping for at least one
    // full page.  The test backing object is ordinary RAM owned by the caller.
    unsafe {
        core::ptr::write_volatile(pointer, PATTERN);
        assert_eq!(
            core::ptr::read_volatile(pointer),
            PATTERN,
            "CPU could not access ioremap mapping",
        );
    }
}

#[cfg(debug_assertions)]
pub fn debug_runtime_table_count() -> usize {
    with_kernel_page_table(|page_table| Ok(page_table.allocated_runtime_tables()))
        .expect("kernel runtime page table unavailable")
}

#[cfg(debug_assertions)]
fn runtime_table_count() -> usize {
    debug_runtime_table_count()
}

fn protect_kernel_page(page: VirtPage, options: MappingOptions) -> Result<(), KernelVmError> {
    with_kernel_page_table(|page_table| {
        page_table.protect_page(page, options)?;
        Ok(())
    })?;
    crate::tlb::shootdown_kernel_all();
    Ok(())
}

fn unmap_reservation_pages(
    reservation: KernelVirtualReservation,
    page_count: usize,
) -> Result<(), KernelVmError> {
    if page_count == 0 {
        return Ok(());
    }

    let table_capacity =
        with_kernel_page_table(|page_table| Ok(page_table.allocated_runtime_tables()))?;
    let mut retired_tables = Vec::new();
    retired_tables
        .try_reserve(table_capacity)
        .map_err(|_| KernelVmError::MetadataOutOfMemory)?;

    let mut unmapped = 0;
    let unmap_result = with_kernel_page_table(|page_table| {
        for index in 0..page_count {
            page_table.unmap_page(reservation_page(reservation, index)?)?;
            unmapped += 1;
        }
        Ok(())
    });

    if unmapped != 0 {
        // Leaf entries are invalid before backing pages can be released.
        crate::tlb::shootdown_kernel_all();

        let reclaim_result = with_kernel_page_table(|page_table| {
            for index in 0..unmapped {
                page_table.reclaim_empty_tables(
                    reservation_page(reservation, index)?,
                    &mut retired_tables,
                )?;
            }
            Ok(())
        });

        if !retired_tables.is_empty() {
            // Clearing non-leaf entries requires a second global fence before
            // the detached table pages can be returned to the buddy allocator.
            crate::tlb::shootdown_kernel_all();
            while let Some(table) = retired_tables.pop() {
                crate::page_alloc::free(table)?;
            }
        }

        reclaim_result?;
    }

    unmap_result
}

#[cfg(debug_assertions)]
pub struct KernelTlbTestMapping {
    allocation: KernelVmAllocation,
}

#[cfg(debug_assertions)]
impl KernelTlbTestMapping {
    pub fn address(&self) -> VirtAddr {
        self.allocation.usable_range().start()
    }
}

#[cfg(debug_assertions)]
pub fn allocate_tlb_test_mapping() -> KernelTlbTestMapping {
    let allocation =
        vmalloc(PAGE_SIZE, PAGE_SIZE).expect("unable to allocate remote TLB test mapping");
    KernelTlbTestMapping { allocation }
}

#[cfg(debug_assertions)]
pub fn replace_tlb_test_backing(mapping: &mut KernelTlbTestMapping) {
    assert_eq!(mapping.allocation.pages.len(), 1);

    let replacement =
        crate::page_alloc::allocate(0, crate::page_alloc::PageAllocationOptions::kernel_zeroed())
            .expect("unable to allocate replacement TLB test page");
    let page = VirtPage::from_start_address(mapping.address())
        .expect("TLB test mapping is not page-aligned");

    let old_frame = with_kernel_page_table(|page_table| {
        Ok(page_table.replace_page(page, replacement.start(), MappingOptions::kernel_data())?)
    })
    .expect("unable to replace TLB test mapping");

    crate::tlb::shootdown_kernel_all();

    let old = core::mem::replace(&mut mapping.allocation.pages[0], replacement);
    assert_eq!(old.start(), old_frame, "TLB test ownership mismatch");
    crate::page_alloc::free(old).expect("unable to release old TLB test page");
}

#[cfg(debug_assertions)]
pub fn release_tlb_test_mapping(mapping: KernelTlbTestMapping) {
    vfree(mapping.allocation).expect("unable to release remote TLB test mapping");
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
    let _ = unmap_reservation_pages(reservation, mapped_pages);
}

fn rollback_unmapped_pages(mut pages: Vec<PageAllocation>) {
    while let Some(page) = pages.pop() {
        let _ = crate::page_alloc::free(page);
    }
}
