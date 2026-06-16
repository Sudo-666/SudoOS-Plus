use alloc::vec::Vec;
use core::cmp::min;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use myos_mm::{
    AsidAllocator, AsidAllocatorError, AsidToken, FaultAccess, PAGE_SIZE, PageAllocation, PhysAddr,
    UserAddressSpace, UserMmError, VirtAddr, VirtPage, VmArea, VmAreaFlags,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::runtime_page_table::{RuntimePageTable, RuntimePageTableError};

const VMA_CAPACITY: usize = 16;

static ASID_ALLOCATOR: IrqSpinLock<Option<AsidAllocator>> =
    IrqSpinLock::new_with_class(None, LockClass::new("user_asid_allocator", LockRank::Vm, 1));
static ACTIVE_MM: AtomicPtr<UserMm> = AtomicPtr::new(ptr::null_mut());
static ASID_ROLLOVER_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LIVE_MMS: AtomicUsize = AtomicUsize::new(0);
static LIVE_ROOTS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BACKINGS: AtomicUsize = AtomicUsize::new(0);

#[allow(dead_code)]
#[derive(Debug)]
pub enum UserMmRuntimeError {
    AddressOverflow,
    AlreadyActive,
    Asid(AsidAllocatorError),
    AsidRolloverInProgress,
    AsidRolloverWithLiveMms,
    Core(UserMmError),
    InvalidRange,
    MetadataOutOfMemory,
    NoActiveAddressSpace,
    NotMapped,
    PageAllocator(crate::page_alloc::GlobalPageAllocatorError),
    PageTable(RuntimePageTableError),
    PermissionDenied,
    Vm(crate::vm::KernelVmError),
}

impl From<AsidAllocatorError> for UserMmRuntimeError {
    fn from(error: AsidAllocatorError) -> Self {
        Self::Asid(error)
    }
}

impl From<UserMmError> for UserMmRuntimeError {
    fn from(error: UserMmError) -> Self {
        Self::Core(error)
    }
}

impl From<crate::page_alloc::GlobalPageAllocatorError> for UserMmRuntimeError {
    fn from(error: crate::page_alloc::GlobalPageAllocatorError) -> Self {
        Self::PageAllocator(error)
    }
}

impl From<RuntimePageTableError> for UserMmRuntimeError {
    fn from(error: RuntimePageTableError) -> Self {
        Self::PageTable(error)
    }
}

impl From<crate::vm::KernelVmError> for UserMmRuntimeError {
    fn from(error: crate::vm::KernelVmError) -> Self {
        Self::Vm(error)
    }
}

struct MappedPage {
    page: VirtPage,
    backing: PageAllocation,
}

struct UserMmState {
    core: UserAddressSpace<VMA_CAPACITY>,
    page_table: Option<RuntimePageTable>,
    pages: Vec<MappedPage>,
}

pub struct UserMm {
    state: IrqSpinLock<UserMmState>,
}

impl UserMm {
    pub fn new(areas: &[VmArea]) -> Result<Self, UserMmRuntimeError> {
        let asid = reserve_asid_for_mm()?;
        let result: Result<Self, UserMmRuntimeError> = (|| {
            let mut core = UserAddressSpace::new(crate::arch::memory::layout::USER_RANGE, asid);

            // Reject invalid VMA topology before allocating a hardware root so
            // metadata failure cannot leak page-table ownership.
            for area in areas {
                core.map_area(*area)?;
            }

            let page_table = crate::vm::create_user_page_table()?;
            LIVE_ROOTS.fetch_add(1, Ordering::AcqRel);
            Ok(Self {
                state: IrqSpinLock::new_with_class(
                    UserMmState {
                        core,
                        page_table: Some(page_table),
                        pages: Vec::new(),
                    },
                    LockClass::new("user_mm", LockRank::Vm, 2),
                ),
            })
        })();

        if result.is_err() {
            release_mm_reservation();
        }
        result
    }

    pub fn asid(&self) -> AsidToken {
        self.state.lock().core.asid()
    }

    pub fn root_is_private(&self) -> Result<bool, UserMmRuntimeError> {
        let state = self.state.lock();
        let page_table = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        Ok(page_table.is_owned_user_root()
            && page_table.root_frame() != crate::vm::kernel_page_table_root()?)
    }

    pub fn kernel_mapping_is_shared(
        &self,
        kernel_address: VirtAddr,
    ) -> Result<bool, UserMmRuntimeError> {
        #[cfg(target_arch = "riscv64")]
        {
            let state = self.state.lock();
            let user = state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .translate(kernel_address)?;
            let kernel = crate::vm::kernel_translate(kernel_address)?;
            Ok(user.is_some() && user == kernel)
        }

        #[cfg(target_arch = "loongarch64")]
        {
            let _ = kernel_address;
            Ok(crate::arch::memory::paging::current_upper_root()
                == crate::vm::kernel_page_table_root()?)
        }
    }

    /// Explicitly installs one zeroed page. B3 deliberately does not call this
    /// from the fault path; demand allocation remains behind the B2 planner.
    pub fn populate_page(&self, address: VirtAddr) -> Result<PhysAddr, UserMmRuntimeError> {
        let mut state = self.state.lock();
        let area = state
            .core
            .layout()
            .find_area(address)
            .ok_or(UserMmRuntimeError::PermissionDenied)?;
        let page_address = address
            .align_down(PAGE_SIZE)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let page =
            VirtPage::from_start_address(page_address).ok_or(UserMmRuntimeError::InvalidRange)?;

        if let Some(physical) = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .translate(address)?
        {
            return Ok(physical);
        }

        state
            .pages
            .try_reserve(1)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        let backing = crate::page_alloc::allocate(
            0,
            crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
        )?;
        let offset = address
            .get()
            .checked_sub(page_address.get())
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let physical = match backing.start().start_address().checked_add(offset) {
            Some(physical) => physical,
            None => {
                crate::page_alloc::free(backing)?;
                return Err(UserMmRuntimeError::AddressOverflow);
            }
        };
        let page_table = state
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        if let Err(error) = page_table.map_page(page, backing.start(), area.mapping_options()) {
            crate::page_alloc::free(backing)?;
            return Err(error.into());
        }

        state.pages.push(MappedPage { page, backing });
        LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
        Ok(physical)
    }

    pub fn copy_from_user(
        &self,
        address: usize,
        output: &mut [u8],
    ) -> Result<(), UserMmRuntimeError> {
        if output.is_empty() {
            return Ok(());
        }

        let state = self.state.lock();
        validate_range(&state, address, output.len(), FaultAccess::Read)?;
        let page_table = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        validate_mapped_range(page_table, address, output.len())?;

        let mut copied = 0;
        while copied < output.len() {
            let current = address
                .checked_add(copied)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let physical = page_table
                .translate(VirtAddr::new(current))?
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let in_page = current & (PAGE_SIZE - 1);
            let chunk = min(PAGE_SIZE - in_page, output.len() - copied);
            let source = crate::arch::memory::phys_access::ram_ptr::<u8>(physical)
                .map_err(|_| UserMmRuntimeError::NotMapped)?;

            // SAFETY: VMA permissions and every translated page were checked;
            // output owns at least `chunk` bytes from this offset.
            unsafe {
                core::ptr::copy_nonoverlapping(source, output.as_mut_ptr().add(copied), chunk);
            }
            copied += chunk;
        }
        Ok(())
    }

    pub fn copy_to_user(&self, address: usize, input: &[u8]) -> Result<(), UserMmRuntimeError> {
        if input.is_empty() {
            return Ok(());
        }

        let state = self.state.lock();
        validate_range(&state, address, input.len(), FaultAccess::Write)?;
        let page_table = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        validate_mapped_range(page_table, address, input.len())?;

        let mut copied = 0;
        while copied < input.len() {
            let current = address
                .checked_add(copied)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let physical = page_table
                .translate(VirtAddr::new(current))?
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let in_page = current & (PAGE_SIZE - 1);
            let chunk = min(PAGE_SIZE - in_page, input.len() - copied);
            let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(physical)
                .map_err(|_| UserMmRuntimeError::NotMapped)?;

            // SAFETY: VMA permissions and every translated page were checked;
            // input owns at least `chunk` bytes from this offset.
            unsafe {
                core::ptr::copy_nonoverlapping(input.as_ptr().add(copied), destination, chunk);
            }
            copied += chunk;
        }
        Ok(())
    }

    pub fn bind(&self) -> Result<(), UserMmRuntimeError> {
        let pointer = self as *const Self as *mut Self;
        ACTIVE_MM
            .compare_exchange(
                ptr::null_mut(),
                pointer,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| UserMmRuntimeError::AlreadyActive)?;
        Ok(())
    }

    pub fn unbind(&self) {
        let pointer = self as *const Self as *mut Self;
        let old = ACTIVE_MM.swap(ptr::null_mut(), Ordering::AcqRel);
        assert_eq!(old, pointer, "M8-B3 unbound a different active mm");
    }

    /// Installs the private root, synchronizes the local ASID, and publishes
    /// this CPU only after both hardware operations are complete.
    pub fn activate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();

        // Keep the generation stable across root installation and active-mask
        // publication. M9 can replace this fail-closed gate with lazy renewal.
        let mut allocator = ASID_ALLOCATOR.lock();
        ensure_asid_allocator(&mut *allocator)?;
        let current_asid_generation = allocator
            .as_ref()
            .expect("ASID allocator was just initialized")
            .generation();

        loop {
            let (root, token, tlb_generation) = {
                let mut state = self.state.lock();
                let token = state.core.asid();
                if !token.is_current(current_asid_generation) {
                    return Err(UserMmError::AsidMismatch.into());
                }
                let tlb_generation = state.core.tlb_generation();
                let page_table = state
                    .page_table
                    .as_mut()
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                crate::vm::synchronize_user_page_table(page_table)?;
                (page_table.root_frame(), token, tlb_generation)
            };

            // SAFETY: the owned root and every shared kernel table remain alive
            // throughout this synchronous, non-preemptible M8 verifier session.
            unsafe {
                crate::vm::activate_user_page_table(root, token.id());
            }
            crate::arch::memory::paging::flush_asid(token.id());

            let state = self.state.lock();
            match state.core.enter_cpu_after_local_sync(
                cpu,
                current_asid_generation,
                tlb_generation,
            ) {
                Ok(()) => return Ok(()),
                Err(UserMmError::TlbGenerationMismatch { .. }) => {
                    drop(state);
                    crate::arch::memory::paging::flush_asid(token.id());
                }
                Err(error) => {
                    drop(state);
                    // SAFETY: KERNEL_PAGE_TABLE permanently owns this root.
                    unsafe {
                        crate::vm::activate_kernel_page_table()?;
                    }
                    crate::arch::memory::paging::flush_asid(token.id());
                    return Err(error.into());
                }
            }
        }
    }

    /// Restores the kernel root and flushes the departing ASID before clearing
    /// this CPU from the mm's active mask.
    pub fn deactivate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();
        let token = self.asid();

        // SAFETY: KERNEL_PAGE_TABLE permanently owns this root.
        unsafe {
            crate::vm::activate_kernel_page_table()?;
        }
        crate::arch::memory::paging::flush_asid(token.id());

        loop {
            let state = self.state.lock();
            let generation = state.core.tlb_generation();
            match state.core.leave_cpu_after_local_flush(cpu, generation) {
                Ok(()) => break,
                Err(UserMmError::TlbGenerationMismatch { .. }) => {
                    drop(state);
                    crate::arch::memory::paging::flush_asid(token.id());
                }
                Err(error) => return Err(error.into()),
            }
        }

        assert_eq!(
            crate::arch::memory::paging::current_lower_root(),
            crate::vm::kernel_page_table_root()?,
            "M8-B3 failed to restore the kernel lower root",
        );
        assert_eq!(
            crate::arch::memory::paging::current_address_space_id(),
            myos_mm::AddressSpaceId::KERNEL,
            "M8-B3 failed to restore the kernel ASID",
        );
        Ok(())
    }

    pub fn assert_hardware_active(&self) -> Result<(), UserMmRuntimeError> {
        let state = self.state.lock();
        let root = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .root_frame();
        let asid = state.core.asid().id();
        let active = state.core.active_cpus();
        let cpu = crate::smp::current_cpu_id().get();

        assert_eq!(
            crate::arch::memory::paging::current_lower_root(),
            root,
            "M8-B3 hardware root does not match the active user mm",
        );
        assert_eq!(
            crate::arch::memory::paging::current_address_space_id(),
            asid,
            "M8-B3 hardware ASID does not match the active user mm",
        );
        assert_eq!(active.count(), 1, "M8-B3 published an unexpected CPU mask");
        let current_is_active = active.contains(cpu).map_err(UserMmError::from)?;
        assert!(
            current_is_active,
            "M8-B3 did not publish the current CPU in active_cpus",
        );
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<(), UserMmRuntimeError> {
        let self_pointer = self as *mut Self;
        assert_ne!(
            ACTIVE_MM.load(Ordering::Acquire),
            self_pointer,
            "M8-B3 attempted to destroy the bound mm",
        );

        let mut state = self.state.lock();
        state.core.assert_inactive_for_destroy()?;
        let table_capacity = state
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .allocated_runtime_tables();
        let mut retired = Vec::new();
        retired
            .try_reserve(table_capacity)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

        while let Some(mapping) = state.pages.pop() {
            let page_table = state
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let frame = page_table.unmap_page(mapping.page)?;
            assert_eq!(
                frame,
                mapping.backing.start(),
                "M8-B3 user leaf returned a different physical frame",
            );
            page_table.reclaim_empty_tables(mapping.page, &mut retired)?;
            crate::page_alloc::free(mapping.backing)?;
            LIVE_BACKINGS.fetch_sub(1, Ordering::AcqRel);
        }

        for table in retired.drain(..) {
            crate::page_alloc::free(table)?;
        }

        let page_table = state
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        assert_eq!(
            page_table.allocated_runtime_tables(),
            0,
            "M8-B3 retained private intermediate page tables",
        );
        page_table.release_empty()?;
        state.page_table = None;
        LIVE_ROOTS.fetch_sub(1, Ordering::AcqRel);
        release_mm_reservation();
        Ok(())
    }
}

impl Drop for UserMm {
    fn drop(&mut self) {
        let state = self.state.lock();
        assert!(
            state.page_table.is_none(),
            "M8-B3 UserMm dropped without explicit root teardown",
        );
        assert!(
            state.pages.is_empty(),
            "M8-B3 UserMm dropped with owned backing pages",
        );
        assert!(
            state.core.assert_inactive_for_destroy().is_ok(),
            "M8-B3 UserMm dropped while active on a CPU",
        );
    }
}

pub fn copy_from_active(address: usize, output: &mut [u8]) -> Result<(), UserMmRuntimeError> {
    with_active(|mm| mm.copy_from_user(address, output))
}

pub fn copy_to_active(address: usize, input: &[u8]) -> Result<(), UserMmRuntimeError> {
    with_active(|mm| mm.copy_to_user(address, input))
}

pub fn assert_no_leaks() {
    assert!(
        ACTIVE_MM.load(Ordering::Acquire).is_null(),
        "M8-B3 leaked an active mm pointer",
    );
    assert!(
        !ASID_ROLLOVER_IN_PROGRESS.load(Ordering::Acquire),
        "M8-B3 leaked the ASID rollover publication gate",
    );
    assert_eq!(
        LIVE_MMS.load(Ordering::Acquire),
        0,
        "M8-B3 leaked an MM reservation",
    );
    assert_eq!(
        LIVE_ROOTS.load(Ordering::Acquire),
        0,
        "M8-B3 leaked a user root"
    );
    assert_eq!(
        LIVE_BACKINGS.load(Ordering::Acquire),
        0,
        "M8-B3 leaked a user backing page",
    );
}

fn with_active<T>(
    f: impl FnOnce(&UserMm) -> Result<T, UserMmRuntimeError>,
) -> Result<T, UserMmRuntimeError> {
    let pointer = ACTIVE_MM.load(Ordering::Acquire);
    if pointer.is_null() {
        return Err(UserMmRuntimeError::NoActiveAddressSpace);
    }

    // SAFETY: UserImage owns the boxed mm for the complete bound session. B3
    // disables local interrupts and neither migrates nor preempts its owner.
    let mm = unsafe { &*pointer };
    f(mm)
}

fn validate_range(
    state: &UserMmState,
    address: usize,
    length: usize,
    access: FaultAccess,
) -> Result<(), UserMmRuntimeError> {
    let end = address
        .checked_add(length)
        .ok_or(UserMmRuntimeError::AddressOverflow)?;
    if end <= address {
        return Err(UserMmRuntimeError::InvalidRange);
    }

    let mut cursor = address;
    while cursor < end {
        let virtual_address = VirtAddr::new(cursor);
        let area = state
            .core
            .layout()
            .find_area(virtual_address)
            .ok_or(UserMmRuntimeError::PermissionDenied)?;
        if !access_allowed(area.flags(), access) {
            return Err(UserMmRuntimeError::PermissionDenied);
        }
        let next_page = (cursor | (PAGE_SIZE - 1))
            .checked_add(1)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        cursor = min(next_page, end);
    }
    Ok(())
}

fn validate_mapped_range(
    page_table: &RuntimePageTable,
    address: usize,
    length: usize,
) -> Result<(), UserMmRuntimeError> {
    let end = address
        .checked_add(length)
        .ok_or(UserMmRuntimeError::AddressOverflow)?;
    let mut cursor = address;
    while cursor < end {
        if page_table.translate(VirtAddr::new(cursor))?.is_none() {
            return Err(UserMmRuntimeError::NotMapped);
        }
        let next_page = (cursor | (PAGE_SIZE - 1))
            .checked_add(1)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        cursor = min(next_page, end);
    }
    Ok(())
}

fn reserve_asid_for_mm() -> Result<AsidToken, UserMmRuntimeError> {
    let allocation = {
        let mut slot = ASID_ALLOCATOR.lock();
        if ASID_ROLLOVER_IN_PROGRESS.load(Ordering::Acquire) {
            return Err(UserMmRuntimeError::AsidRolloverInProgress);
        }
        ensure_asid_allocator(&mut *slot)?;
        let allocator = slot.as_mut().expect("ASID allocator was just initialized");
        let will_roll = allocator.next_allocation_rolls_generation();
        if will_roll && LIVE_MMS.load(Ordering::Acquire) != 0 {
            return Err(UserMmRuntimeError::AsidRolloverWithLiveMms);
        }
        if will_roll {
            assert_eq!(
                ASID_ROLLOVER_IN_PROGRESS.compare_exchange(
                    false,
                    true,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ),
                Ok(false),
                "ASID rollover gate changed while holding the allocator lock",
            );
        }
        let allocation = match allocator.allocate() {
            Ok(allocation) => allocation,
            Err(error) => {
                if will_roll {
                    assert!(
                        ASID_ROLLOVER_IN_PROGRESS.swap(false, Ordering::AcqRel),
                        "ASID rollover gate disappeared after allocation failure",
                    );
                }
                return Err(error.into());
            }
        };
        if !allocation.generation_rolled() {
            LIVE_MMS.fetch_add(1, Ordering::AcqRel);
        }
        allocation
    };

    if allocation.generation_rolled() {
        // Do not hold the Vm-ranked allocator lock while entering the lower
        // CrossCpu-ranked shootdown serializer. The atomic gate prevents any
        // new-generation token from becoming visible before the global flush.
        crate::tlb::shootdown_kernel_all();
        LIVE_MMS.fetch_add(1, Ordering::AcqRel);
        assert!(
            ASID_ROLLOVER_IN_PROGRESS.swap(false, Ordering::AcqRel),
            "ASID rollover gate was cleared before publication",
        );
    }
    Ok(allocation.token())
}

fn release_mm_reservation() {
    let old = LIVE_MMS.fetch_sub(1, Ordering::AcqRel);
    assert_ne!(old, 0, "M8-B3 MM reservation counter underflow");
}

fn ensure_asid_allocator(slot: &mut Option<AsidAllocator>) -> Result<(), UserMmRuntimeError> {
    if slot.is_none() {
        *slot = Some(AsidAllocator::new(
            crate::arch::memory::paging::maximum_address_space_id(),
        )?);
    }
    Ok(())
}

fn access_allowed(flags: VmAreaFlags, access: FaultAccess) -> bool {
    match access {
        FaultAccess::Read => flags.is_readable(),
        FaultAccess::Write => flags.is_writable(),
        FaultAccess::Execute => flags.is_executable(),
    }
}
