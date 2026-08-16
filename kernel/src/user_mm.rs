use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use core::cmp::min;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use myos_mm::{
    AddressSpaceError, AsidAllocator, AsidAllocatorError, AsidToken, FaultAccess, FaultSource,
    PAGE_SIZE, PageAllocation, PageFault, PerMmTlbRequest, PhysAddr, PhysFrame, TlbFlush, TlbScope,
    UserAddressSpace, UserFaultPlan, UserMmError, UserTlbContext, VirtAddr, VirtPage, VirtRange,
    VmArea, VmAreaError, VmAreaFlags,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::runtime_page_table::{RuntimePageTable, RuntimePageTableError};
use crate::tracked_spin::{TrackedSpinLock, TrackedSpinLockGuard};

// Native toolchains keep many shared objects, allocator arenas, thread stacks,
// and guard mappings live at once. LoongArch rustc crosses 1024 VMAs while
// loading librustc_driver and its jemalloc arenas. VmAreaSet stores only live
// entries on the heap, so this Linux-like ceiling does not preallocate 64K
// records and does not consume a kernel task's 64-KiB stack.
const VMA_CAPACITY: usize = 65_536;
const ANONYMOUS_FAULT_CLUSTER_PAGES: usize = 16;

static ASID_ALLOCATOR: IrqSpinLock<Option<AsidAllocator>> =
    IrqSpinLock::new_with_class(None, LockClass::new("user_asid_allocator", LockRank::Vm, 1));
static ASID_ROLLOVER_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LIVE_MMS: AtomicUsize = AtomicUsize::new(0);
static LIVE_ROOTS: AtomicUsize = AtomicUsize::new(0);
static LIVE_BACKINGS: AtomicUsize = AtomicUsize::new(0);
const FILE_PAGE_CACHE_SHARDS: usize = 64;
const FILE_PAGE_CACHE_CAPACITY_PER_SHARD: usize = 4 * 1024;
type FilePageCacheKey = (u64, u64, u64, u64);
static FILE_PAGE_CACHE: [IrqSpinLock<BTreeMap<FilePageCacheKey, PageAllocation>>;
    FILE_PAGE_CACHE_SHARDS] = [const {
    IrqSpinLock::new_with_class(
        BTreeMap::new(),
        LockClass::new("file_page_cache", LockRank::Vm, 3),
    )
}; FILE_PAGE_CACHE_SHARDS];

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
    /// Anonymous/private page shared by fork. The global allocator reference
    /// count owns the physical frame until every parent/child mapping drops it.
    Cow(PhysFrame),
    Shared(PhysFrame),
}

impl PageBacking {
    fn frame(&self) -> PhysFrame {
        match self {
            Self::Owned(allocation) => allocation.start(),
            Self::Cow(frame) | Self::Shared(frame) => *frame,
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
    kind: ForkPageKind,
    area: VmArea,
    present: bool,
}

#[derive(Clone, Copy)]
enum ForkPageKind {
    Cow,
    FileShared,
}

#[derive(Clone)]
struct FileBackedMapping {
    range: VirtRange,
    file_offset: u64,
    file_length: usize,
    generation: u64,
    device: u64,
    inode: u64,
    content_generation: u64,
    shared_cache: bool,
    file: myos_vfs::ArcFile,
}

pub(crate) struct FileFaultRequest {
    pub(crate) file: myos_vfs::ArcFile,
    pub(crate) file_offset: u64,
    pub(crate) read_length: usize,
    pub(crate) page: VirtAddr,
    cache_key: FilePageCacheKey,
    shared_cache: bool,
    generation: u64,
}

/// Cold MM state: VMA layout and file-mapping metadata. Mutated by
/// mmap/munmap/mprotect/brk and read by page-fault resolution. Acquired
/// before `hot`; never held across file I/O, allocation, or TLB waits.
struct UserMmColdState {
    core: Box<UserAddressSpace<VMA_CAPACITY>>,
    // File-backed VMAs indexed by virtual start address. A rustc process can
    // carry hundreds of loader mappings and take hundreds of thousands of
    // file faults; a linear Vec scan here multiplied both numbers together.
    file_mappings: BTreeMap<usize, FileBackedMapping>,
    next_file_generation: u64,
}

/// Hot MM state: the hardware page table and resident-page ownership.
/// Mutated on every fault, COW resolution, and unmap; acquired after `cold`.
struct UserMmHotState {
    page_table: Option<RuntimePageTable>,
    // Resident-page ownership indexed by virtual page address.  Rustc issues
    // thousands of small mmap/munmap/mprotect operations; an unordered Vec
    // made each range retirement scan every page in the address space.
    pages: BTreeMap<usize, MappedPage>,
}

/// Page-table and backing allocations detached under the MM lock.
///
/// The caller must complete the optional TLB request before releasing either
/// allocation vector. Keeping the request and retired storage in one value
/// makes the TLB-before-free contract explicit at every call site.
struct RetirementBatch {
    request: Option<PerMmTlbRequest>,
    backings: Vec<PageAllocation>,
    cow_frames: Vec<PhysFrame>,
    page_tables: Vec<PageAllocation>,
}

impl RetirementBatch {
    fn empty() -> Self {
        Self {
            request: None,
            backings: Vec::new(),
            cow_frames: Vec::new(),
            page_tables: Vec::new(),
        }
    }
}

pub struct UserMm {
    /// §7: the former single `user_mm` lock is split so the hot fault path
    /// (page table + resident pages) is not serialized behind cold VMA and
    /// file-mapping mutations. Ordering is always cold → hot.
    cold: TrackedSpinLock<UserMmColdState>,
    hot: TrackedSpinLock<UserMmHotState>,
    tlb: UserTlbContext,
    root: PhysFrame,
    /// Last TLB generation known synchronized on each CPU. ASIDs let a CPU
    /// retain translations while another process runs; a generation mismatch
    /// is the only normal-switch condition that requires a local flush.
    local_tlb_generation: [AtomicU64; crate::smp::MAX_CPUS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultRecovery {
    Anonymous,
    StackGrowth,
    Spurious,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultFailure {
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
        let cold = self.cold.lock();
        (cold.core.layout().area_count(), VMA_CAPACITY)
    }

    /// Return the single VMA that completely contains `range`.
    ///
    /// mremap uses this snapshot to preserve access flags while resizing an
    /// anonymous mapping.  Requiring one containing VMA avoids silently
    /// merging mappings with different protections or backing kinds.
    pub fn area_containing(&self, range: VirtRange) -> Option<VmArea> {
        let cold = self.cold.lock();
        let area = cold.core.layout().find_area(range.start())?;
        area.range().contains_range(range).then_some(area)
    }

    /// Resolve a user PC back to its file-backed mapping for fatal-fault
    /// diagnostics. This runs only after a process is already doomed, so the
    /// owned path keeps the MM lock out of console output and VFS formatting.
    pub(crate) fn debug_file_location(
        &self,
        address: VirtAddr,
    ) -> Option<(String, usize, usize, u64)> {
        let cold = self.cold.lock();
        let mapping = file_mapping_at(&cold.file_mappings, address)?;
        let delta = address.get().checked_sub(mapping.range.start().get())?;
        let file_offset = mapping.file_offset.checked_add(delta as u64)?;
        Some((
            String::from(mapping.file.path().unwrap_or("?")),
            mapping.range.start().get(),
            mapping.range.end().get(),
            file_offset,
        ))
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
            let root = page_table.root_frame();
            let tlb = core.tlb_context();
            LIVE_ROOTS.fetch_add(1, Ordering::AcqRel);
            Ok(Self {
                // P0-1: UserMm holders are pinned (migration + preemption)
                // while IRQs stay enabled, so a waiter never depends on the
                // SpinIrqWindow to reopen IRQs inside an unknown outer lock
                // context. The window was the entry point of the SCHEDULER
                // self-deadlock observed under BuildStorm.
                cold: TrackedSpinLock::new_with_class(
                    UserMmColdState {
                        core,
                        file_mappings: BTreeMap::new(),
                        next_file_generation: 1,
                    },
                    LockClass::new("user_mm", LockRank::CrossCpu, 10),
                ),
                hot: TrackedSpinLock::new_with_class(
                    UserMmHotState {
                        page_table: Some(page_table),
                        pages: BTreeMap::new(),
                    },
                    LockClass::new("user_mm_hot", LockRank::CrossCpu, 11),
                ),
                tlb,
                root,
                local_tlb_generation: [const { AtomicU64::new(u64::MAX) }; crate::smp::MAX_CPUS],
            })
        })();

        if result.is_err() {
            release_mm_reservation(asid);
        }
        result
    }

    /// Acquires cold then hot (§7). Both guards live for the caller's
    /// critical section; release order is the reverse (hot then cold).
    fn lock_both(
        &self,
    ) -> (
        TrackedSpinLockGuard<'_, UserMmColdState>,
        TrackedSpinLockGuard<'_, UserMmHotState>,
    ) {
        let cold = self.cold.lock();
        let hot = self.hot.lock();
        (cold, hot)
    }

    pub fn asid(&self) -> AsidToken {
        self.tlb.asid()
    }

    /// Last TLB generation known synchronized on one CPU (§10/§11
    /// diagnostics and generation checks).
    pub(crate) fn local_tlb_generation(&self, cpu: crate::smp::CpuId) -> u64 {
        self.local_tlb_generation[cpu.get()].load(core::sync::atomic::Ordering::Acquire)
    }

    pub fn root_is_private(&self) -> Result<bool, UserMmRuntimeError> {
        let hot = self.hot.lock();
        let page_table = hot
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        Ok(page_table.is_owned_user_root()
            && page_table.root_frame() != crate::vm::kernel_page_table_root()?)
    }

    /// Snapshot-forks this address space: private writable pages are
    /// converted to reference-counted COW frames with both sides read-only;
    /// content is never copied here (copy happens lazily on write fault).
    pub fn fork_clone_cow(&self) -> Result<alloc::boxed::Box<Self>, UserMmRuntimeError> {
        // Allocate the empty root/ASID before taking user_mm. The complete VMA
        // and resident-page snapshot is then captured under one parent lock,
        // so mmap activity in another rustc thread cannot make fork fail or
        // mix two generations of the address space.
        let child = alloc::boxed::Box::new(Self::new(&[])?);
        let (areas, program_break, mapped_pages, file_mappings, next_file_generation, request) = {
            let (mut cold, mut hot) = self.lock_both();
            let layout = cold.core.layout();

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
            let program_break = layout.program_break();

            let mut keys = Vec::new();
            keys.try_reserve(hot.pages.len())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            keys.extend(hot.pages.keys().copied());

            let mut mapped_pages = Vec::new();
            mapped_pages
                .try_reserve(keys.len())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            let mut retained_cow_frames = Vec::new();
            retained_cow_frames
                .try_reserve(keys.len())
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
            let mut first_protected = None::<VirtAddr>;
            let mut last_protected_end = None::<VirtAddr>;

            let result: Result<(), UserMmRuntimeError> = (|| {
                for key in keys {
                    let (page, physical, kind, area, present) = {
                        let mapping = hot
                            .pages
                            .get(&key)
                            .expect("fork page disappeared while user_mm was locked");
                        let area = cold
                            .core
                            .layout()
                            .find_area(mapping.page.start_address())
                            .ok_or(UserMmRuntimeError::PermissionDenied)?;
                        let physical = mapping.backing.physical();
                        let translated = hot
                            .page_table
                            .as_ref()
                            .ok_or(UserMmRuntimeError::NotMapped)?
                            .translate(mapping.page.start_address())?;
                        if let Some(translated) = translated {
                            if translated != physical {
                                return Err(UserMmRuntimeError::NotMapped);
                            }
                        } else if area.flags().access_only() != VmAreaFlags::empty() {
                            return Err(UserMmRuntimeError::NotMapped);
                        }
                        let kind = if matches!(&mapping.backing, PageBacking::Shared(_)) {
                            ForkPageKind::FileShared
                        } else {
                            ForkPageKind::Cow
                        };
                        (mapping.page, physical, kind, area, translated.is_some())
                    };

                    if matches!(kind, ForkPageKind::Cow) {
                        let frame = PhysFrame::from_start_address(physical)
                            .ok_or(UserMmRuntimeError::AddressOverflow)?;
                        // Reference acquisition is batched after the walk: one
                        // global allocator lock acquisition for the whole mm
                        // instead of one per resident page.
                        retained_cow_frames.push(frame);
                    }

                    mapped_pages.push(MappedPageSource {
                        page,
                        physical,
                        kind,
                        area,
                        present,
                    });
                }
                Ok(())
            })();

            if let Err(error) = result {
                // No references were taken and no page was converted yet.
                return Err(error);
            }

            if !retained_cow_frames.is_empty()
                && let Err(error) =
                    crate::page_alloc::increment_reference_many(&retained_cow_frames)
            {
                return Err(error.into());
            }

            let conversion_result: Result<(), UserMmRuntimeError> = (|| {
                for source in &mapped_pages {
                    if !matches!(source.kind, ForkPageKind::Cow) {
                        continue;
                    }
                    let frame = PhysFrame::from_start_address(source.physical)
                        .ok_or(UserMmRuntimeError::AddressOverflow)?;
                    let mapping = hot
                        .pages
                        .get_mut(&source.page.start_address().get())
                        .expect("fork page disappeared while converting to COW");
                    let previous =
                        core::mem::replace(&mut mapping.backing, PageBacking::Cow(frame));
                    debug_assert!(matches!(
                        previous,
                        PageBacking::Owned(_) | PageBacking::Cow(_)
                    ));

                    if source.present && source.area.flags().is_writable() {
                        let readonly = VmArea::new(
                            source.area.range(),
                            source.area.flags().without(VmAreaFlags::WRITE),
                            source.area.kind(),
                        );
                        apply_page_protection(
                            hot.page_table
                                .as_mut()
                                .ok_or(UserMmRuntimeError::NotMapped)?,
                            source.page,
                            frame,
                            readonly,
                        )?;
                        first_protected.get_or_insert(source.page.start_address());
                        last_protected_end = Some(
                            source.page
                                .start_address()
                                .checked_add(PAGE_SIZE)
                                .ok_or(UserMmRuntimeError::AddressOverflow)?,
                        );
                    }
                }
                Ok(())
            })();

            if let Err(error) = conversion_result {
                // Converted parent pages remain valid COW pages with one
                // reference; a read-only leaf will become writable on its
                // next write fault. Only the prospective child references
                // need rolling back here.
                for frame in retained_cow_frames {
                    release_cow_frame(frame)?;
                }
                return Err(error);
            }

            let request = match (first_protected, last_protected_end) {
                (Some(start), Some(end)) => match cold.core.plan_tlb_request(TlbFlush::Range {
                    scope: TlbScope::AddressSpace(cold.core.asid().id()),
                    range: VirtRange::from_bounds(start.get(), end.get()),
                }) {
                    Ok(request) => Some(request),
                    Err(error) => {
                        for frame in retained_cow_frames {
                            release_cow_frame(frame)?;
                        }
                        return Err(error.into());
                    }
                },
                _ => None,
            };
            (
                areas,
                program_break,
                mapped_pages,
                cold.file_mappings.clone(),
                cold.next_file_generation,
                request,
            )
        };

        // This is the fork snapshot point. Every CPU that could still hold a
        // writable parent translation must acknowledge the revocation before
        // either address space can run independently.
        if let Some(request) = request {
            self.shootdown_user_request(request);
        }

        let child_layout_result: Result<(), UserMmRuntimeError> = (|| {
            let mut cold = child.cold.lock();
            for area in &areas {
                cold.core.map_area(*area)?;
            }
            if let Some(program_break) = program_break {
                cold.core
                    .layout_mut()
                    .configure_program_break(program_break.start(), program_break.limit())
                    .map_err(UserMmError::from)?;
                cold.core
                    .layout_mut()
                    .set_program_break_and_sync_heap(program_break.current())
                    .map_err(UserMmError::from)?;
            }
            cold.file_mappings = file_mappings;
            cold.next_file_generation = next_file_generation;
            Ok(())
        })();
        if let Err(error) = child_layout_result {
            for source in &mapped_pages {
                if matches!(source.kind, ForkPageKind::Cow) {
                    let frame = PhysFrame::from_start_address(source.physical)
                        .ok_or(UserMmRuntimeError::AddressOverflow)?;
                    release_cow_frame(frame)?;
                }
            }
            return Err(error);
        }

        // The child mm is not published anywhere yet: one lock acquisition
        // for the whole mapping loop instead of one per resident page.
        let (mut cold, mut hot) = child.lock_both();
        for (index, source) in mapped_pages.iter().copied().enumerate() {
            let area = cold
                .core
                .layout()
                .find_area(source.page.start_address())
                .ok_or(UserMmRuntimeError::PermissionDenied)?;

            let frame = PhysFrame::from_start_address(source.physical)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let backing = match source.kind {
                ForkPageKind::Cow => PageBacking::Cow(frame),
                ForkPageKind::FileShared => PageBacking::Shared(frame),
            };

            if area.flags().access_only() != VmAreaFlags::empty() {
                let mapped_area = match source.kind {
                    ForkPageKind::Cow if area.flags().is_writable() => VmArea::new(
                        area.range(),
                        area.flags().without(VmAreaFlags::WRITE),
                        area.kind(),
                    ),
                    _ => area,
                };
                let page_table = hot
                    .page_table
                    .as_mut()
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                if let Err(error) =
                    page_table.map_page(source.page, backing.frame(), mapped_area.mapping_options())
                {
                    drop(hot);
                    drop(cold);
                    // The child Drop releases already-installed COW pages;
                    // release this page and every not-yet-installed child
                    // reference explicitly.
                    for pending in &mapped_pages[index..] {
                        if matches!(pending.kind, ForkPageKind::Cow) {
                            let pending_frame = PhysFrame::from_start_address(pending.physical)
                                .ok_or(UserMmRuntimeError::AddressOverflow)?;
                            release_cow_frame(pending_frame)?;
                        }
                    }
                    return Err(error.into());
                }
            }

            let previous = hot
                .pages
                .insert(source.page.start_address().get(), MappedPage {
                    page: source.page,
                    backing,
                });
            debug_assert!(previous.is_none());
        }
        drop(hot);
        drop(cold);

        Ok(child)
    }

    pub fn kernel_mapping_is_shared(
        &self,
        kernel_address: VirtAddr,
    ) -> Result<bool, UserMmRuntimeError> {
        #[cfg(target_arch = "riscv64")]
        {
            let hot = self.hot.lock();
            let user = hot
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
        let (mut cold, mut hot) = self.lock_both();
        let area = cold
            .core
            .layout()
            .find_area(address)
            .ok_or(UserMmRuntimeError::PermissionDenied)?;
        let page_address = address
            .align_down(PAGE_SIZE)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let page =
            VirtPage::from_start_address(page_address).ok_or(UserMmRuntimeError::InvalidRange)?;

        if let Some(physical) = hot
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .translate(address)?
        {
            return Ok(physical);
        }

        // A fork-Cow-ified page without a PTE keeps its frame: map the
        // existing frame read-only (content = the parent's fork snapshot)
        // instead of a fresh zero page. The entry stays Cow; the next write
        // fault breaks COW through the resolver.
        let cow_frame = match hot.pages.get(&page.start_address().get()) {
            Some(MappedPage {
                backing: PageBacking::Cow(frame),
                ..
            }) => Some(*frame),
            _ => None,
        };
        if let Some(frame) = cow_frame {
            let mapped_area = if area.flags().is_writable() {
                VmArea::new(
                    area.range(),
                    area.flags().without(VmAreaFlags::WRITE),
                    area.kind(),
                )
            } else {
                area
            };
            let offset = address
                .get()
                .checked_sub(page_address.get())
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let physical = frame
                .start_address()
                .checked_add(offset)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            hot.page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .map_page(page, frame, mapped_area.mapping_options())?;
            return Ok(physical);
        }

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
        let page_table = hot
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        if let Err(error) = page_table.map_page(page, backing.start(), area.mapping_options()) {
            crate::page_alloc::free(backing)?;
            return Err(error.into());
        }

        let previous = hot.pages.insert(page.start_address().get(), MappedPage {
            page,
            backing: PageBacking::Owned(backing),
        });
        debug_assert!(previous.is_none());
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
        let (cold, mut hot) = self.lock_both();

        let mut copied = 0;
        while copied < input.len() {
            let current = address
                .checked_add(copied)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            let area = cold
                .core
                .layout()
                .find_area(current)
                .ok_or(UserMmRuntimeError::PermissionDenied)?;
            let physical = map_zero_page_locked(&mut hot, area, current)?;
            let in_page = current.get() & (PAGE_SIZE - 1);
            let chunk = min(PAGE_SIZE - in_page, input.len() - copied);
            let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(physical)
                .map_err(|_| UserMmRuntimeError::NotMapped)?;
            // SAFETY: the populated translation names RAM owned by this MM,
            // and the copy is bounded to its current page.
            unsafe {
                core::ptr::copy_nonoverlapping(input.as_ptr().add(copied), destination, chunk);
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
        let (cold, mut hot) = self.lock_both();

        let mut page = first_page;
        while page.get() < end {
            let area = cold
                .core
                .layout()
                .find_area(page)
                .ok_or(UserMmRuntimeError::PermissionDenied)?;
            map_zero_page_locked(&mut hot, area, page)?;
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

        let (cold, hot) = self.lock_both();
        validate_range(&cold, address, output.len(), FaultAccess::Read)?;
        let page_table = hot
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
        //
        // A resident destination may still sit behind a COW read-only leaf
        // (a fork-converted private page). Kernel-side writes must break COW
        // first: writing through the kernel alias would silently modify the
        // frame every side of the fork shares.
        {
            let (mut cold, mut hot) = self.lock_both();
            validate_range(&cold, address, input.len(), FaultAccess::Write)?;
            let fully_mapped = {
                let page_table = hot
                    .page_table
                    .as_ref()
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                match validate_mapped_range(page_table, address, input.len()) {
                    Ok(()) => true,
                    Err(UserMmRuntimeError::NotMapped) => false,
                    Err(error) => return Err(error),
                }
            };
            if fully_mapped {
                let requests = Self::break_cow_for_range(&mut cold, &mut hot, address, input.len())?;
                {
                    let page_table = hot
                        .page_table
                        .as_ref()
                        .ok_or(UserMmRuntimeError::NotMapped)?;
                    copy_to_mapped_pages(page_table, address, input)?;
                }
                drop(hot);
                drop(cold);
                self.flush_local_requests(requests)?;
                return Ok(());
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

        let (mut cold, mut hot) = self.lock_both();
        validate_range(&cold, address, input.len(), FaultAccess::Write)?;
        validate_mapped_range(
            hot.page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?,
            address,
            input.len(),
        )?;
        let requests = Self::break_cow_for_range(&mut cold, &mut hot, address, input.len())?;
        {
            let page_table = hot
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            copy_to_mapped_pages(page_table, address, input)?;
        }
        drop(hot);
        drop(cold);
        self.flush_local_requests(requests)?;
        Ok(())
    }

    /// Breaks COW on every Cow-backed page of `[address, address + length)`
    /// so a subsequent kernel write via the direct map cannot modify a frame
    /// shared with a fork sibling. Returns the local TLB requests that
    /// publish the new leaves; the caller must execute them after releasing
    /// the MM lock.
    fn break_cow_for_range(
        cold: &mut UserMmColdState,
        hot: &mut UserMmHotState,
        address: usize,
        length: usize,
    ) -> Result<Vec<PerMmTlbRequest>, UserMmRuntimeError> {
        let end = address
            .checked_add(length)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        let mut requests = Vec::new();
        let mut page = address & !(PAGE_SIZE - 1);
        while page < end {
            if let Some((_, Some(request))) =
                resolve_cow_write_fault_locked(cold, hot, VirtAddr::new(page), FaultAccess::Write)?
            {
                requests.push(request);
            }
            page = page
                .checked_add(PAGE_SIZE)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
        }
        Ok(requests)
    }

    /// Executes post-COW-break TLB requests. copy_to_user runs in syscall
    /// context with interrupts enabled, so this uses the interruptible
    /// `shootdown_user` path (the IRQ-disabled `_local` variant is reserved
    /// for fault context). The same-ASID argument applies: replaced leaves
    /// point at new frames (old translations stay read-only and re-fault
    /// correctly), restored-write leaves are repaired by spurious recovery
    /// on remote CPUs.
    fn flush_local_requests(&self, requests: Vec<PerMmTlbRequest>) -> Result<(), UserMmRuntimeError> {
        for request in requests {
            let request = request
                .local_only(crate::smp::current_cpu_id().get())
                .map_err(UserMmRuntimeError::from)?;
            self.shootdown_user_request(request);
        }
        Ok(())
    }

    pub fn configure_program_break(
        &self,
        start: VirtAddr,
        limit: VirtAddr,
    ) -> Result<(), UserMmRuntimeError> {
        let mut cold = self.cold.lock();
        cold.core
            .layout_mut()
            .configure_program_break(start, limit)
            .map_err(UserMmError::from)?;
        Ok(())
    }

    pub fn program_break(&self) -> Result<VirtAddr, UserMmRuntimeError> {
        let cold = self.cold.lock();
        cold.core
            .layout()
            .program_break()
            .map(|program_break| program_break.current())
            .ok_or(UserMmRuntimeError::InvalidRange)
    }

    pub fn set_program_break(&self, new_break: VirtAddr) -> Result<VirtAddr, UserMmRuntimeError> {
        let (current, retirement) = {
            let (mut cold, mut hot) = self.lock_both();
            let old_layout = cold.core.layout().clone();
            let old = old_layout
                .program_break()
                .ok_or(UserMmRuntimeError::InvalidRange)?;
            let current = cold
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
                match retire_range_locked(&mut cold, &mut hot, range) {
                    Ok(retirement) => (current, retirement),
                    Err(error) => {
                        *cold.core.layout_mut() = old_layout;
                        return Err(error);
                    }
                }
            } else {
                (current, RetirementBatch::empty())
            }
        };
        self.finish_retirement(retirement)?;
        Ok(current)
    }

    pub fn map_anonymous(
        &self,
        search: VirtRange,
        size: usize,
        flags: VmAreaFlags,
    ) -> Result<VirtAddr, UserMmRuntimeError> {
        let mut cold = self.cold.lock();
        let area = cold
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
        let mut cold = self.cold.lock();
        cold.core
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
            let (mut cold, mut hot) = self.lock_both();
            let old_layout = cold.core.layout().clone();
            let retained_file_mappings = file_mappings_without_range(&cold.file_mappings, range)?;

            // 1. Remove overlapping VMAs from the topology.
            cold.core
                .layout_mut()
                .unmap_range(range)
                .map_err(UserMmError::from)?;

            // 2. Insert the new exact VMA before detaching old PTEs/backings so
            //    that a rollback of this step only needs to restore the layout.
            if let Err(e) =
                cold.core
                    .map_area(VmArea::new(range, flags, myos_mm::VmAreaKind::Anonymous))
            {
                *cold.core.layout_mut() = old_layout;
                return Err(UserMmRuntimeError::Core(e));
            }

            // 3. Detach old PTEs and backings.  Allocations happen before any
            //    PTE/backing modification, so an error here leaves the state
            //    consistent and a layout rollback is sufficient.
            match retire_range_locked(&mut cold, &mut hot, range) {
                Ok(retirement) => {
                    cold.file_mappings = retained_file_mappings;
                    retirement
                }
                Err(error) => {
                    *cold.core.layout_mut() = old_layout;
                    return Err(error);
                }
            }
        };
        self.finish_retirement(retirement)?;
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
        let mut cold = self.cold.lock();
        cold.core
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
        content_generation: u64,
        shared_cache: bool,
        file: myos_vfs::ArcFile,
    ) -> Result<(), UserMmRuntimeError> {
        let mut cold = self.cold.lock();
        let area = cold
            .core
            .layout()
            .find_area(range.start())
            .ok_or(UserMmRuntimeError::NotMapped)?;
        if !area.range().contains_range(range) {
            return Err(UserMmRuntimeError::InvalidRange);
        }
        let generation = cold.next_file_generation;
        cold.next_file_generation = cold.next_file_generation.wrapping_add(1).max(1);
        let mapping = FileBackedMapping {
            range,
            file_offset,
            file_length,
            generation,
            device,
            inode,
            content_generation,
            shared_cache,
            file,
        };
        if cold
            .file_mappings
            .insert(range.start().get(), mapping)
            .is_some()
        {
            return Err(UserMmRuntimeError::InvalidRange);
        }
        Ok(())
    }

    pub fn file_fault_request(
        &self,
        address: VirtAddr,
        access: FaultAccess,
    ) -> Result<Option<FileFaultRequest>, UserMmRuntimeError> {
        let (cold, hot) = self.lock_both();
        file_fault_request_locked(&cold, &hot, address, access)
    }

    /// Collect a sequential fault-around window while taking user_mm once.
    /// The old caller performed one lock/lookup/Arc clone round trip per page,
    /// which made a 64-page cache hit contend on the same MM 64 times.
    pub fn file_fault_cluster(
        &self,
        first: FileFaultRequest,
        access: FaultAccess,
        maximum: usize,
    ) -> Result<Vec<FileFaultRequest>, UserMmRuntimeError> {
        let first_page = first.page;
        let first_offset = first.file_offset;
        let file = alloc::sync::Arc::clone(&first.file);
        let mut requests = Vec::new();
        requests
            .try_reserve(maximum)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        requests.push(first);
        let (cold, hot) = self.lock_both();
        for index in 1..maximum {
            if requests
                .last()
                .is_some_and(|request| request.read_length < PAGE_SIZE)
            {
                break;
            }
            let Some(page) = first_page.checked_add(index * PAGE_SIZE) else {
                break;
            };
            let Some(next) = file_fault_request_locked(&cold, &hot, page, access)? else {
                break;
            };
            let expected_offset = first_offset
                .checked_add((index * PAGE_SIZE) as u64)
                .ok_or(UserMmRuntimeError::AddressOverflow)?;
            if next.file_offset != expected_offset || !alloc::sync::Arc::ptr_eq(&file, &next.file) {
                break;
            }
            requests.push(next);
        }
        Ok(requests)
    }

    pub fn install_file_fault(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        self.install_file_fault_inner(fault, data, true, None)
    }

    /// Map a full file page already retained by the kernel-wide cache without
    /// issuing another VFS read or taking the cache lock twice.
    pub fn install_cached_file_fault(
        &self,
        fault: &FileFaultRequest,
    ) -> Result<Option<UserFaultResolution>, UserMmRuntimeError> {
        self.install_cached_file_page(fault, true)
    }

    /// Fault-around companion for an already cached file page. The demanded
    /// page is installed last with a local invalidation, publishing all of the
    /// preceding leaf writes with one architecture barrier.
    pub fn install_cached_file_prefetch(
        &self,
        fault: &FileFaultRequest,
    ) -> Result<Option<UserFaultResolution>, UserMmRuntimeError> {
        self.install_cached_file_page(fault, false)
    }

    /// Install the cached prefix of a fault-around window under one MM lock.
    /// Returns None only when the demanded first page is not cached.
    pub fn install_cached_file_cluster(
        &self,
        faults: &[FileFaultRequest],
    ) -> Result<Option<UserFaultResolution>, UserMmRuntimeError> {
        let Some(first) = faults.first() else {
            return Ok(None);
        };
        let mut frames = Vec::new();
        frames
            .try_reserve(faults.len())
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        for fault in faults {
            if !fault.shared_cache || fault.read_length != PAGE_SIZE {
                break;
            }
            let Some(frame) = file_page_cache_lookup(fault.cache_key) else {
                break;
            };
            frames.push(frame);
        }
        if frames.is_empty() {
            return Ok(None);
        }

        let (request, first_spurious, first_unmapped, installed_any) = {
            let (cold, mut hot) = self.lock_both();
            let request = cold.core.plan_post_install_tlb(first.page)?;
            let mut first_spurious = false;
            let mut first_unmapped = false;
            let mut installed_any = false;
            for (index, (fault, frame)) in faults.iter().zip(frames.iter()).enumerate() {
                let still_mapped = file_mapping_at(&cold.file_mappings, fault.page)
                    .is_some_and(|mapping| mapping.generation == fault.generation);
                if !still_mapped {
                    if index == 0 {
                        first_unmapped = true;
                    }
                    break;
                }
                if hot
                    .page_table
                    .as_ref()
                    .ok_or(UserMmRuntimeError::NotMapped)?
                    .translate(fault.page)?
                    .is_some()
                {
                    if index == 0 {
                        first_spurious = true;
                    }
                    continue;
                }
                let area = cold
                    .core
                    .layout()
                    .find_area(fault.page)
                    .ok_or(UserMmRuntimeError::NotMapped)?;
                let page = VirtPage::from_start_address(fault.page)
                    .ok_or(UserMmRuntimeError::InvalidRange)?;
                // A writable VMA must not alias a shared cache frame (see
                // install_file_fault_inner); convert before the PTE install.
                let (install_frame, install_backing) = if area.flags().is_writable() {
                    let allocation = crate::page_alloc::allocate(
                        0,
                        crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
                    )?;
                    if let Err(error) =
                        copy_physical_page(frame.start_address(), allocation.start().start_address())
                    {
                        crate::page_alloc::free(allocation)?;
                        return Err(error);
                    }
                    (allocation.start(), PageBacking::Owned(allocation))
                } else {
                    (*frame, PageBacking::Shared(*frame))
                };
                if let Err(error) = hot
                    .page_table
                    .as_mut()
                    .ok_or(UserMmRuntimeError::NotMapped)?
                    .map_page(page, install_frame, area.mapping_options())
                {
                    if let PageBacking::Owned(allocation) = install_backing {
                        crate::page_alloc::free(allocation)?;
                    }
                    return Err(error.into());
                }
                let owned = matches!(&install_backing, PageBacking::Owned(_));
                let previous = hot.pages.insert(page.start_address().get(), MappedPage {
                    page,
                    backing: install_backing,
                });
                debug_assert!(previous.is_none());
                if owned {
                    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
                }
                installed_any = true;
            }
            (request, first_spurious, first_unmapped, installed_any)
        };

        if installed_any {
            flush_post_install_local(request)?;
        }
        if first_unmapped {
            return Ok(Some(UserFaultResolution::Fatal(
                UserFaultFailure::SegmentationViolation,
            )));
        }
        Ok(Some(UserFaultResolution::Recovered(if first_spurious {
            UserFaultRecovery::Spurious
        } else {
            UserFaultRecovery::Anonymous
        })))
    }

    fn install_cached_file_page(
        &self,
        fault: &FileFaultRequest,
        flush_local: bool,
    ) -> Result<Option<UserFaultResolution>, UserMmRuntimeError> {
        static UNUSED_CACHE_HIT_DATA: [u8; PAGE_SIZE] = [0; PAGE_SIZE];
        if !fault.shared_cache || fault.read_length != PAGE_SIZE {
            return Ok(None);
        }
        let Some(frame) = file_page_cache_lookup(fault.cache_key) else {
            return Ok(None);
        };
        self.install_file_fault_inner(fault, &UNUSED_CACHE_HIT_DATA, flush_local, Some(frame))
            .map(Some)
    }

    pub fn install_file_prefetch(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
    ) -> Result<UserFaultResolution, UserMmRuntimeError> {
        self.install_file_fault_inner(fault, data, false, None)
    }

    fn install_file_fault_inner(
        &self,
        fault: &FileFaultRequest,
        data: &[u8],
        flush_local: bool,
        cached_frame: Option<PhysFrame>,
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
        let mut backing = if let Some(frame) = cached_frame {
            Some(PageBacking::Shared(frame))
        } else if cacheable && let Some(frame) = file_page_cache_lookup(fault.cache_key) {
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
            let (cold, mut hot) = self.lock_both();
            let still_mapped = file_mapping_at(&cold.file_mappings, fault.page)
                .is_some_and(|mapping| mapping.generation == fault.generation);
            if !still_mapped {
                return Ok(InstallOutcome::Unmapped);
            }
            if hot
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?
                .translate(fault.page)?
                .is_some()
            {
                return Ok(InstallOutcome::Spurious);
            }
            let area = cold
                .core
                .layout()
                .find_area(fault.page)
                .ok_or(UserMmRuntimeError::NotMapped)?;
            // Defense in depth: the request-time writability gate can race a
            // concurrent mprotect promotion. A shared cache frame behind a
            // writable VMA would alias every process mapping the same file
            // page, so materialize a private copy before the PTE install.
            if area.flags().is_writable()
                && let Some(PageBacking::Shared(shared_frame)) = &backing
            {
                let allocation = crate::page_alloc::allocate(
                    0,
                    crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
                )?;
                if let Err(error) =
                    copy_physical_page(shared_frame.start_address(), allocation.start().start_address())
                {
                    crate::page_alloc::free(allocation)?;
                    return Err(error);
                }
                backing = Some(PageBacking::Owned(allocation));
            }
            let page =
                VirtPage::from_start_address(fault.page).ok_or(UserMmRuntimeError::InvalidRange)?;
            let request = cold.core.plan_post_install_tlb(fault.page)?;
            if let Err(error) = hot
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
            let previous = hot
                .pages
                .insert(page.start_address().get(), MappedPage { page, backing });
            debug_assert!(previous.is_none());
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
                    backing
                        .take()
                        .expect("spurious file fault lost its backing"),
                )?;
                return Ok(UserFaultResolution::Recovered(UserFaultRecovery::Spurious));
            }
            InstallOutcome::Unmapped => {
                free_uninstalled_backing(
                    backing
                        .take()
                        .expect("unmapped file fault lost its backing"),
                )?;
                return Ok(UserFaultResolution::Fatal(
                    UserFaultFailure::SegmentationViolation,
                ));
            }
        };
        if !flush_local {
            return Ok(UserFaultResolution::Recovered(UserFaultRecovery::Anonymous));
        }

        flush_post_install_local(request)?;
        Ok(UserFaultResolution::Recovered(UserFaultRecovery::Anonymous))
    }

    pub fn unmap_range(&self, range: VirtRange) -> Result<(), UserMmRuntimeError> {
        let retirement = {
            let (mut cold, mut hot) = self.lock_both();
            let old_layout = cold.core.layout().clone();
            let retained_file_mappings = file_mappings_without_range(&cold.file_mappings, range)?;
            cold.core
                .layout_mut()
                .unmap_range(range)
                .map_err(UserMmError::from)?;
            match retire_range_locked(&mut cold, &mut hot, range) {
                Ok(retirement) => {
                    cold.file_mappings = retained_file_mappings;
                    retirement
                }
                Err(error) => {
                    *cold.core.layout_mut() = old_layout;
                    return Err(error);
                }
            }
        };
        self.finish_retirement(retirement)
    }

    /// Drop resident pages from adjacent anonymous VMAs without removing them.
    ///
    /// This implements the observable part of Linux MADV_DONTNEED used by
    /// jemalloc: a later access faults the page back in as zero-filled.  ELF
    /// file mappings are eagerly populated by this kernel and cannot yet be
    /// reconstructed on demand, so callers leave non-anonymous VMAs intact.
    pub fn discard_anonymous_range(&self, range: VirtRange) -> Result<(), UserMmRuntimeError> {
        let retirement = {
            let (mut cold, mut hot) = self.lock_both();
            if file_mappings_overlap(&cold.file_mappings, range) {
                return Ok(());
            }
            let mut cursor = range.start();
            loop {
                let area = cold
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
            retire_range_locked(&mut cold, &mut hot, range)?
        };
        self.finish_retirement(retirement)
    }

    pub fn protect_range(
        &self,
        range: VirtRange,
        access: VmAreaFlags,
    ) -> Result<(), UserMmRuntimeError> {
        let diag = crate::user::BUILDSTORM_DIAGNOSTICS
            && crate::user::BUILDSTORM_SAFE_ACTIVE.load(Ordering::Relaxed);
        let diag_start = if diag {
            Some(crate::arch::time::counter())
        } else {
            None
        };
        // Sub-phase attribution inside the real path: 0 = lock + layout clone,
        // 1 = private-copy promotion, 2 = changed-page walk + VMA/PTE rewrite,
        // 3 = cross-CPU shootdown wait.  Defined here so the marks can live
        // inside the locked block without borrowing diag_prev mutably twice.
        let mut diag_prev = diag_start;
        macro_rules! sub_mark {
            ($sub:expr) => {
                if diag {
                    if let Some(prev) = diag_prev {
                        let now = crate::arch::time::counter();
                        crate::user::BUILDSTORM_MPROTECT_SUBS[$sub]
                            .fetch_add(now.wrapping_sub(prev), Ordering::Relaxed);
                        diag_prev = Some(now);
                    }
                }
            };
        }
        let request = {
            let (mut cold, mut hot) = self.lock_both();

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
                let Some(area) = cold.core.layout().find_area(cursor) else {
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
                if diag {
                    crate::user::BUILDSTORM_MPROTECT_NOOP.fetch_add(1, Ordering::Relaxed);
                }
                return Ok(());
            }

            sub_mark!(0);

            // Cached file pages are shared only while immutable. If userspace
            // promotes such a mapping to writable, materialize a private copy
            // before changing either the VMA or PTE permissions.
            if requested_access.is_writable() {
                // Demand-paged pages of the range that are not resident yet
                // fault in later: demote their registration so those faults
                // materialize private copies instead of installing a shared
                // page-cache frame behind a writable PTE. Demoting the whole
                // overlapping registration is conservative and safe — MAP_PRIVATE
                // pages that were written stay private for the mm's lifetime.
                for mapping in cold.file_mappings.values_mut() {
                    if mapping.shared_cache
                        && mapping.range.end() > range.start()
                        && mapping.range.start() < range.end()
                    {
                        mapping.shared_cache = false;
                    }
                }
                let mut private_copy_keys = Vec::new();
                private_copy_keys
                    .try_reserve(hot.pages.len().min(range.size() / PAGE_SIZE + 1))
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                for (&key, mapping) in hot.pages.range(range.start().get()..range.end().get()) {
                    let must_copy = match &mapping.backing {
                        PageBacking::Shared(_) => true,
                        PageBacking::Cow(frame) => crate::page_alloc::reference_count(*frame)? > 1,
                        PageBacking::Owned(_) => false,
                    };
                    if must_copy {
                        private_copy_keys.push(key);
                    }
                }
                for key in private_copy_keys {
                    let (page, source, old_cow) = {
                        let mapping = hot
                            .pages
                            .get(&key)
                            .expect("writable-promotion page disappeared");
                        (
                            mapping.page,
                            mapping.backing.physical(),
                            match &mapping.backing {
                                PageBacking::Cow(frame) => Some(*frame),
                                _ => None,
                            },
                        )
                    };
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
                    let present = hot
                        .page_table
                        .as_ref()
                        .ok_or(UserMmRuntimeError::NotMapped)?
                        .translate(page.start_address())?
                        .is_some();
                    if present {
                        let area = cold
                            .core
                            .layout()
                            .find_area(page.start_address())
                            .ok_or(UserMmRuntimeError::PermissionDenied)?;
                        let replace = hot
                            .page_table
                            .as_mut()
                            .ok_or(UserMmRuntimeError::NotMapped)?
                            .replace_page(page, allocation.start(), area.mapping_options());
                        if let Err(error) = replace {
                            crate::page_alloc::free(allocation)?;
                            return Err(error.into());
                        }
                    }
                    hot.pages
                        .get_mut(&key)
                        .expect("writable-promotion page disappeared")
                        .backing = PageBacking::Owned(allocation);
                    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
                    if let Some(frame) = old_cow {
                        release_cow_frame(frame)?;
                    }
                }
            }
            sub_mark!(1);

            let mut changed_pages = Vec::new();
            let range_pages = range
                .size()
                .checked_add(PAGE_SIZE - 1)
                .ok_or(UserMmRuntimeError::AddressOverflow)?
                / PAGE_SIZE;
            // Every page that can change is tracked in hot.pages, so at most
            // one entry per tracked page in the range is pushed. Clamp the
            // reserve so a huge mostly demand-paged range does not pay for a
            // pointless multi-megabyte allocation up front.
            changed_pages
                .try_reserve(range_pages.min(hot.pages.len()).min(4096))
                .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

            // mprotect is extremely hot in rustc. Walking every page of the
            // range with a full page-table translate per page made the
            // operation O(range size) even when nearly all pages are
            // demand-paged and absent. Every user-visible PTE install is
            // paired with a hot.pages registration, so the pages that can
            // change are exactly the tracked pages of the range: iterate
            // those. Tracked pages whose leaf is absent (PROT_NONE
            // round-trips, MAP_FIXED leaf retirement) keep the translate-miss
            // fallback and are restored from their retained backing below.
            let page_table = hot
                .page_table
                .as_ref()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            let mut walked_tracked = 0usize;
            for (&key, mapping) in hot.pages.range(range.start().get()..range.end().get()) {
                walked_tracked += 1;
                let page_address = VirtAddr::new(key);
                let page = VirtPage::from_start_address(page_address)
                    .ok_or(UserMmRuntimeError::InvalidRange)?;
                let old_area = cold.core.layout().find_area(page_address);
                let frame = match page_table.translate(page_address)? {
                    Some(physical) => PhysFrame::from_start_address(physical)
                        .ok_or(UserMmRuntimeError::AddressOverflow)?,
                    None => mapping.backing.frame(),
                };
                changed_pages.push((
                    page,
                    old_area.expect("mapped user page has no old VMA"),
                    frame,
                ));
            }

            if diag {
                // Shadow scan: count resident pages the tracked walk cannot
                // see (present PTE without a hot.pages entry). Install/track
                // pairing must keep this at zero; it exists to prove that
                // invariant under load. Timed separately so the walk bucket
                // stays comparable across runs.
                let ab_start = crate::arch::time::counter();
                let mut address = range.start().get();
                while address < range.end().get() {
                    if let Ok(Some(_)) = page_table.translate(VirtAddr::new(address)) {
                        if !hot.pages.contains_key(&address) {
                            crate::user::BUILDSTORM_MPROTECT_AB
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    match address.checked_add(PAGE_SIZE) {
                        Some(next) => address = next,
                        None => break,
                    }
                }
                crate::user::BUILDSTORM_MPROTECT_AB_CYCLES.fetch_add(
                    crate::arch::time::counter().wrapping_sub(ab_start),
                    Ordering::Relaxed,
                );
                crate::user::BUILDSTORM_MPROTECT_WALKED
                    .fetch_add(walked_tracked as u64, Ordering::Relaxed);
                crate::user::BUILDSTORM_MPROTECT_CHANGED
                    .fetch_add(changed_pages.len() as u64, Ordering::Relaxed);
            }

            // Snapshot the pre-mutation layout only when a rollback can
            // happen. VmAreaSet::protect_range is atomic on failure (a failed
            // call leaves the layout untouched), and the PTE-apply rollback
            // below can only run when the walk found pages to rewrite. Calls
            // that change nothing resident skip the full VMA-table clone.
            let old_layout = if changed_pages.is_empty() {
                None
            } else {
                Some(cold.core.layout().clone())
            };
            let request = if changed_pages.is_empty() {
                None
            } else {
                Some(cold.core.plan_tlb_request(TlbFlush::Range {
                    scope: TlbScope::AddressSpace(cold.core.asid().id()),
                    range,
                })?)
            };

            // Try the full VMA-splitting protect first.
            let layout_result = cold.core.layout_mut().protect_range(range, access);

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
                if let Some(page_table) = hot.page_table.as_mut() {
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
                    if let Some(page_table) = hot.page_table.as_mut() {
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
                    if let Some(snapshot) = old_layout {
                        *cold.core.layout_mut() = snapshot;
                    }
                    return Err(UserMmError::from(error).into());
                }

                let mut updated_pages = Vec::new();
                updated_pages
                    .try_reserve(changed_pages.len())
                    .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
                let result: Result<(), RuntimePageTableError> = (|| {
                    for (page, _, frame) in &changed_pages {
                        let area = cold
                            .core
                            .layout()
                            .find_area(page.start_address())
                            .expect("mprotect removed a mapped page's VMA");
                        let page_table = hot
                            .page_table
                            .as_mut()
                            .ok_or(RuntimePageTableError::NotMapped)?;
                        apply_page_protection(page_table, *page, *frame, area)?;
                        updated_pages.push(*page);
                    }
                    Ok(())
                })();

                if let Err(error) = result {
                    let page_table = hot
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
                    if let Some(snapshot) = old_layout {
                        *cold.core.layout_mut() = snapshot;
                    }
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
        sub_mark!(2);
        if let Some(request) = request {
            self.shootdown_user_request(request);
        }
        sub_mark!(3);
        if diag {
            crate::user::BUILDSTORM_MPROTECT_REAL.fetch_add(1, Ordering::Relaxed);
            if let Some(start) = diag_start {
                let now = crate::arch::time::counter();
                crate::user::BUILDSTORM_MPROTECT_REAL_CYCLES
                    .fetch_add(now.wrapping_sub(start), Ordering::Relaxed);
            }
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
            let (mut cold, mut hot) = self.lock_both();
            if let Some(cow) = resolve_cow_write_fault_locked(&mut cold, &mut hot, address, access)? {
                cow
            } else {
                let present = match hot
                    .page_table
                    .as_ref()
                    .ok_or(UserMmRuntimeError::NotMapped)?
                    .translate(address)
                {
                    Ok(mapping) => mapping.is_some(),
                    // A non-canonical user address is a SIGSEGV, not a kernel
                    // error. Report it absent so the fault planner classifies
                    // it as a segmentation violation instead of panicking.
                    Err(RuntimePageTableError::InvalidVirtualAddress) => false,
                    Err(error) => return Err(error.into()),
                };
                let fault = PageFault::new(address, access, FaultSource::User, present);
                match cold.core.plan_user_fault(fault, user_sp)? {
                    UserFaultPlan::MapAnonymous { area, page } => {
                        let request = cold.core.plan_post_install_tlb(page)?;
                        map_zero_page_locked(&mut hot, area, page)?;
                        if access == FaultAccess::Write
                            && matches!(
                                area.kind(),
                                myos_mm::VmAreaKind::Anonymous | myos_mm::VmAreaKind::Heap
                            )
                        {
                            // Rustc grows bump arenas and Vec allocations mostly
                            // forward. Populate one 64-KiB folio-sized run under
                            // the existing MM lock and use the demanded page's
                            // single TLB request as the publication barrier.
                            for index in 1..ANONYMOUS_FAULT_CLUSTER_PAGES {
                                let Some(next) = page.checked_add(index * PAGE_SIZE) else {
                                    break;
                                };
                                if !area.range().contains(next) {
                                    break;
                                }
                                if map_zero_page_locked(&mut hot, area, next).is_err() {
                                    break;
                                }
                            }
                        }
                        (
                            UserFaultResolution::Recovered(UserFaultRecovery::Anonymous),
                            Some(request),
                        )
                    }
                    UserFaultPlan::GrowStack { growth } => {
                        let request = cold.core.plan_post_install_tlb(growth.fault_page())?;
                        cold.core.commit_stack_growth(growth)?;
                        if let Err(error) =
                            map_zero_page_locked(&mut hot, growth.new_area(), growth.fault_page())
                        {
                            let removed = cold
                                .core
                                .unmap_exact(growth.new_area().range())
                                .expect("stack-growth rollback lost the expanded VMA");
                            assert_eq!(removed, growth.new_area());
                            cold.core
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
                        let request = cold.core.plan_post_install_tlb(address)?;
                        (
                            UserFaultResolution::Recovered(UserFaultRecovery::Spurious),
                            Some(request),
                        )
                    }
                    UserFaultPlan::RestoreWrite { area } => {
                        // A write fault on a present read-only leaf inside a
                        // writable COW-flagged VMA: repair the leaf's write
                        // permission in place. The refcount-driven COW resolver
                        // above already owns Cow backings; this arm serves the
                        // planner's COW-flag classification. Owned backings are
                        // exclusive by invariant, so no page copy is needed.
                        let page_address = address
                            .align_down(PAGE_SIZE)
                            .ok_or(UserMmRuntimeError::AddressOverflow)?;
                        let page = VirtPage::from_start_address(page_address)
                            .ok_or(UserMmRuntimeError::InvalidRange)?;
                        let physical = hot
                            .page_table
                            .as_ref()
                            .ok_or(UserMmRuntimeError::NotMapped)?
                            .translate(page_address)?
                            .ok_or(UserMmRuntimeError::NotMapped)?;
                        let frame = PhysFrame::from_start_address(physical)
                            .ok_or(UserMmRuntimeError::AddressOverflow)?;
                        apply_page_protection(
                            hot.page_table
                                .as_mut()
                                .ok_or(UserMmRuntimeError::NotMapped)?,
                            page,
                            frame,
                            area,
                        )?;
                        let request = cold.core.plan_post_install_tlb(address)?;
                        (
                            UserFaultResolution::Recovered(UserFaultRecovery::Spurious),
                            Some(request),
                        )
                    }
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
            }
        };

        if let Some(request) = request {
            // This path only installs a previously invalid leaf or repairs a
            // spurious local translation; no page is unmapped, freed, or
            // permission-revoked. Restrict the post-install request to this
            // CPU: another CPU using the same mm either observes the new valid
            // PTE directly or faults on its own stale invalid translation and
            // performs the same local recovery. munmap/mprotect/retirement
            // still retain the original full active_cpus mask and use
            // shootdown_user().
            //
            // §10 情况A: fault resolution runs with interrupts enabled
            // (SyscallInterruptGuard in handle_fault), so the flush helper
            // temporarily disables interrupts around the local-only request
            // instead of assuming the trap-entry IRQ state.
            flush_post_install_local(request)?;
            // post-install fault recovery is local-only by construction
        }
        Ok(resolution)
    }

    /// Installs the private root and publishes this CPU after synchronizing
    /// only generations invalidated while this mm was inactive locally.
    pub fn activate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();

        // SUDOOS_ASID_SCOPE_V1
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
            let token = self.tlb.asid();
            if !token.is_current(current_asid_generation) {
                return Err(UserMmError::AsidMismatch.into());
            }
            let tlb_generation = self.tlb.tlb_generation();

            // SAFETY: Process owns the root and shared kernel tables, while the
            // scheduler's loaded_mm Arc pins this UserMm across installation and
            // the complete interval in which this CPU can execute the user task.
            unsafe {
                crate::vm::activate_user_page_table(self.root, token.id());
            }
            // QEMU's LoongArch targeted invalidations have proven unreliable
            // (see arch flush_asid): a generation-gated skip left stale
            // translations reachable across a switch and corrupted guest
            // heaps (cagent factorial abort: "unaligned tcache chunk",
            // fs-create segv). Address-space entry remains a correctness
            // boundary on LoongArch, so it invalidates the complete local
            // TLB unconditionally; RISC-V keeps the generation-gated ASID
            // path. §11 still removes the redundant departure flush.
            #[cfg(target_arch = "loongarch64")]
            crate::arch::memory::paging::flush_asid(token.id());
            #[cfg(target_arch = "riscv64")]
            if self.local_tlb_generation[cpu].load(Ordering::Acquire) != tlb_generation {
                crate::arch::memory::paging::flush_asid(token.id());
                self.local_tlb_generation[cpu].store(tlb_generation, Ordering::Release);
            }

            match self
                .tlb
                .enter_cpu_after_local_sync(cpu, current_asid_generation, tlb_generation)
            {
                Ok(()) => return Ok(()),
                Err(UserMmError::TlbGenerationMismatch { .. }) => {
                    crate::arch::memory::paging::flush_asid(token.id());
                    self.local_tlb_generation[cpu]
                        .store(tlb_generation.wrapping_add(1), Ordering::Release);
                }
                Err(error) => {
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

    /// Restores the kernel root and clears this CPU from the mm's active mask.
    /// A stable generation snapshot proves that any destructive change which
    /// raced this departure either flushed us while active or will advance the
    /// generation and force a flush on the next activation.
    pub fn deactivate_current_cpu(&self) -> Result<(), UserMmRuntimeError> {
        crate::context::assert_interrupts_disabled();
        let cpu = crate::smp::current_cpu_id().get();
        let token = self.asid();

        // SAFETY: KERNEL_PAGE_TABLE permanently owns this root.
        unsafe {
            crate::vm::activate_kernel_page_table()?;
        }
        // §11: departure no longer flushes. The generation stored below
        // records the state this CPU's TLB reflects for this mm; activation
        // re-flushes only when the generation has advanced (and every
        // destructive change's shootdown flushes through the full arch
        // primitive on both arches), so stale entries can never survive a
        // re-entry, and an unchanged generation keeps the TLB warm.

        loop {
            let generation = self.tlb.tlb_generation();
            match self.tlb.leave_cpu_after_local_flush(cpu, generation) {
                Ok(()) => {
                    self.local_tlb_generation[cpu].store(generation, Ordering::Release);
                    break;
                }
                Err(UserMmError::TlbGenerationMismatch { .. }) => {
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
        let root = self.root;
        let asid = self.tlb.asid().id();
        let active = self.tlb.active_cpus();
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
        assert!(
            active.count() >= 1,
            "M8-B3 published an unexpected CPU mask"
        );
        let current_is_active = active.contains(cpu).map_err(UserMmError::from)?;
        assert!(
            current_is_active,
            "M8-B3 did not publish the current CPU in active_cpus",
        );
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<(), UserMmRuntimeError> {
        let diag = crate::user::BUILDSTORM_DIAGNOSTICS
            && crate::user::BUILDSTORM_SAFE_ACTIVE.load(Ordering::Relaxed);
        let diag_start = if diag {
            Some(crate::arch::time::counter())
        } else {
            None
        };
        // P0-2B: destroy(&mut self) already proves exclusive ownership, so
        // teardown takes the payloads directly instead of acquiring the
        // runtime cold/hot spin locks. Final destruction must never become
        // a hidden cross-CPU lock acquisition.
        let cold = self.cold.get_mut();
        let hot = self.hot.get_mut();
        cold.core.assert_inactive_for_destroy()?;
        let table_capacity = hot
            .page_table
            .as_ref()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .allocated_runtime_tables();
        let mut retired = Vec::new();
        retired
            .try_reserve(table_capacity)
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;

        // §8 retirement model applied to final teardown: collect every backing
        // inside the lock, then free in batches once the page-table walk is
        // done.  Exec teardown retires tens of thousands of pages per exec and
        // per-page global allocator lock churn (two to three acquisitions each,
        // with seven other CPUs faulting pages in at the same time) made this
        // the hottest buildstorm exec phase.  The per-page translate pre-check
        // and reclaim_empty_tables walk are dropped as well: unmap_page's
        // NotMapped error covers the already-unmapped leaf case, and
        // retire_all_private_tables below releases every intermediate table in
        // a single pass.
        let mut owned_retired: Vec<PageAllocation> = Vec::new();
        owned_retired
            .try_reserve(hot.pages.len())
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        let mut cow_retired: Vec<PhysFrame> = Vec::new();
        cow_retired
            .try_reserve(hot.pages.len())
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        let mut shared_retired = 0_usize;

        let mut already_unmapped = 0_usize;
        while let Some((_key, mapping)) = hot.pages.pop_first() {
            let page_table = hot
                .page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?;
            match page_table.unmap_page(mapping.page) {
                Ok(frame) => {
                    assert_eq!(
                        frame,
                        mapping.backing.frame(),
                        "M8-B3 user leaf returned a different physical frame",
                    );
                }
                // MAP_FIXED/munmap may retire the leaf before the final
                // owner reaches process teardown. The backing remains
                // uniquely owned by this record and still must be freed.
                Err(RuntimePageTableError::NotMapped) => {
                    already_unmapped += 1;
                }
                Err(error) => return Err(error.into()),
            }
            match mapping.backing {
                PageBacking::Owned(allocation) => owned_retired.push(allocation),
                PageBacking::Cow(frame) => cow_retired.push(frame),
                PageBacking::Shared(_) => shared_retired += 1,
            }
        }

        if already_unmapped != 0 && crate::user::oscomp_verbose_user_trace_active() {
            crate::println!(
                "user-mm: reclaimed {} backing pages whose leaves were already unmapped",
                already_unmapped,
            );
        }

        hot.page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .retire_all_private_tables(&mut retired)?;

        // Free the backings in batches: one allocator lock acquisition per
        // phase instead of two per page.
        let owned_count = owned_retired.len();
        if let Err(error) = crate::page_alloc::free_many(owned_retired) {
            crate::println!(
                "user-mm: batched backing free failed owned={} error={:?}",
                owned_count,
                error,
            );
            return Err(error.into());
        }
        LIVE_BACKINGS.fetch_sub(owned_count, Ordering::AcqRel);

        let mut freed_scratch: Vec<PageAllocation> = Vec::new();
        freed_scratch
            .try_reserve(cow_retired.len())
            .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
        let cow_freed =
            crate::page_alloc::release_many_unreferenced(&cow_retired, &mut freed_scratch)?;
        LIVE_BACKINGS.fetch_sub(cow_freed, Ordering::AcqRel);

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

        let page_table = hot
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?;
        assert_eq!(
            page_table.allocated_runtime_tables(),
            0,
            "M8-B3 retained private intermediate page tables",
        );
        page_table.release_empty()?;
        hot.page_table = None;
        LIVE_ROOTS.fetch_sub(1, Ordering::AcqRel);
        let asid = cold.core.asid();
        // No guards to drop: cold/hot are plain `get_mut` borrows that end
        // here (NLL).
        release_mm_reservation(asid);
        if let Some(start) = diag_start {
            let elapsed = crate::arch::time::counter().wrapping_sub(start);
            crate::user::BUILDSTORM_DESTROY_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::user::BUILDSTORM_DESTROY_CYCLES.fetch_add(elapsed, Ordering::Relaxed);
            crate::user::BUILDSTORM_DESTROY_PAGES.fetch_add(
                (owned_count + cow_retired.len() + shared_retired) as u64,
                Ordering::Relaxed,
            );
            crate::user::BUILDSTORM_DESTROY_TABLES
                .fetch_add(table_capacity as u64, Ordering::Relaxed);
            crate::user::BUILDSTORM_DESTROY_UNMAPPED
                .fetch_add(already_unmapped as u64, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Frees the pages and page tables a retirement batch collected, after
    /// every target CPU has acknowledged the accompanying TLB request.
    fn finish_retirement(&self, retirement: RetirementBatch) -> Result<(), UserMmRuntimeError> {
        if let Some(request) = retirement.request {
            self.shootdown_user_request(request);
        }
        for backing in retirement.backings {
            crate::page_alloc::free(backing)?;
            LIVE_BACKINGS.fetch_sub(1, Ordering::AcqRel);
        }
        for frame in retirement.cow_frames {
            release_cow_frame(frame)?;
        }
        for table in retirement.page_tables {
            crate::page_alloc::free(table)?;
        }
        Ok(())
    }

    /// Executes one per-mm TLB request on behalf of `self`, passing the mm's
    /// per-CPU local-generation array so the §9 IPI handler can publish the
    /// seen generation for §11 switch-in flush elision.
    fn shootdown_user_request(&self, request: myos_mm::PerMmTlbRequest) {
        if crate::arch::interrupt::are_disabled() || !crate::task::scheduler_is_initialized() {
            crate::tlb::shootdown_user_local(request);
        } else {
            crate::tlb::shootdown_user(request, Some(self.local_tlb_generation.as_ptr()));
        }
    }
}

impl Drop for UserMm {
    fn drop(&mut self) {
        // P0-2C: &mut self already proves no concurrent locker exists —
        // the final Arc drop must not acquire the runtime cold/hot spin
        // locks. That made MM final destruction a hidden CrossCpu lock
        // acquisition inside SCHEDULER/IRQ-off switch contexts (the
        // BuildStorm self-deadlock entry point).
        let needs_teardown = self.hot.get_mut().page_table.is_some();
        if needs_teardown {
            if let Err(error) = self.destroy() {
                panic!("M8-B3 UserMm teardown failed during drop: {error:?}");
            }
        }
        let cold = self.cold.get_mut();
        let hot = self.hot.get_mut();
        assert!(
            hot.pages.is_empty(),
            "M8-B3 UserMm dropped with owned backing pages",
        );
        assert!(
            cold.core.assert_inactive_for_destroy().is_ok(),
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

fn resolve_cow_write_fault_locked(
    cold: &mut UserMmColdState,
    hot: &mut UserMmHotState,
    address: VirtAddr,
    access: FaultAccess,
) -> Result<Option<(UserFaultResolution, Option<PerMmTlbRequest>)>, UserMmRuntimeError> {
    if access != FaultAccess::Write {
        return Ok(None);
    }
    let page_address = address
        .align_down(PAGE_SIZE)
        .ok_or(UserMmRuntimeError::AddressOverflow)?;
    let page =
        VirtPage::from_start_address(page_address).ok_or(UserMmRuntimeError::InvalidRange)?;
    let frame = match hot.pages.get(&page_address.get()) {
        Some(MappedPage {
            backing: PageBacking::Cow(frame),
            ..
        }) => *frame,
        _ => return Ok(None),
    };
    let area = cold
        .core
        .layout()
        .find_area(page_address)
        .ok_or(UserMmRuntimeError::NotMapped)?;
    let pte_present = match hot
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?
        .translate(page_address)
    {
        Ok(mapping) => mapping.is_some(),
        // A non-canonical user address is a SIGSEGV, not a kernel error.
        Err(RuntimePageTableError::InvalidVirtualAddress) => false,
        Err(error) => return Err(error.into()),
    };
    if !area.flags().is_writable() || !pte_present {
        return Ok(None);
    }

    let request = cold.core.plan_post_install_tlb(page_address)?;
    let references = crate::page_alloc::reference_count(frame)?;
    if references > 1 {
        let allocation = crate::page_alloc::allocate(
            0,
            crate::page_alloc::PageAllocationOptions::kernel_zeroed(),
        )?;
        if let Err(error) =
            copy_physical_page(frame.start_address(), allocation.start().start_address())
        {
            crate::page_alloc::free(allocation)?;
            return Err(error);
        }
        let replaced = match hot
            .page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .replace_page(page, allocation.start(), area.mapping_options())
        {
            Ok(replaced) => replaced,
            Err(error) => {
                crate::page_alloc::free(allocation)?;
                return Err(error.into());
            }
        };
        debug_assert_eq!(replaced, frame);
        hot.pages
            .get_mut(&page_address.get())
            .expect("COW page disappeared while user_mm was locked")
            .backing = PageBacking::Owned(allocation);
        LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
        release_cow_frame(frame)?;
    } else if references == 1 {
        apply_page_protection(
            hot.page_table
                .as_mut()
                .ok_or(UserMmRuntimeError::NotMapped)?,
            page,
            frame,
            area,
        )?;
    } else {
        return Err(UserMmRuntimeError::NotMapped);
    }

    Ok(Some((
        UserFaultResolution::Recovered(UserFaultRecovery::Anonymous),
        Some(request),
    )))
}

fn map_zero_page_locked(
    hot: &mut UserMmHotState,
    area: VmArea,
    address: VirtAddr,
) -> Result<PhysAddr, UserMmRuntimeError> {
    let offset = address.get() & (PAGE_SIZE - 1);
    let page_address = address
        .align_down(PAGE_SIZE)
        .ok_or(UserMmRuntimeError::AddressOverflow)?;
    let page =
        VirtPage::from_start_address(page_address).ok_or(UserMmRuntimeError::InvalidRange)?;
    if let Some(physical) = hot
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?
        .translate(address)?
    {
        return Ok(physical);
    }

    // A fork-Cow-ified page without a PTE keeps its frame: map the existing
    // frame read-only (content = the parent's fork snapshot) instead of a
    // fresh zero page. The entry stays Cow; the next write fault breaks COW
    // through the resolver.
    let cow_frame = match hot.pages.get(&page.start_address().get()) {
        Some(MappedPage {
            backing: PageBacking::Cow(frame),
            ..
        }) => Some(*frame),
        _ => None,
    };
    if let Some(frame) = cow_frame {
        let mapped_area = if area.flags().is_writable() {
            VmArea::new(
                area.range(),
                area.flags().without(VmAreaFlags::WRITE),
                area.kind(),
            )
        } else {
            area
        };
        let physical = frame
            .start_address()
            .checked_add(offset)
            .ok_or(UserMmRuntimeError::AddressOverflow)?;
        hot.page_table
            .as_mut()
            .ok_or(UserMmRuntimeError::NotMapped)?
            .map_page(page, frame, mapped_area.mapping_options())?;
        return Ok(physical);
    }

    let backing =
        crate::page_alloc::allocate(0, crate::page_alloc::PageAllocationOptions::kernel_zeroed())?;
    let physical = match backing.start().start_address().checked_add(offset) {
        Some(physical) => physical,
        None => {
            crate::page_alloc::free(backing)?;
            return Err(UserMmRuntimeError::AddressOverflow);
        }
    };
    let page_table = hot
        .page_table
        .as_mut()
        .ok_or(UserMmRuntimeError::NotMapped)?;
    if let Err(error) = page_table.map_page(page, backing.start(), area.mapping_options()) {
        crate::page_alloc::free(backing)?;
        return Err(error.into());
    }

    let previous = hot.pages.insert(page.start_address().get(), MappedPage {
        page,
        backing: PageBacking::Owned(backing),
    });
    debug_assert!(previous.is_none());
    LIVE_BACKINGS.fetch_add(1, Ordering::AcqRel);
    Ok(physical)
}

fn file_mappings_without_range(
    mappings: &BTreeMap<usize, FileBackedMapping>,
    removed: VirtRange,
) -> Result<BTreeMap<usize, FileBackedMapping>, UserMmRuntimeError> {
    let mut retained = BTreeMap::new();
    for mapping in mappings.values() {
        if !mapping.range.overlaps(removed) {
            retained.insert(mapping.range.start().get(), mapping.clone());
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
            retained.insert(fragment.range.start().get(), fragment);
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
            retained.insert(fragment.range.start().get(), fragment);
        }
    }
    Ok(retained)
}

fn file_mapping_at(
    mappings: &BTreeMap<usize, FileBackedMapping>,
    address: VirtAddr,
) -> Option<&FileBackedMapping> {
    mappings
        .range(..=address.get())
        .next_back()
        .map(|(_, mapping)| mapping)
        .filter(|mapping| mapping.range.contains(address))
}

fn file_mappings_overlap(mappings: &BTreeMap<usize, FileBackedMapping>, range: VirtRange) -> bool {
    if file_mapping_at(mappings, range.start()).is_some() {
        return true;
    }
    mappings
        .range(range.start().get()..range.end().get())
        .next()
        .is_some()
}

fn retire_range_locked(
    cold: &mut UserMmColdState,
    hot: &mut UserMmHotState,
    range: VirtRange,
) -> Result<RetirementBatch, UserMmRuntimeError> {
    let mut keys = Vec::new();
    keys.try_reserve(range.size() / PAGE_SIZE + 1)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    keys.extend(
        hot.pages
            .range(range.start().get()..range.end().get())
            .map(|(&key, _)| key),
    );
    let count = keys.len();
    if count == 0 {
        return Ok(RetirementBatch::empty());
    }

    let page_table = hot
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?;
    for key in &keys {
        let mapping = hot
            .pages
            .get(key)
            .expect("retirement key disappeared during preflight");
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
    let mut cow_frames = Vec::new();
    cow_frames
        .try_reserve(count)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    let mut tables = Vec::new();
    tables
        .try_reserve(table_capacity)
        .map_err(|_| UserMmRuntimeError::MetadataOutOfMemory)?;
    let request = cold.core.plan_tlb_request(TlbFlush::Range {
        scope: TlbScope::AddressSpace(cold.core.asid().id()),
        range,
    })?;

    for key in keys {
        let mapping = hot
            .pages
            .remove(&key)
            .expect("retirement key disappeared after preflight");
        let page_table = hot
            .page_table
            .as_mut()
            .expect("retirement preflight lost the user page table");
        if page_table
            .translate(mapping.page.start_address())?
            .is_some()
        {
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
        match mapping.backing {
            PageBacking::Owned(allocation) => backings.push(allocation),
            PageBacking::Cow(frame) => cow_frames.push(frame),
            PageBacking::Shared(_) => {}
        }
    }

    Ok(RetirementBatch {
        request: Some(request),
        backings,
        cow_frames,
        page_tables: tables,
    })
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

/// Publish newly valid user leaves on the faulting CPU. New mappings do not
/// revoke access or free memory, so remote CPUs may recover independently.
fn flush_post_install_local(request: PerMmTlbRequest) -> Result<(), UserMmRuntimeError> {
    let restore_enabled = !crate::arch::interrupt::are_disabled();
    if restore_enabled {
        crate::arch::interrupt::disable();
    }
    let request = match request.local_only(crate::smp::current_cpu_id().get()) {
        Ok(request) => request,
        Err(error) => {
            if restore_enabled {
                // SAFETY: restore the entry state before propagating failure.
                unsafe { crate::arch::interrupt::enable() };
            }
            return Err(UserMmRuntimeError::from(error));
        }
    };
    crate::tlb::shootdown_user_local(request);
    if restore_enabled {
        // SAFETY: restores the enabled state observed above.
        unsafe { crate::arch::interrupt::enable() };
    }
    Ok(())
}

fn file_fault_request_locked(
    cold: &UserMmColdState,
    hot: &UserMmHotState,
    address: VirtAddr,
    access: FaultAccess,
) -> Result<Option<FileFaultRequest>, UserMmRuntimeError> {
    let Some(mapping) = file_mapping_at(&cold.file_mappings, address) else {
        return Ok(None);
    };
    let area = cold
        .core
        .layout()
        .find_area(address)
        .ok_or(UserMmRuntimeError::NotMapped)?;
    let allowed = match access {
        FaultAccess::Read => area.flags().is_readable(),
        FaultAccess::Write => area.flags().is_writable(),
        FaultAccess::Execute => area.flags().is_executable(),
    };
    let pte_present = match hot
        .page_table
        .as_ref()
        .ok_or(UserMmRuntimeError::NotMapped)?
        .translate(address)
    {
        Ok(mapping) => mapping.is_some(),
        // A non-canonical user address is a SIGSEGV, not a kernel error.
        Err(RuntimePageTableError::InvalidVirtualAddress) => false,
        Err(error) => return Err(error.into()),
    };
    if !allowed || pte_present {
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
        cache_key: (
            mapping.device,
            mapping.inode,
            mapping.content_generation,
            file_offset,
        ),
        // A shared page-cache frame behind a writable VMA would alias every
        // process mapping the same file page: user writes would land in the
        // globally shared frame. Writable areas must fault private copies.
        shared_cache: mapping.shared_cache && !area.flags().is_writable(),
        generation: mapping.generation,
    }))
}

fn file_page_cache_shard(key: FilePageCacheKey) -> usize {
    let mut hash = key.0
        ^ key.1.rotate_left(13)
        ^ key.2.rotate_left(29)
        ^ (key.3 / PAGE_SIZE as u64).rotate_left(47);
    // SplitMix64 finalizer: file offsets are page-aligned and therefore have
    // zero low bits; using those low bits directly collapsed a large ELF into
    // one shard. Avalanche every input bit before choosing a shard.
    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    hash as usize & (FILE_PAGE_CACHE_SHARDS - 1)
}

fn file_page_cache_lookup(key: FilePageCacheKey) -> Option<PhysFrame> {
    let shard = file_page_cache_shard(key);
    let result = FILE_PAGE_CACHE[shard]
        .lock()
        .get(&key)
        .map(PageAllocation::start);
    result
}

/// Retain versioned file pages for the kernel lifetime. The generation in the
/// key gives writers Linux-like invalidation semantics without freeing frames
/// that are still mapped by an older process. The fixed 1 GiB ceiling bounds
/// pinning under the 8 GiB BuildStorm configuration.
fn file_page_cache_install(
    key: FilePageCacheKey,
    allocation: PageAllocation,
) -> (PageBacking, Option<PageAllocation>) {
    let shard = file_page_cache_shard(key);
    let mut cache = FILE_PAGE_CACHE[shard].lock();
    if let Some(existing) = cache.get(&key) {
        return (PageBacking::Shared(existing.start()), Some(allocation));
    }
    if cache.len() >= FILE_PAGE_CACHE_CAPACITY_PER_SHARD {
        return (PageBacking::Owned(allocation), None);
    }
    let frame = allocation.start();
    cache.insert(key, allocation);
    (PageBacking::Shared(frame), None)
}

fn free_uninstalled_backing(backing: PageBacking) -> Result<(), UserMmRuntimeError> {
    match backing {
        PageBacking::Owned(allocation) => crate::page_alloc::free(allocation)?,
        PageBacking::Cow(frame) => release_cow_frame(frame)?,
        PageBacking::Shared(_) => {}
    }
    Ok(())
}

fn release_cow_frame(frame: PhysFrame) -> Result<(), UserMmRuntimeError> {
    if crate::page_alloc::decrement_reference(frame)? == 0 {
        crate::page_alloc::free_unreferenced_frame(frame)?;
        LIVE_BACKINGS.fetch_sub(1, Ordering::AcqRel);
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
    cold: &UserMmColdState,
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
        let area = cold
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
