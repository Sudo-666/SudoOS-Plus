use alloc::{boxed::Box, vec::Vec};
use core::cmp::min;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use myos_mm::{
    AddressSpaceError, AsidAllocator, AsidAllocatorError, AsidToken, FaultAccess, FaultSource,
    PAGE_SIZE, PageAllocation, PageFault, PerMmTlbRequest, PhysAddr, PhysFrame, TlbFlush, TlbScope,
    UserAddressSpace, UserFaultPlan, UserMmError, VirtAddr, VirtPage, VirtRange, VmArea,
    VmAreaError, VmAreaFlags,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::runtime_page_table::{RuntimePageTable, RuntimePageTableError};

// Native toolchains keep many shared objects, metadata files, thread stacks,
// and guard mappings live at once.  A clean rustc invocation exceeds the old
// 96-entry contest baseline before it can allocate its signal alt stack.
const VMA_CAPACITY: usize = 256;

static ASID_ALLOCATOR: IrqSpinLock<Option<AsidAllocator>> =
    IrqSpinLock::new_with_class(None, LockClass::new("user_asid_allocator", LockRank::Vm, 1));
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

#[derive(Clone, Copy)]
struct MappedPageSource {
    page: VirtPage,
    physical: PhysAddr,
}

struct UserMmState {
    core: Box<UserAddressSpace<VMA_CAPACITY>>,
    page_table: Option<RuntimePageTable>,
    pages: Vec<MappedPage>,
}

/// Page-table and backing allocations detached under the MM lock.
///
/// The caller must complete the optional TLB request before releasing either
/// allocation vector. Keeping the request and retired storage in one value
/// makes the TLB-before-free contract explicit at every call site.
struct RetirementBatch {
    request: Option<PerMmTlbRequest>,
    backings: Vec<PageAllocation>,
    page_tables: Vec<PageAllocation>,
}

impl RetirementBatch {
    fn empty() -> Self {
        Self {
            request: None,
            backings: Vec::new(),
            page_tables: Vec::new(),
        }
    }
}

pub struct UserMm {
    state: IrqSpinLock<UserMmState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultRecovery {
    Anonymous,
    StackGrowth,
    Spurious,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultFailure {
    CopyOnWriteUnsupported,
    ProtectionViolation,
    SegmentationViolation,
    KernelBug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultResolution {
    Recovered(UserFaultRecovery),
    Fatal(UserFaultFailure),
}

impl UserMm {
    pub fn vma_usage(&self) -> (usize, usize) {
        let state = self.state.lock();
        (state.core.layout().area_count(), VMA_CAPACITY)
    }

    pub fn new(areas: &[VmArea]) -> Result<Self, UserMmRuntimeError> {
        let asid = reserve_asid_for_mm()?;
        let result: Result<Self, UserMmRuntimeError> = (|| {
            let mut core = Box::new(UserAddressSpace::new(
                crate::arch::memory::layout::USER_RANGE,
                asid,
            ));

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

    pub fn fork_clone_eager(&self) -> Result<alloc::boxed::Box<Self>, UserMmRuntimeError> {
        let (areas, program_break, mapped_pages) = {
            let state = self.state.lock();
            let layout = state.core.layout();
            let area_count = layout.area_count();
            let page_count = state.pages.len();
            crate::println!(
                "fork-clone: areas={} pages={} vma_cap={}",
                area_count, page_count, VMA_CAPACITY,
            );
            let mut areas = Vec::new();
            areas
                .try_reserve(area_count)
                .map_err(|_| {
                    crate::println!("fork-clone: FAIL at areas.try_reserve");
                    UserMmRuntimeError::MetadataOutOfMemory
                })?;
            for index in 0..layout.area_count() {
                areas.push(
                    layout
                        .area_at(index)
                        .expect("area index below count was empty"),
                );
            }

            let page_table = state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let mut mapped_pages = Vec::new();
            mapped_pages
                .try_reserve(page_count)
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            for mapping in &state.pages {
                let physical = page_table
                    .translate(mapping.page.start_address())?
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                mapped_pages.push(MappedPageSource {
                    page: mapping.page,
                    physical,
                });
            }
            (areas, layout.program_break(), mapped_pages)
        };

        let child = alloc::boxed::Box::new(match Self::new(&areas) {
            Ok(mm) => mm,
            Err(e) => {
                crate::println!("fork-clone: FAIL at UserMm::new: {:?}", e);
                return Err(e);
            }
        });
        if let Some(program_break) = program_break {
            child.configure_program_break(program_break.start(), program_break.limit())?;
            child.set_program_break(program_break.current())?;
        }

        // COW fork for LA only: RV's ld-linux repeatedly mmap/mprotects the
        // same page, causing infinite COW-break loops.  LA needs COW to avoid
        // fork ENOMEM from eager-copying 963 pages.
        #[cfg(target_arch = "loongarch64")]
        {
        let cow_count = mapped_pages.len();
        let cow_ops: Vec<_> = {
            let child_state = child.state.lock();
            mapped_pages.iter().map(|source| {
                let area = child_state.core.layout().find_area(source.page.start_address())
                    .ok_or(UserMmRuntimeError::PermissionDenied)?;
                let orig_opts = area.mapping_options();
                let ro_opts = myos_mm::MappingOptions::new(
                    myos_mm::PagePermissions::read_only()
                ).with_user(orig_opts.is_user())
                 .with_memory_type(orig_opts.memory_type());
                let frame = myos_mm::PhysFrame::from_start_address(source.physical)
                    .ok_or(UserMmRuntimeError::InvalidRange)?;
                Ok((source.page, frame, ro_opts))
            }).collect::<Result<Vec<(VirtPage, myos_mm::PhysFrame, myos_mm::MappingOptions)>, UserMmRuntimeError>>()?
        };
        // Now take mutable page-table borrows for parent and child.
        {
            let mut parent_state = self.state.lock();
            let parent_pt = parent_state
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let mut child_state = child.state.lock();
            let child_pt = child_state
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?;

            for (i, (page, frame, ro_opts)) in cow_ops.iter().enumerate() {
                if let Err(e) = child_pt.map_page(*page, *frame, *ro_opts) {
                    crate::println!("fork-clone: FAIL COW map child page {}/{}: {:?}", i, cow_count, e);
                    return Err(UserMmRuntimeError::PageTable(e.into()));
                }
                if let Err(e) = parent_pt.protect_page(*page, *ro_opts) {
                    crate::println!("fork-clone: FAIL COW protect parent page {}/{}: {:?}", i, cow_count, e);
                    return Err(UserMmRuntimeError::PageTable(e.into()));
                }
            }
        }
        crate::println!("fork-clone: COW shared {} pages", cow_count);
        } // end #[cfg(target_arch = "loongarch64")]

        #[cfg(not(target_arch = "loongarch64"))]
        {
            for source in &mapped_pages {
                let destination = child.populate_page(source.page.start_address())?;
                let old_ptr = crate::arch::memory::phys_access::ram_ptr::<u8>(source.physical)
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                let new_ptr = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(destination)
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                unsafe { core::ptr::copy_nonoverlapping(old_ptr, new_ptr, PAGE_SIZE); }
            }
            crate::println!("fork-clone: eager copied {} pages", mapped_pages.len());
        }

        Ok(child)
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

        {
            let state = self.state.lock();
            validate_range(&state, address, input.len(), FaultAccess::Write)?;
        }

        // Linux uaccess faults in a valid demand-mapped destination. Do the
        // same before taking the state lock used for the physical copy;
        // otherwise untouched .bss buffers incorrectly produce EFAULT.
        let end = address
            .checked_add(input.len())
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let mut page = address & !(PAGE_SIZE - 1);
        while page < end {
            self.populate_page(VirtAddr::new(page))?;
            page = page
                .checked_add(PAGE_SIZE)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
        }

        let state = self.state.lock();
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

    pub fn configure_program_break(
        &self,
        start: VirtAddr,
        limit: VirtAddr,
    ) -> Result<(), UserMmRuntimeError> {
        let mut state = self.state.lock();
        state
            .core
            .layout_mut()
            .configure_program_break(start, limit)
            .map_err(UserMmError::from)?;
        Ok(())
    }

    pub fn program_break(&self) -> Result<VirtAddr, UserMmRuntimeError> {
        let state = self.state.lock();
        state
            .core
            .layout()
            .program_break()
            .map(|program_break| program_break.current())
            .ok_or(UserMmRuntimeError::InvalidRange)
    }

    pub fn set_program_break(&self, new_break: VirtAddr) -> Result<VirtAddr, UserMmRuntimeError> {
        let (current, retirement) = {
            let mut state = self.state.lock();
            let old_layout = state.core.layout().clone();
            let old = old_layout
                .program_break()
                .ok_or(UserMmRuntimeError::InvalidRange)?;
            let current = state
                .core
                .layout_mut()
                .set_program_break_and_sync_heap(new_break)
                .map_err(UserMmError::from)?;
            let old_end = old
                .current()
                .align_up(PAGE_SIZE)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let new_end = current
                .align_up(PAGE_SIZE)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            if new_end < old_end {
                let range =
                    VirtRange::new(new_end, old_end).ok_or(UserMmRuntimeError::InvalidRange)?;
                match retire_range_locked(&mut state, range) {
                    Ok(retirement) => (current, retirement),
                    Err(error) => {
                        *state.core.layout_mut() = old_layout;
                        return Err(error);
                    }
                }
            } else {
                (current, RetirementBatch::empty())
            }
        };
        finish_retirement(retirement)?;
        Ok(current)
    }

    pub fn map_anonymous(
        &self,
        search: VirtRange,
        size: usize,
        flags: VmAreaFlags,
    ) -> Result<VirtAddr, UserMmRuntimeError> {
        let mut state = self.state.lock();
        let area = state
            .core
            .layout_mut()
            .map_anonymous(search, size, PAGE_SIZE, flags)
            .map_err(UserMmError::from)?;
        Ok(area.range().start())
    }

    pub fn map_anonymous_exact(
        &self,
        range: VirtRange,
        flags: VmAreaFlags,
    ) -> Result<VirtAddr, UserMmRuntimeError> {
        let mut state = self.state.lock();
        state
            .core
            .map_area(VmArea::new(range, flags, myos_mm::VmAreaKind::Anonymous))?;
        Ok(range.start())
    }

    pub fn unmap_range(&self, range: VirtRange) -> Result<(), UserMmRuntimeError> {
        let retirement = {
            let mut state = self.state.lock();
            let old_layout = state.core.layout().clone();
            state
                .core
                .layout_mut()
                .unmap_range(range)
                .map_err(UserMmError::from)?;
            match retire_range_locked(&mut state, range) {
                Ok(retirement) => retirement,
                Err(error) => {
                    *state.core.layout_mut() = old_layout;
                    return Err(error);
                }
            }
        };
        finish_retirement(retirement)
    }

    pub fn protect_range(
        &self,
        range: VirtRange,
        access: VmAreaFlags,
    ) -> Result<(), UserMmRuntimeError> {
        let request = {
            let mut state = self.state.lock();
            let old_layout = state.core.layout().clone();
            let mapped_count = state
                .pages
                .iter()
                .filter(|mapping| range.contains(mapping.page.start_address()))
                .count();
            let mut changed_pages = Vec::new();
            changed_pages
                .try_reserve(mapped_count)
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            for mapping in &state.pages {
                if !range.contains(mapping.page.start_address()) {
                    continue;
                }
                let old_area = old_layout
                    .find_area(mapping.page.start_address())
                    .expect("mapped user page has no old VMA");
                changed_pages.push((mapping.page, old_area, mapping.backing.start()));
            }

            let request = if changed_pages.is_empty() {
                None
            } else {
                Some(state.core.plan_tlb_request(TlbFlush::Range {
                    scope: TlbScope::AddressSpace(state.core.asid().id()),
                    range,
                })?)
            };

            // Try the full VMA-splitting protect first.
            let layout_result = state.core.layout_mut().protect_range(range, access);

            // PTE-only fallback: when VMA capacity is exhausted but the
            // range is fully covered by existing VMAs, update only the
            // page-table entries without splitting VMAs.
            // Covers both RELRO (PROT_READ) and text-segment mmap
            // finalization (R-X or R-- from temporary RW).
            let is_downgrade = !access.contains(VmAreaFlags::WRITE);
            let pte_fallback = layout_result.is_err() && is_downgrade && !changed_pages.is_empty();

            if pte_fallback {
                // Don't update the VMA layout — just protect existing PTEs.
                let mut pte_result = Ok(());
                if let Some(page_table) = state.page_table.as_mut() {
                    for (page, old_area, frame) in &changed_pages {
                        // Build a temporary read-only VMA to derive mapping options.
                        let ro_start = page.start_address().get();
                        let ro_end = ro_start
                            .checked_add(PAGE_SIZE)
                            .ok_or(RuntimePageTableError::NotMapped)?;
                        let ro_range = VirtRange::from_bounds(ro_start, ro_end);
                        let ro_area = VmArea::new(
                            ro_range,
                            old_area.flags().with_access(access),
                            old_area.kind(),
                        );
                        if let Err(error) =
                            apply_page_protection(page_table, *page, *frame, ro_area)
                        {
                            pte_result = Err(error);
                            break;
                        }
                    }
                }
                if let Err(e) = pte_result {
                    // Rollback: restore original PTE flags.
                    if let Some(page_table) = state.page_table.as_mut() {
                        for (page, old_area, frame) in &changed_pages {
                            let _ = apply_page_protection(page_table, *page, *frame, *old_area);
                        }
                    }
                    return Err(UserMmRuntimeError::Core(UserMmError::AddressSpace(
                        AddressSpaceError::Area(VmAreaError::CapacityExceeded),
                    )));
                }
                // PTE-only fallback succeeded — skip VMA layout update.
                request
            } else {
                // The first call already committed the VMA split on success.
                // Calling it a second time can split an already-updated layout
                // and makes rollback operate on a different topology.
                if let Err(error) = layout_result {
                    *state.core.layout_mut() = old_layout;
                    return Err(UserMmError::from(error).into());
                }

                let mut updated_pages = Vec::new();
                updated_pages
                    .try_reserve(changed_pages.len())
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                let result: Result<(), RuntimePageTableError> = (|| {
                    for (page, _, frame) in &changed_pages {
                        let area = state
                            .core
                            .layout()
                            .find_area(page.start_address())
                            .expect("mprotect removed a mapped page's VMA");
                        let page_table = state
                            .page_table
                            .as_mut()
                            .ok_or(RuntimePageTableError::NotMapped)?;
                        apply_page_protection(page_table, *page, *frame, area)?;
                        updated_pages.push(*page);
                    }
                    Ok(())
                })();

                if let Err(error) = result {
                    let page_table = state
                        .page_table
                        .as_mut()
                        .expect("mprotect rollback lost the user page table");
                    for page in &updated_pages {
                        if let Some((_, old_area, frame)) = changed_pages
                            .iter()
                            .find(|(changed_page, _, _)| changed_page == page)
                        {
                            let _ = apply_page_protection(page_table, *page, *frame, *old_area);
                        }
                    }
                    *state.core.layout_mut() = old_layout;
                    crate::println!(
                        "sudoos-diag: mprotect PTE update failed range=[{:#x},{:#x}) updated={} error={:?}",
                        range.start().get(),
                        range.end().get(),
                        updated_pages.len(),
                        error,
                    );
                    return Err(error.into());
                }
                request
            }
        };
        if let Some(request) = request {
            shootdown_user_request(request);
        }
        Ok(())
    }

    pub fn resolve_user_fault(
        &self,
        address: VirtAddr,
        access: FaultAccess,
        user_sp: VirtAddr,
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        let (resolution, request) = {
            let mut state = self.state.lock();
            let present = state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .translate(address)?
                .is_some();
            let fault = PageFault::new(address, access, FaultSource::User, present);
            match state.core.plan_user_fault(fault, user_sp)? {
                UserFaultPlan::MapAnonymous { area, page } => {
                    let request = state.core.plan_post_install_tlb(page)?;
                    map_zero_page_locked(&mut state, area, page)?;
                    (
                        UserFaultResolution::Recovered(UserFaultRecovery::Anonymous),
                        Some(request),
                    )
                }
                UserFaultPlan::GrowStack { growth } => {
                    let request = state.core.plan_post_install_tlb(growth.fault_page())?;
                    state.core.commit_stack_growth(growth)?;
                    if let Err(error) =
                        map_zero_page_locked(&mut state, growth.new_area(), growth.fault_page())
                    {
                        let removed = state
                            .core
                            .unmap_exact(growth.new_area().range())
                            .expect("stack-growth rollback lost the expanded VMA");
                        assert_eq!(removed, growth.new_area());
                        state
                            .core
                            .map_area(growth.old_area())
                            .expect("stack-growth rollback could not restore the old VMA");
                        return Err(error);
                    }
                    (
                        UserFaultResolution::Recovered(UserFaultRecovery::StackGrowth),
                        Some(request),
                    )
                }
                UserFaultPlan::Spurious { .. } => {
                    let request = state.core.plan_post_install_tlb(address)?;
                    (
                        UserFaultResolution::Recovered(UserFaultRecovery::Spurious),
                        Some(request),
                    )
                }
                UserFaultPlan::CopyOnWriteUnsupported { area } => {
                    // G7 COW break: copy the shared read-only page to a new
                    // writable frame, preserving content from the old frame.
                    let fault_vpage = VirtPage::from_start_address(
                        fault.address().align_down(PAGE_SIZE)
                            .ok_or(UserMmRuntimeError::AddressOverflow)?
                    ).ok_or(UserMmRuntimeError::InvalidRange)?;
                    let old_physical = state.page_table.as_ref()
                        .ok_or(UserMmRuntimeError::NotMapped)?
                        .translate(fault_vpage.start_address())?
                        .ok_or(UserMmRuntimeError::NotMapped)?;
                    let new_backing = crate::page_alloc::allocate(
                        0, crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
                    ).map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                    let new_frame = new_backing.start();
                    // Copy the old page contents into the new frame.
                    let old_ptr = crate::arch::memory::phys_access::ram_ptr::<u8>(old_physical)
                        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                    let new_ptr = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(
                        new_frame.start_address()
                    ).map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                    unsafe { core::ptr::copy_nonoverlapping(old_ptr, new_ptr, PAGE_SIZE); }
                    // Replace the PTE with the new writable frame.
                    let orig_opts = area.mapping_options();
                    let replace_result = state.page_table.as_mut()
                        .ok_or(UserMmRuntimeError::NotMapped)?
                        .replace_page(fault_vpage, new_frame, orig_opts);
                    if let Err(e) = replace_result {
                        crate::page_alloc::free(new_backing).ok();
                        return Err(UserMmRuntimeError::PageTable(e.into()));
                    }
                    // Track the new backing so it's freed on mm teardown.
                    state.pages.push(MappedPage { page: fault_vpage, backing: new_backing });
                    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
                    let request = state.core.plan_post_install_tlb(fault_vpage.start_address())?;
                    (
                        UserFaultResolution::Recovered(UserFaultRecovery::Anonymous),
                        Some(request),
                    )
                },
                UserFaultPlan::ProtectionViolation { .. } => (
                    UserFaultResolution::Fatal(UserFaultFailure::ProtectionViolation),
                    None,
                ),
                UserFaultPlan::SegmentationViolation => (
                    UserFaultResolution::Fatal(UserFaultFailure::SegmentationViolation),
                    None,
                ),
                UserFaultPlan::KernelBug => (
                    UserFaultResolution::Fatal(UserFaultFailure::KernelBug),
                    None,
                ),
            }
        };

        if let Some(request) = request {
            crate::tlb::shootdown_user_local(request);
        }
        Ok(resolution)
    }

    /// Installs the private root, synchronizes the local ASID, and publishes
    /// this CPU only after both hardware operations are complete.
    pub fn activate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();

        // Keep the generation stable across root installation and active-mask
        // publication. A future lazy-renewal path may relax this fail-closed gate.
        let mut allocator = ASID_ALLOCATOR.lock();
        ensure_asid_allocator(&mut allocator)?;
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

            // SAFETY: Process owns the root and shared kernel tables, while the
            // scheduler's loaded_mm Arc pins this UserMm across installation and
            // the complete interval in which this CPU can execute the user task.
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
        assert!(active.count() >= 1, "M8-B3 published an unexpected CPU mask");
        let current_is_active = active.contains(cpu).map_err(UserMmError::from)?;
        assert!(
            current_is_active,
            "M8-B3 did not publish the current CPU in active_cpus",
        );
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<(), UserMmRuntimeError> {
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

        let mut already_unmapped = 0_usize;
        while let Some(mapping) = state.pages.pop() {
            let page_table = state
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            match page_table.translate(mapping.page.start_address())? {
                Some(physical) => {
                    assert_eq!(
                        physical,
                        mapping.backing.start().start_address(),
                        "M8-B3 user leaf returned a different physical address",
                    );
                    let frame = page_table.unmap_page(mapping.page)?;
                    assert_eq!(
                        frame,
                        mapping.backing.start(),
                        "M8-B3 user leaf returned a different physical frame",
                    );
                }
                None => {
                    // MAP_FIXED/munmap may retire the leaf before the final
                    // owner reaches process teardown. The backing remains
                    // uniquely owned by this record and still must be freed.
                    already_unmapped += 1;
                }
            }
            page_table.reclaim_empty_tables(mapping.page, &mut retired)?;
            let backing_start = mapping.backing.start().start_address().get();
            let backing_order = mapping.backing.order();
            if let Err(error) = crate::page_alloc::free(mapping.backing) {
                crate::println!(
                    "user-mm: backing free failed page={:#x} phys={:#x} order={} error={:?}",
                    mapping.page.start_address().get(),
                    backing_start,
                    backing_order,
                    error,
                );
                return Err(error.into());
            }
            LIVE_BACKINGS.fetch_sub(1, Ordering::AcqRel);
        }

        if already_unmapped != 0 {
            crate::println!(
                "user-mm: reclaimed {} backing pages whose leaves were already unmapped",
                already_unmapped,
            );
        }

        state
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .retire_all_private_tables(&mut retired)?;

        for table in retired.drain(..) {
            let start = table.start().start_address().get();
            let order = table.order();
            if let Err(error) = crate::page_alloc::free(table) {
                crate::println!(
                    "user-mm: table free failed phys={:#x} order={} error={:?}",
                    start,
                    order,
                    error,
                );
                return Err(error.into());
            }
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
        let needs_teardown = {
            let state = self.state.lock();
            state.page_table.is_some()
        };
        if needs_teardown {
            if let Err(error) = self.destroy() {
                panic!("M8-B3 UserMm teardown failed during drop: {error:?}");
            }
        }
        let state = self.state.lock();
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

pub fn assert_no_leaks() {
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

fn map_zero_page_locked(
    state: &mut UserMmState,
    area: VmArea,
    page_address: VirtAddr,
) -> Result<(), UserMmRuntimeError> {
    let page_address = page_address
        .align_down(PAGE_SIZE)
        .ok_or(UserMmRuntimeError::AddressOverflow)?;
    let page =
        VirtPage::from_start_address(page_address).ok_or(UserMmRuntimeError::InvalidRange)?;
    if state
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?
        .translate(page_address)?
        .is_some()
    {
        return Ok(());
    }

    state
        .pages
        .try_reserve(1)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    let backing =
        crate::page_alloc::allocate(0, crate::page_alloc::PageAllocationOptions::kernel_zeroed())?;
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
    Ok(())
}

fn retire_range_locked(
    state: &mut UserMmState,
    range: VirtRange,
) -> Result<RetirementBatch, UserMmRuntimeError> {
    let count = state
        .pages
        .iter()
        .filter(|mapping| range.contains(mapping.page.start_address()))
        .count();
    if count == 0 {
        return Ok(RetirementBatch::empty());
    }

    let page_table = state
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?;
    for mapping in &state.pages {
        if !range.contains(mapping.page.start_address()) {
            continue;
        }
        let physical = page_table
            .translate(mapping.page.start_address())?
            .ok_or(UserMmRuntimeError::NotMapped)?;
        assert_eq!(
            physical,
            mapping.backing.start().start_address(),
            "user retirement preflight found a mismatched backing frame",
        );
    }

    let table_capacity = page_table.allocated_runtime_tables();
    let mut backings = Vec::new();
    backings
        .try_reserve(count)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    let mut tables = Vec::new();
    tables
        .try_reserve(table_capacity)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    let request = state.core.plan_tlb_request(TlbFlush::Range {
        scope: TlbScope::AddressSpace(state.core.asid().id()),
        range,
    })?;

    let mut index = 0;
    while index < state.pages.len() {
        if !range.contains(state.pages[index].page.start_address()) {
            index += 1;
            continue;
        }
        let mapping = state.pages.swap_remove(index);
        let page_table = state
            .page_table
            .as_mut()
            .expect("retirement preflight lost the user page table");
        let frame = page_table
            .unmap_page(mapping.page)
            .expect("retirement preflight accepted a missing user leaf");
        assert_eq!(
            frame,
            mapping.backing.start(),
            "user unmap returned a different backing frame",
        );
        page_table
            .reclaim_empty_tables(mapping.page, &mut tables)
            .expect("user page-table reclamation violated the reviewed topology");
        backings.push(mapping.backing);
    }

    Ok(RetirementBatch {
        request: Some(request),
        backings,
        page_tables: tables,
    })
}

fn finish_retirement(retirement: RetirementBatch) -> Result<(), UserMmRuntimeError> {
    if let Some(request) = retirement.request {
        shootdown_user_request(request);
    }
    for backing in retirement.backings {
        crate::page_alloc::free(backing)?;
        LIVE_BACKINGS.fetch_sub(1, Ordering::AcqRel);
    }
    for table in retirement.page_tables {
        crate::page_alloc::free(table)?;
    }
    Ok(())
}

fn apply_page_protection(
    page_table: &mut RuntimePageTable,
    page: VirtPage,
    frame: PhysFrame,
    area: VmArea,
) -> Result<(), RuntimePageTableError> {
    if area.flags().access_only() == VmAreaFlags::empty() {
        return match page_table.unmap_page(page) {
            Ok(mapped) => {
                debug_assert_eq!(mapped, frame);
                Ok(())
            }
            Err(RuntimePageTableError::NotMapped) => Ok(()),
            Err(error) => Err(error),
        };
    }

    let options = area.mapping_options();
    match page_table.protect_page(page, options) {
        Ok(()) => Ok(()),
        Err(RuntimePageTableError::NotMapped) => page_table.map_page(page, frame, options),
        Err(error) => Err(error),
    }
}

fn shootdown_user_request(request: myos_mm::PerMmTlbRequest) {
    if crate::arch::interrupt::are_disabled() || !crate::task::scheduler_is_initialized() {
        crate::tlb::shootdown_user_local(request);
    } else {
        crate::tlb::shootdown_user(request);
    }
}

fn copy_physical_page(source: PhysAddr, destination: PhysAddr) -> Result<(), UserMmRuntimeError> {
    let source = crate::arch::memory::phys_access::ram_ptr::<u8>(source)
        .map_err(|_| UserMmRuntimeError::NotMapped)?;
    let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(destination)
        .map_err(|_| UserMmRuntimeError::NotMapped)?;

    // SAFETY: both pointers name RAM pages owned by live user address spaces,
    // and the copy is exactly one page starting at page-aligned translations.
    unsafe {
        core::ptr::copy_nonoverlapping(source, destination, PAGE_SIZE);
    }
    Ok(())
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
        ensure_asid_allocator(&mut slot)?;
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
