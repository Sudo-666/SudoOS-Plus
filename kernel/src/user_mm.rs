use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
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

// Native toolchains keep many shared objects, allocator arenas, thread stacks,
// and guard mappings live at once. LoongArch rustc crosses 1024 VMAs while
// loading librustc_driver and its jemalloc arenas. VmAreaSet stores only live
// entries on the heap, so this Linux-like ceiling does not preallocate 64K
// records and does not consume a kernel task's 64-KiB stack.
const VMA_CAPACITY: usize = 65_536;

static ASID_ALLOCATOR: IrqSpinLock<Option<AsidAllocator>> =
    IrqSpinLock::new_with_class(None, LockClass::new("user_asid_allocator", LockRank::Vm, 1));
static ASID_ROLLOVER_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LIVE_MMS: AtomicUsize = AtomicUsize::new(0);
static LIVE_ROOTS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BACKINGS: AtomicUsize = AtomicUsize::new(0);
const FILE_PAGE_CACHE_CAPACITY: usize = 128 * 1024;
static FILE_PAGE_CACHE: IrqSpinLock<BTreeMap<(u64, u64, u64), PageAllocation>> =
    IrqSpinLock::new_with_class(
        BTreeMap::new(),
        LockClass::new("file_page_cache", LockRank::Vm, 3),
    );

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
    backing: PageBacking,
}

enum PageBacking {
    Owned(PageAllocation),
    Shared(PhysFrame),
}

impl PageBacking {
    fn frame(&self) -> PhysFrame {
        match self {
            Self::Owned(allocation) => allocation.start(),
            Self::Shared(frame) => *frame,
        }
    }

    fn physical(&self) -> PhysAddr {
        self.frame().start_address()
    }
}

#[derive(Clone, Copy)]
struct MappedPageSource {
    page: VirtPage,
    physical: PhysAddr,
    shared: bool,
}

#[derive(Clone)]
struct FileBackedMapping {
    range: VirtRange,
    file_offset: u64,
    file_length: usize,
    generation: u64,
    device: u64,
    inode: u64,
    shared_cache: bool,
    file: myos_vfs::ArcFile,
}

pub(crate) struct FileFaultRequest {
    pub(crate) file: myos_vfs::ArcFile,
    pub(crate) file_offset: u64,
    pub(crate) read_length: usize,
    pub(crate) page: VirtAddr,
    cache_key: (u64, u64, u64),
    shared_cache: bool,
    generation: u64,
}

struct UserMmState {
    core: Box<UserAddressSpace<VMA_CAPACITY>>,
    page_table: Option<RuntimePageTable>,
    pages: Vec<MappedPage>,
    file_mappings: Vec<FileBackedMapping>,
    next_file_generation: u64,
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

    /// Return the single VMA that completely contains `range`.
    ///
    /// mremap uses this snapshot to preserve access flags while resizing an
    /// anonymous mapping.  Requiring one containing VMA avoids silently
    /// merging mappings with different protections or backing kinds.
    pub fn area_containing(&self, range: VirtRange) -> Option<VmArea> {
        let state = self.state.lock();
        let area = state.core.layout().find_area(range.start())?;
        area.range().contains_range(range).then_some(area)
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
                        file_mappings: Vec::new(),
                        next_file_generation: 1,
                    },
                    LockClass::new("user_mm", LockRank::Vm, 2),
                ),
            })
        })();

        if result.is_err() {
            release_mm_reservation(asid);
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

    // FORK_CLONE_PRESERVE_PROT_NONE_V1
    pub fn fork_clone_eager(&self) -> Result<alloc::boxed::Box<Self>, UserMmRuntimeError> {
        let (areas, program_break, mapped_pages, file_mappings, next_file_generation) = {
            let state = self.state.lock();
            let layout = state.core.layout();

            let mut areas = Vec::new();
            areas
                .try_reserve(layout.area_count())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
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
                .try_reserve(state.pages.len())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

            for mapping in &state.pages {
                let area = layout
                    .find_area(mapping.page.start_address())
                    .ok_or(UserMmRuntimeError::PermissionDenied)?;
                let physical = mapping.backing.physical();

                match page_table.translate(mapping.page.start_address())? {
                    Some(translated) => {
                        if translated != physical {
                            return Err(UserMmRuntimeError::NotMapped);
                        }
                    }
                    None => {
                        if area.flags().access_only() != VmAreaFlags::empty() {
                            return Err(UserMmRuntimeError::NotMapped);
                        }
                    }
                }

                mapped_pages.push(MappedPageSource {
                    page: mapping.page,
                    physical,
                    shared: matches!(&mapping.backing, PageBacking::Shared(_)),
                });
            }

            let mut file_mappings = Vec::new();
            file_mappings
                .try_reserve(state.file_mappings.len())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            file_mappings.extend(state.file_mappings.iter().cloned());

            (
                areas,
                layout.program_break(),
                mapped_pages,
                file_mappings,
                state.next_file_generation,
            )
        };

        let child = alloc::boxed::Box::new(Self::new(&areas)?);
        if let Some(program_break) = program_break {
            child.configure_program_break(program_break.start(), program_break.limit())?;
            child.set_program_break(program_break.current())?;
        }
        {
            let mut state = child.state.lock();
            state.file_mappings = file_mappings;
            state.next_file_generation = next_file_generation;
        }

        for source in mapped_pages {
            let mut state = child.state.lock();
            let area = state
                .core
                .layout()
                .find_area(source.page.start_address())
                .ok_or(UserMmRuntimeError::PermissionDenied)?;

            state
                .pages
                .try_reserve(1)
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

            let backing = if source.shared {
                PageBacking::Shared(
                    PhysFrame::from_start_address(source.physical)
                        .ok_or(UserMmRuntimeError::AddressOverflow)?,
                )
            } else {
                let allocation = crate::page_alloc::allocate(
                    0,
                    crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
                )?;
                let destination = allocation.start().start_address();
                if let Err(error) = copy_physical_page(source.physical, destination) {
                    crate::page_alloc::free(allocation)?;
                    return Err(error);
                }
                PageBacking::Owned(allocation)
            };

            if area.flags().access_only() != VmAreaFlags::empty() {
                let page_table = state
                    .page_table
                    .as_mut()
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                if let Err(error) =
                    page_table.map_page(source.page, backing.frame(), area.mapping_options())
                {
                    if let PageBacking::Owned(allocation) = backing {
                        crate::page_alloc::free(allocation)?;
                    }
                    return Err(error.into());
                }
            }

            let owned = matches!(&backing, PageBacking::Owned(_));
            state.pages.push(MappedPage {
                page: source.page,
                backing,
            });
            if owned {
                LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
            }
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

        state.pages.push(MappedPage {
            page,
            backing: PageBacking::Owned(backing),
        });
        LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
        Ok(physical)
    }

    /// Populate and initialize a range in one MM-lock acquisition.
    ///
    /// Execve uses this while the new address space is inactive. File-backed
    /// mmap also uses it for each bulk read while the calling thread owns the
    /// syscall. In both cases batching avoids one lock acquisition, VMA lookup,
    /// and page-table setup pass per 4 KiB page.
    pub(crate) fn load_bytes(
        &self,
        address: VirtAddr,
        input: &[u8],
    ) -> Result<(), UserMmRuntimeError> {
        if input.is_empty() {
            return Ok(());
        }
        let end = address
            .get()
            .checked_add(input.len())
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let covered = input
            .len()
            .checked_add(address.get() & (PAGE_SIZE - 1))
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let page_count = covered
            .checked_add(PAGE_SIZE - 1)
            .ok_or(UserMmRuntimeError::AddressOverflow)?
            / PAGE_SIZE;
        let mut state = self.state.lock();
        state
            .pages
            .try_reserve(page_count)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

        let mut copied = 0;
        while copied < input.len() {
            let current = address
                .checked_add(copied)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let area = state
                .core
                .layout()
                .find_area(current)
                .ok_or(UserMmRuntimeError::PermissionDenied)?;
            let physical = map_zero_page_locked(&mut state, area, current)?;
            let in_page = current.get() & (PAGE_SIZE - 1);
            let chunk = min(PAGE_SIZE - in_page, input.len() - copied);
            let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(physical)
                .map_err(|_| UserMmRuntimeError::NotMapped)?;
            // SAFETY: the populated translation names RAM owned by this MM,
            // and the copy is bounded to its current page.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    input.as_ptr().add(copied),
                    destination,
                    chunk,
                );
            }
            copied += chunk;
        }
        debug_assert_eq!(address.get() + copied, end);
        Ok(())
    }

    /// Ensure every page intersecting `range` has a zeroed backing page.
    ///
    /// Fresh allocations are already kernel-zeroed, so unlike a byte-at-a-time
    /// memset this only needs to install missing pages. Existing pages are left
    /// intact, which is required when the zero tail shares its first page with
    /// file data loaded immediately before this call.
    pub(crate) fn populate_zeroed_range(
        &self,
        address: VirtAddr,
        length: usize,
    ) -> Result<(), UserMmRuntimeError> {
        if length == 0 {
            return Ok(());
        }
        let end = address
            .get()
            .checked_add(length)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let first_page = address
            .align_down(PAGE_SIZE)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let page_count = length
            .checked_add(address.get() & (PAGE_SIZE - 1))
            .and_then(|covered| covered.checked_add(PAGE_SIZE - 1))
            .ok_or(UserMmRuntimeError::AddressOverflow)?
            / PAGE_SIZE;

        let mut state = self.state.lock();
        state
            .pages
            .try_reserve(page_count)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

        let mut page = first_page;
        while page.get() < end {
            let area = state
                .core
                .layout()
                .find_area(page)
                .ok_or(UserMmRuntimeError::PermissionDenied)?;
            map_zero_page_locked(&mut state, area, page)?;
            page = page
                .checked_add(PAGE_SIZE)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
        }
        Ok(())
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

        // The overwhelmingly common read(2) destination is an already
        // resident libc/rustc heap buffer. Keep validation and copying under
        // one MM lock in that case. The old path called populate_page() for
        // every page even when present, taking the lock and walking the page
        // table once per page before validating and walking it all over again.
        {
            let state = self.state.lock();
            validate_range(&state, address, input.len(), FaultAccess::Write)?;
            let page_table = state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            match validate_mapped_range(page_table, address, input.len()) {
                Ok(()) => return copy_to_mapped_pages(page_table, address, input),
                Err(UserMmRuntimeError::NotMapped) => {}
                Err(error) => return Err(error),
            }
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
        copy_to_mapped_pages(page_table, address, input)
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

    /// Atomically replace any overlapping VMAs with a single exact anonymous
    /// VMA — the VMA topology change (remove old, insert new) is done under a
    /// single MM lock so no concurrent observer sees the intermediate "hole".
    ///
    /// Old page-table entries and backings are detached inside the lock but
    /// freed after the lock is released (TLB-before-free).
    ///
    /// Returns the exact start address on success.
    pub fn replace_anonymous_exact(
        &self,
        range: VirtRange,
        flags: VmAreaFlags,
    ) -> Result<VirtAddr, UserMmRuntimeError> {
        let retirement = {
            let mut state = self.state.lock();
            let old_layout = state.core.layout().clone();
            let retained_file_mappings =
                file_mappings_without_range(&state.file_mappings, range)?;

            // 1. Remove overlapping VMAs from the topology.
            state
                .core
                .layout_mut()
                .unmap_range(range)
                .map_err(UserMmError::from)?;

            // 2. Insert the new exact VMA before detaching old PTEs/backings so
            //    that a rollback of this step only needs to restore the layout.
            if let Err(e) =
                state
                    .core
                    .map_area(VmArea::new(range, flags, myos_mm::VmAreaKind::Anonymous))
            {
                *state.core.layout_mut() = old_layout;
                return Err(UserMmRuntimeError::Core(e));
            }

            // 3. Detach old PTEs and backings.  Allocations happen before any
            //    PTE/backing modification, so an error here leaves the state
            //    consistent and a layout rollback is sufficient.
            match retire_range_locked(&mut state, range) {
                Ok(retirement) => {
                    state.file_mappings = retained_file_mappings;
                    retirement
                }
                Err(error) => {
                    *state.core.layout_mut() = old_layout;
                    return Err(error);
                }
            }
        };
        finish_retirement(retirement)?;
        Ok(range.start())
    }

    /// Exact anonymous mapping that MUST NOT replace any existing mapping.
    ///
    /// Returns `Core(AddressSpace(Area(Overlap)))` when any part of `range`
    /// overlaps an existing VMA — the caller maps this to `EEXIST`.
    /// The check and the map happen under the same MM lock so there is no
    /// TOCTOU window between the overlap test and the insertion.
    pub fn map_anonymous_noreplace(
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

    pub fn register_file_mapping(
        &self,
        range: VirtRange,
        file_offset: u64,
        file_length: usize,
        device: u64,
        inode: u64,
        shared_cache: bool,
        file: myos_vfs::ArcFile,
    ) -> Result<(), UserMmRuntimeError> {
        let mut state = self.state.lock();
        let area = state
            .core
            .layout()
            .find_area(range.start())
            .ok_or(UserMmRuntimeError::NotMapped)?;
        if !area.range().contains_range(range) {
            return Err(UserMmRuntimeError::InvalidRange);
        }
        state
            .file_mappings
            .try_reserve(1)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        let generation = state.next_file_generation;
        state.next_file_generation = state.next_file_generation.wrapping_add(1).max(1);
        state.file_mappings.push(FileBackedMapping {
            range,
            file_offset,
            file_length,
            generation,
            device,
            inode,
            shared_cache,
            file,
        });
        Ok(())
    }

    pub fn file_fault_request(
        &self,
        address: VirtAddr,
        access: FaultAccess,
    ) -> Result<Option<FileFaultRequest>, UserMmRuntimeError> {
        let state = self.state.lock();
        let Some(mapping) = state
            .file_mappings
            .iter()
            .find(|mapping| mapping.range.contains(address))
        else {
            return Ok(None);
        };
        let area = state
            .core
            .layout()
            .find_area(address)
            .ok_or(UserMmRuntimeError::NotMapped)?;
        let allowed = match access {
            FaultAccess::Read => area.flags().is_readable(),
            FaultAccess::Write => area.flags().is_writable(),
            FaultAccess::Execute => area.flags().is_executable(),
        };
        if !allowed
            || state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .translate(address)?
                .is_some()
        {
            return Ok(None);
        }
        let page = address
            .align_down(PAGE_SIZE)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let delta = page
            .get()
            .checked_sub(mapping.range.start().get())
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let read_length = mapping.file_length.saturating_sub(delta).min(PAGE_SIZE);
        let file_offset = mapping
            .file_offset
            .checked_add(delta as u64)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        Ok(Some(FileFaultRequest {
            file: alloc::sync::Arc::clone(&mapping.file),
            file_offset,
            read_length,
            page,
            cache_key: (mapping.device, mapping.inode, file_offset),
            shared_cache: mapping.shared_cache,
            generation: mapping.generation,
        }))
    }

    pub fn install_file_fault(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        self.install_file_fault_inner(fault, data, true)
    }

    /// Map a full immutable file page already retained by the kernel-wide
    /// cache without issuing another VFS read or copying its contents.
    pub fn install_cached_file_fault(
        &self,
        fault: &FileFaultRequest,
    ) -> Result<Option<UserFaultResolution>, UserMmRuntimeError> {
        static UNUSED_CACHE_HIT_DATA: [u8; PAGE_SIZE] = [0; PAGE_SIZE];

        if !fault.shared_cache
            || fault.read_length != PAGE_SIZE
            || file_page_cache_lookup(fault.cache_key).is_none()
        {
            return Ok(None);
        }
        self.install_file_fault_inner(fault, &UNUSED_CACHE_HIT_DATA, true)
            .map(Some)
    }

    pub fn install_file_prefetch(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        self.install_file_fault_inner(fault, data, false)
    }

    fn install_file_fault_inner(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
        flush_local: bool,
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        enum InstallOutcome {
            Installed(PerMmTlbRequest),
            Spurious,
            Unmapped,
        }

        if data.len() != fault.read_length {
            return Err(UserMmRuntimeError::InvalidRange);
        }
        let cacheable = fault.shared_cache && fault.read_length == PAGE_SIZE;
        let mut backing = if cacheable
            && let Some(frame) = file_page_cache_lookup(fault.cache_key)
        {
            Some(PageBacking::Shared(frame))
        } else {
            let allocation = crate::page_alloc::allocate(
                0,
                crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
            )?;
            if !data.is_empty() {
                let destination = match crate::arch::memory::phys_access::ram_mut_ptr::<u8>(
                    allocation.start().start_address(),
                ) {
                    Ok(destination) => destination,
                    Err(_) => {
                        crate::page_alloc::free(allocation)?;
                        return Err(UserMmRuntimeError::NotMapped);
                    }
                };
                unsafe {
                    core::ptr::copy_nonoverlapping(data.as_ptr(), destination, data.len());
                }
            }
            let (cached, duplicate) = if cacheable {
                file_page_cache_install(fault.cache_key, allocation)
            } else {
                (PageBacking::Owned(allocation), None)
            };
            if let Some(duplicate) = duplicate {
                crate::page_alloc::free(duplicate)?;
            }
            Some(cached)
        };

        let outcome: Result<InstallOutcome, UserMmRuntimeError> = (|| {
            let mut state = self.state.lock();
            let still_mapped = state.file_mappings.iter().any(|mapping| {
                mapping.generation == fault.generation && mapping.range.contains(fault.page)
            });
            if !still_mapped {
                return Ok(InstallOutcome::Unmapped);
            }
            if state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .translate(fault.page)?
                .is_some()
            {
                return Ok(InstallOutcome::Spurious);
            }
            let area = state
                .core
                .layout()
                .find_area(fault.page)
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let page = VirtPage::from_start_address(fault.page)
                .ok_or(UserMmRuntimeError::InvalidRange)?;
            state
                .pages
                .try_reserve(1)
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            let request = state.core.plan_post_install_tlb(fault.page)?;
            if let Err(error) = state
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .map_page(
                    page,
                    backing
                        .as_ref()
                        .expect("file-fault backing disappeared before PTE install")
                        .frame(),
                    area.mapping_options(),
                )
            {
                return Err(error.into());
            }
            let backing = backing
                .take()
                .expect("file-fault backing disappeared after PTE install");
            let owned = matches!(&backing, PageBacking::Owned(_));
            state.pages.push(MappedPage {
                page,
                backing,
            });
            if owned {
                LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
            }
            Ok(InstallOutcome::Installed(request))
        })();

        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                if let Some(unused) = backing.take() {
                    let _ = free_uninstalled_backing(unused);
                }
                return Err(error);
            }
        };
        let request = match outcome {
            InstallOutcome::Installed(request) => request,
            InstallOutcome::Spurious => {
                free_uninstalled_backing(
                    backing.take().expect("spurious file fault lost its backing"),
                )?;
                return Ok(UserFaultResolution::Recovered(UserFaultRecovery::Spurious));
            }
            InstallOutcome::Unmapped => {
                free_uninstalled_backing(
                    backing.take().expect("unmapped file fault lost its backing"),
                )?;
                return Ok(UserFaultResolution::Fatal(
                    UserFaultFailure::SegmentationViolation,
                ));
            }
        };
        if !flush_local {
            return Ok(UserFaultResolution::Recovered(UserFaultRecovery::Anonymous));
        }

        // Installing a previously invalid leaf never revokes access or frees
        // memory. A remote CPU may keep its stale invalid translation and take
        // the same recoverable fault, so only this CPU needs an immediate
        // invalidation. Briefly mask interrupts when called from syscall
        // uaccess; trap-fault callers already arrive with interrupts masked.
        let restore_enabled = !crate::arch::interrupt::are_disabled();
        if restore_enabled {
            crate::arch::interrupt::disable();
        }
        let request = match request.local_only(crate::smp::current_cpu_id().get()) {
            Ok(request) => request,
            Err(error) => {
                if restore_enabled {
                    // SAFETY: restore the entry interrupt state before
                    // propagating a request-shaping error.
                    unsafe { crate::arch::interrupt::enable() };
                }
                return Err(UserMmRuntimeError::from(error));
            }
        };
        crate::tlb::shootdown_user_local(request);
        if restore_enabled {
            // SAFETY: this restores the enabled state observed immediately
            // before the short local TLB critical section.
            unsafe { crate::arch::interrupt::enable() };
        }
        Ok(UserFaultResolution::Recovered(UserFaultRecovery::Anonymous))
    }

    pub fn unmap_range(&self, range: VirtRange) -> Result<(), UserMmRuntimeError> {
        let retirement = {
            let mut state = self.state.lock();
            let old_layout = state.core.layout().clone();
            let retained_file_mappings =
                file_mappings_without_range(&state.file_mappings, range)?;
            state
                .core
                .layout_mut()
                .unmap_range(range)
                .map_err(UserMmError::from)?;
            match retire_range_locked(&mut state, range) {
                Ok(retirement) => {
                    state.file_mappings = retained_file_mappings;
                    retirement
                }
                Err(error) => {
                    *state.core.layout_mut() = old_layout;
                    return Err(error);
                }
            }
        };
        finish_retirement(retirement)
    }

    /// Drop resident pages from adjacent anonymous VMAs without removing them.
    ///
    /// This implements the observable part of Linux MADV_DONTNEED used by
    /// jemalloc: a later access faults the page back in as zero-filled.  ELF
    /// file mappings are eagerly populated by this kernel and cannot yet be
    /// reconstructed on demand, so callers leave non-anonymous VMAs intact.
    pub fn discard_anonymous_range(&self, range: VirtRange) -> Result<(), UserMmRuntimeError> {
        let retirement = {
            let mut state = self.state.lock();
            if state
                .file_mappings
                .iter()
                .any(|mapping| mapping.range.overlaps(range))
            {
                return Ok(());
            }
            let mut cursor = range.start();
            loop {
                let area = state
                    .core
                    .layout()
                    .find_area(cursor)
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                if !matches!(area.kind(), myos_mm::VmAreaKind::Anonymous) {
                    return Ok(());
                }
                let next = min(area.range().end(), range.end());
                if next == range.end() {
                    break;
                }
                cursor = next;
            }
            retire_range_locked(&mut state, range)?
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

            // glibc/rustc frequently repeats mprotect() over ranges that
            // already have the requested access (allocator arenas and thread
            // stack guards are the main cases). Such a call is observably a
            // no-op: avoid cloning/rebuilding the VMA table, rewriting the
            // same PTE bits, and issuing a synchronous cross-CPU shootdown.
            // Walk by VMA rather than by page so the check stays cheap even
            // for large mappings, while gaps still fall through to the normal
            // validation path and return the same error as before.
            let requested_access = access.access_only();
            let mut cursor = range.start();
            let mut already_protected = true;
            loop {
                let Some(area) = state.core.layout().find_area(cursor) else {
                    already_protected = false;
                    break;
                };
                let old_access = area.flags().access_only();
                if old_access != requested_access {
                    already_protected = false;
                }
                let next = core::cmp::min(area.range().end(), range.end());
                if next == range.end() {
                    break;
                }
                cursor = next;
            }
            if already_protected {
                return Ok(());
            }

            let old_layout = state.core.layout().clone();

            // Cached file pages are shared only while immutable. If userspace
            // promotes such a mapping to writable, materialize a private copy
            // before changing either the VMA or PTE permissions.
            if requested_access.is_writable() {
                let mut shared_indices = Vec::new();
                shared_indices
                    .try_reserve(state.pages.len().min(range.size() / PAGE_SIZE + 1))
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                for (index, mapping) in state.pages.iter().enumerate() {
                    if range.contains(mapping.page.start_address())
                        && matches!(&mapping.backing, PageBacking::Shared(_))
                    {
                        shared_indices.push(index);
                    }
                }
                for index in shared_indices {
                    let page = state.pages[index].page;
                    let source = state.pages[index].backing.physical();
                    let allocation = crate::page_alloc::allocate(
                        0,
                        crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
                    )?;
                    if let Err(error) =
                        copy_physical_page(source, allocation.start().start_address())
                    {
                        crate::page_alloc::free(allocation)?;
                        return Err(error);
                    }
                    let present = state
                        .page_table
                        .as_ref()
                        .ok_or(UserMmRuntimeError::NotMapped)?
                        .translate(page.start_address())?
                        .is_some();
                    if present {
                        let area = old_layout
                            .find_area(page.start_address())
                            .ok_or(UserMmRuntimeError::PermissionDenied)?;
                        let replace = state
                            .page_table
                            .as_mut()
                            .ok_or(UserMmRuntimeError::NotMapped)?
                            .replace_page(page, allocation.start(), area.mapping_options());
                        if let Err(error) = replace {
                            crate::page_alloc::free(allocation)?;
                            return Err(error.into());
                        }
                    }
                    state.pages[index].backing = PageBacking::Owned(allocation);
                    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
                }
            }

            let mut changed_pages = Vec::new();
            let range_pages = range
                .size()
                .checked_add(PAGE_SIZE - 1)
                .ok_or(UserMmRuntimeError::AddressOverflow)?
                / PAGE_SIZE;
            changed_pages
                .try_reserve(range_pages)
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

            // mprotect is extremely hot in rustc. Scanning every resident page
            // in the process for each small stack/allocator protection change
            // made the operation O(total address-space pages). Walk only the
            // requested virtual range and query the page table directly.
            let page_table = state
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let mut address = range.start().get();
            while address < range.end().get() {
                let page_address = VirtAddr::new(address);
                let page = VirtPage::from_start_address(page_address)
                    .ok_or(UserMmRuntimeError::InvalidRange)?;
                let old_area = old_layout.find_area(page_address);
                let frame = match page_table.translate(page_address)? {
                    Some(physical) => Some(PhysFrame::from_start_address(physical).ok_or(
                        UserMmRuntimeError::AddressOverflow,
                    )?),
                    None => state
                        .pages
                        .iter()
                        .find(|mapping| mapping.page == page)
                        .map(|mapping| mapping.backing.frame()),
                };
                if let Some(frame) = frame {
                    changed_pages.push((
                        page,
                        old_area.expect("mapped user page has no old VMA"),
                        frame,
                    ));
                }
                address = address
                    .checked_add(PAGE_SIZE)
                    .ok_or(UserMmRuntimeError::AddressOverflow)?;
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
                UserFaultPlan::CopyOnWriteUnsupported { .. } => (
                    UserFaultResolution::Fatal(UserFaultFailure::CopyOnWriteUnsupported),
                    None,
                ),
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
            // User faults execute with local interrupts disabled, so they
            // cannot enter the synchronous remote IPI/ACK path. This path only
            // installs a previously invalid leaf or repairs a spurious local
            // translation; no page is unmapped, freed, or permission-revoked.
            //
            // Restrict the post-install request to this CPU. Another CPU using
            // the same mm either observes the new valid PTE directly or faults
            // on its own stale invalid translation and performs the same local
            // recovery. munmap/mprotect/retirement still retain the original
            // full active_cpus mask and use shootdown_user().
            let request = request
                .local_only(crate::smp::current_cpu_id().get())
                .map_err(UserMmRuntimeError::from)?;
            crate::tlb::shootdown_user_local(request);
            // post-install fault recovery is local-only by construction
        }
        Ok(resolution)
    }

    /// Installs the private root, synchronizes the local ASID, and publishes
    /// this CPU only after both hardware operations are complete.
    pub fn activate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();

        // SUDOOS_BUILDSTORM_ROOTFIX_ASID_SCOPE_V1
        // Snapshot generation under the global allocator, then release it
        // before taking user_mm or installing page-table hardware state.
        let current_asid_generation = {
            let mut allocator = ASID_ALLOCATOR.lock();
            ensure_asid_allocator(&mut allocator)?;
            allocator
                .as_ref()
                .expect("ASID allocator was just initialized")
                .generation()
        };

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
        // `active_cpus` is the Linux-like mm_cpumask, not a single-owner
        // marker. A CLONE_VM/pthread process may legally execute this same mm
        // on multiple CPUs at once. Hardware root/ASID checks above and the
        // current-CPU membership check below are the required local invariant.
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
                        mapping.backing.physical(),
                        "M8-B3 user leaf returned a different physical address",
                    );
                    let frame = page_table.unmap_page(mapping.page)?;
                    assert_eq!(
                        frame,
                        mapping.backing.frame(),
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
            if let PageBacking::Owned(allocation) = mapping.backing {
                let backing_start = allocation.start().start_address().get();
                let backing_order = allocation.order();
                if let Err(error) = crate::page_alloc::free(allocation) {
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
        }

        if already_unmapped != 0 && crate::user::oscomp_verbose_user_trace_active() {
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
        let asid = state.core.asid();
        drop(state);
        release_mm_reservation(asid);
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
    address: VirtAddr,
) -> Result<PhysAddr, UserMmRuntimeError> {
    let offset = address.get() & (PAGE_SIZE - 1);
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
    let backing =
        crate::page_alloc::allocate(0, crate::page_alloc::PageAllocationOptions::kernel_zeroed())?;
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

    state.pages.push(MappedPage {
        page,
        backing: PageBacking::Owned(backing),
    });
    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
    Ok(physical)
}

fn file_mappings_without_range(
    mappings: &[FileBackedMapping],
    removed: VirtRange,
) -> Result<Vec<FileBackedMapping>, UserMmRuntimeError> {
    let mut retained = Vec::new();
    retained
        .try_reserve(mappings.len().saturating_mul(2))
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    for mapping in mappings {
        if !mapping.range.overlaps(removed) {
            retained.push(mapping.clone());
            continue;
        }
        let overlap_start = core::cmp::max(mapping.range.start(), removed.start());
        let overlap_end = core::cmp::min(mapping.range.end(), removed.end());
        if mapping.range.start() < overlap_start {
            let left = VirtRange::new(mapping.range.start(), overlap_start)
                .ok_or(UserMmRuntimeError::InvalidRange)?;
            let mut fragment = mapping.clone();
            fragment.range = left;
            fragment.file_length = fragment.file_length.min(left.size());
            retained.push(fragment);
        }
        if overlap_end < mapping.range.end() {
            let right = VirtRange::new(overlap_end, mapping.range.end())
                .ok_or(UserMmRuntimeError::InvalidRange)?;
            let delta = overlap_end
                .get()
                .checked_sub(mapping.range.start().get())
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let mut fragment = mapping.clone();
            fragment.range = right;
            fragment.file_offset = fragment
                .file_offset
                .checked_add(delta as u64)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            fragment.file_length = mapping.file_length.saturating_sub(delta).min(right.size());
            retained.push(fragment);
        }
    }
    Ok(retained)
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
        // PROT_NONE deliberately removes the leaf PTE while retaining both
        // the VMA and this backing-owner record.  A later MAP_FIXED/munmap of
        // that range must still retire the owned page; Linux does not reject
        // the operation merely because there is no present translation.
        if let Some(physical) = page_table.translate(mapping.page.start_address())? {
            assert_eq!(
                physical,
                mapping.backing.physical(),
                "user retirement preflight found a mismatched backing frame",
            );
        }
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
        if page_table.translate(mapping.page.start_address())?.is_some() {
            let frame = page_table.unmap_page(mapping.page)?;
            assert_eq!(
                frame,
                mapping.backing.frame(),
                "user unmap returned a different backing frame",
            );
        }
        page_table
            .reclaim_empty_tables(mapping.page, &mut tables)
            .expect("user page-table reclamation violated the reviewed topology");
        if let PageBacking::Owned(allocation) = mapping.backing {
            backings.push(allocation);
        }
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

fn file_page_cache_lookup(key: (u64, u64, u64)) -> Option<PhysFrame> {
    FILE_PAGE_CACHE.lock().get(&key).map(PageAllocation::start)
}

/// Retain immutable file pages for the kernel lifetime. The fixed 512 MiB
/// ceiling bounds pinning under the 8 GiB BuildStorm configuration; small
/// CAgent runs populate only their few dynamic-library pages.
fn file_page_cache_install(
    key: (u64, u64, u64),
    allocation: PageAllocation,
) -> (PageBacking, Option<PageAllocation>) {
    let mut cache = FILE_PAGE_CACHE.lock();
    if let Some(existing) = cache.get(&key) {
        return (PageBacking::Shared(existing.start()), Some(allocation));
    }
    if cache.len() >= FILE_PAGE_CACHE_CAPACITY {
        return (PageBacking::Owned(allocation), None);
    }
    let frame = allocation.start();
    cache.insert(key, allocation);
    (PageBacking::Shared(frame), None)
}

fn free_uninstalled_backing(backing: PageBacking) -> Result<(), UserMmRuntimeError> {
    if let PageBacking::Owned(allocation) = backing {
        crate::page_alloc::free(allocation)?;
    }
    Ok(())
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

fn copy_to_mapped_pages(
    page_table: &RuntimePageTable,
    address: usize,
    input: &[u8],
) -> Result<(), UserMmRuntimeError> {
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

        // SAFETY: the caller validated write permission and residency for the
        // complete range while holding the MM lock; input contains `chunk`
        // bytes from this offset and the translation names owned user RAM.
        unsafe {
            core::ptr::copy_nonoverlapping(input.as_ptr().add(copied), destination, chunk);
        }
        copied += chunk;
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
        let allocation = match allocator.allocate() {
            Ok(allocation) => allocation,
            Err(error) => return Err(error.into()),
        };
        if allocation.requires_global_flush() {
            assert_eq!(
                ASID_ROLLOVER_IN_PROGRESS.compare_exchange(
                    false,
                    true,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ),
                Ok(false),
                "ASID reuse gate changed while holding the allocator lock",
            );
        }
        LIVE_MMS.fetch_add(1, Ordering::AcqRel);
        allocation
    };

    if allocation.requires_global_flush() {
        // Do not hold the Vm-ranked allocator lock while entering the lower
        // CrossCpu-ranked shootdown serializer. The atomic gate prevents any
        // new-generation token from becoming visible before the global flush.
        crate::tlb::shootdown_kernel_all();
        assert!(
            ASID_ROLLOVER_IN_PROGRESS.swap(false, Ordering::AcqRel),
            "ASID rollover gate was cleared before publication",
        );
    }
    Ok(allocation.token())
}

fn release_mm_reservation(asid: AsidToken) {
    {
        let mut slot = ASID_ALLOCATOR.lock();
        slot.as_mut()
            .expect("releasing an ASID before allocator initialization")
            .release(asid);
    }
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
