use alloc::{sync::Arc, vec::Vec};
use core::{
    ptr::NonNull,
    sync::atomic::{Ordering, compiler_fence},
};

use myos_mm::{PAGE_SIZE, PageAllocation, PhysAddr, VirtAddr};
use virtio_drivers::{
    BufferDirection, Hal,
    device::blk::{BlkReq, BlkResp, SECTOR_SIZE, VirtIOBlk},
    transport::{
        DeviceType, Transport,
        mmio::{MmioTransport, VirtIOHeader},
        pci::{
            PciTransport,
            bus::{
                BarInfo, Cam, Command, ConfigurationAccess, DeviceFunction, MemoryBarType, MmioCam,
                PciRoot,
            },
            virtio_device_type,
        },
    },
};

use crate::{
    block::{self, BlockDevice, BlockError},
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    net::NetDevice,
    page_alloc::{self, PageAllocationOptions},
};

const MAX_MMIO_REGIONS: usize = 32;
const MAX_PCI_HOSTS: usize = 8;
const DMA32_LIMIT: u64 = 0x1_0000_0000;

const DMA_LOCK: LockClass = LockClass::new("virtio.dma", LockRank::Vfs, 1);
const BLK_LOCK: LockClass = LockClass::new("virtio.blk", LockRank::Vfs, 21);

static DMA_ALLOCATIONS: IrqSpinLock<Vec<DmaAllocation>> =
    IrqSpinLock::new_with_class(Vec::new(), DMA_LOCK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioRegion {
    base: usize,
    size: usize,
}

impl MmioRegion {
    pub const fn new(base: usize, size: usize) -> Self {
        Self { base, size }
    }

    pub const fn base(self) -> usize {
        self.base
    }

    pub const fn size(self) -> usize {
        self.size
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioRegions {
    entries: [Option<MmioRegion>; MAX_MMIO_REGIONS],
    len: usize,
    overflow: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciHostBridge {
    name: &'static str,
    ecam: myos_mm::PhysRange,
    mem32: myos_mm::PhysRange,
    first_bus: u8,
    last_bus: u8,
}

impl PciHostBridge {
    pub fn new(
        name: &str,
        ecam: myos_fdt::MemoryRegion,
        mem32: myos_fdt::MemoryRegion,
        first_bus: u8,
        last_bus: u8,
    ) -> Self {
        let name = if name == "pcie" { "pcie" } else { "pci" };
        Self {
            name,
            ecam: myos_mm::PhysRange::from_start_size(PhysAddr::new(ecam.start()), ecam.size())
                .expect("FDT PCI ECAM range overflowed while collecting host bridges"),
            mem32: myos_mm::PhysRange::from_start_size(PhysAddr::new(mem32.start()), mem32.size())
                .expect("FDT PCI MEM32 range overflowed while collecting host bridges"),
            first_bus,
            last_bus,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }

    pub const fn ecam(self) -> myos_mm::PhysRange {
        self.ecam
    }

    pub const fn mem32(self) -> myos_mm::PhysRange {
        self.mem32
    }

    pub const fn first_bus(self) -> u8 {
        self.first_bus
    }

    pub const fn last_bus(self) -> u8 {
        self.last_bus
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PciHostBridges {
    entries: [Option<PciHostBridge>; MAX_PCI_HOSTS],
    len: usize,
    overflow: usize,
}

impl PciHostBridges {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_PCI_HOSTS],
            len: 0,
            overflow: 0,
        }
    }

    pub fn push(&mut self, host: PciHostBridge) {
        if self.len < self.entries.len() {
            self.entries[self.len] = Some(host);
            self.len += 1;
        } else {
            self.overflow += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = PciHostBridge> + '_ {
        self.entries[..self.len].iter().filter_map(|entry| *entry)
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn overflow(self) -> usize {
        self.overflow
    }
}

impl MmioRegions {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_MMIO_REGIONS],
            len: 0,
            overflow: 0,
        }
    }

    pub fn push(&mut self, region: MmioRegion) {
        if self.len < self.entries.len() {
            self.entries[self.len] = Some(region);
            self.len += 1;
        } else {
            self.overflow += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = MmioRegion> + '_ {
        self.entries[..self.len].iter().filter_map(|entry| *entry)
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn overflow(self) -> usize {
        self.overflow
    }
}

struct DmaAllocation {
    paddr: u64,
    vaddr: usize,
    allocation: PageAllocation,
}

pub struct SudoHal;

// SAFETY: DMA buffers are allocated as private, contiguous, zeroed physical
// pages from the DMA32 zone, tracked until virtio-drivers returns the exact
// allocation tuple to dma_dealloc. QEMU virt machines are coherent for these
// MMIO transports, so share/unshare can use direct physical translations.
// Compiler fences document the DMA ordering boundary around descriptor-visible
// memory: the transport driver still owns the device-specific MMIO barriers.
unsafe impl Hal for SudoHal {
    fn dma_alloc(
        pages: usize,
        _direction: BufferDirection,
        _access_platform: bool,
    ) -> (virtio_drivers::PhysAddr, NonNull<u8>) {
        if pages == 0 {
            return (0, NonNull::dangling());
        }

        let Some(rounded_pages) = pages.checked_next_power_of_two() else {
            return (0, NonNull::dangling());
        };
        let order = rounded_pages.trailing_zeros() as usize;

        let allocation = match page_alloc::allocate(order, PageAllocationOptions::dma32_zeroed()) {
            Ok(allocation) => allocation,
            Err(error) => {
                crate::println!("virtio: DMA allocation failed: {error:?}");
                return (0, NonNull::dangling());
            }
        };

        let paddr = allocation.range().start().get() as u64;
        if paddr
            .checked_add(allocation.size() as u64)
            .is_none_or(|end| end > DMA32_LIMIT)
        {
            crate::println!("virtio: DMA allocation escaped DMA32 zone");
            let _ = page_alloc::free(allocation);
            return (0, NonNull::dangling());
        }

        let pointer =
            match crate::arch::memory::phys_access::ram_mut_ptr::<u8>(allocation.range().start()) {
                Ok(pointer) => pointer,
                Err(error) => {
                    crate::println!("virtio: DMA allocation is not CPU-accessible: {error:?}");
                    let _ = page_alloc::free(allocation);
                    return (0, NonNull::dangling());
                }
            };

        let Some(vaddr) = NonNull::new(pointer) else {
            let _ = page_alloc::free(allocation);
            return (0, NonNull::dangling());
        };

        {
            let mut allocations = DMA_ALLOCATIONS.lock();
            if allocations.try_reserve(1).is_err() {
                drop(allocations);
                let _ = page_alloc::free(allocation);
                return (0, NonNull::dangling());
            }

            allocations.push(DmaAllocation {
                paddr,
                vaddr: vaddr.as_ptr() as usize,
                allocation,
            });
        }

        compiler_fence(Ordering::Release);
        (paddr, vaddr)
    }

    unsafe fn dma_dealloc(
        paddr: virtio_drivers::PhysAddr,
        vaddr: NonNull<u8>,
        _pages: usize,
        _access_platform: bool,
    ) -> i32 {
        let allocation = {
            let mut allocations = DMA_ALLOCATIONS.lock();
            let Some(index) = allocations
                .iter()
                .position(|entry| entry.paddr == paddr && entry.vaddr == vaddr.as_ptr() as usize)
            else {
                return -1;
            };

            allocations.swap_remove(index).allocation
        };

        compiler_fence(Ordering::Acquire);
        match page_alloc::free(allocation) {
            Ok(()) => 0,
            Err(error) => {
                crate::println!("virtio: DMA deallocation failed: {error:?}");
                -1
            }
        }
    }

    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        let physical = PhysAddr::new(paddr as usize);
        let pointer = crate::arch::memory::phys_access::mmio_mut_ptr::<u8>(physical)
            .unwrap_or_else(|error| {
                panic!(
                    "virtio: unsupported PCI MMIO translation paddr={paddr:#x} size={size:#x}: {error:?}",
                );
            });

        NonNull::new(pointer).expect("MMIO physical translation returned a null pointer")
    }

    unsafe fn share(
        buffer: NonNull<[u8]>,
        _direction: BufferDirection,
        _access_platform: bool,
    ) -> virtio_drivers::PhysAddr {
        let bytes = {
            // SAFETY: Hal::share's caller guarantees a valid, non-empty
            // mutable slice for the duration of this call.
            unsafe { buffer.as_ref().len() }
        };
        let address = buffer.as_ptr().cast::<u8>() as usize;
        match virtual_to_physical(VirtAddr::new(address), bytes) {
            Some(physical) => physical.get() as u64,
            None => panic!(
                "virtio: cannot share non-direct-mapped buffer vaddr={address:#x} len={bytes:#x}",
            ),
        }
    }

    unsafe fn unshare(
        _paddr: virtio_drivers::PhysAddr,
        _buffer: NonNull<[u8]>,
        _direction: BufferDirection,
        _access_platform: bool,
    ) {
    }
}

// Slotted concurrent reads. The single shared bounce buffer and the blocking
// read_blocks() call serialized every disk read behind BLK_LOCK, and the
// driver's internal wait busy-spins with interrupts disabled — every CPU
// wanting a disk block convoyed on the lock with timers off, and the
// BuildStorm syscall accounting shows several CPUs continuously blocked in
// read(). The virtqueue holds QUEUE_SIZE descriptors, so the device can
// service several requests concurrently; give each read its own slot so a
// submit never waits for an earlier request to complete.
//
// A read claims a free slot, submits its descriptor chain, releases the
// lock and yields. Any CPU that next takes the lock drains completions —
// the used ring is FIFO, so finished chains must be popped in order and
// their slots finished (copy to the caller, free per-request DMA buffers)
// before later chains become reachable.
const BLK_READ_SLOTS: usize = 5;
const BLK_SLOT_BUFFER_BYTES: usize = 128 * 1024;

enum BlkReadSlotState {
    Free,
    InFlight {
        token: u16,
        len: usize,
        dst: usize,
        dma: Option<PageAllocation>,
    },
    Done {
        result: Result<(), BlockError>,
    },
}

struct BlkReadSlot {
    state: BlkReadSlotState,
    req: BlkReq,
    resp: BlkResp,
    buffer: PageAllocation,
}

impl BlkReadSlot {
    fn new() -> Option<Self> {
        let pages = BLK_SLOT_BUFFER_BYTES
            .checked_add(PAGE_SIZE - 1)?
            / PAGE_SIZE;
        let rounded = pages.checked_next_power_of_two()?;
        let allocation =
            page_alloc::allocate(rounded.trailing_zeros() as usize, PageAllocationOptions::dma32_zeroed())
                .ok()?;
        if allocation.size() < BLK_SLOT_BUFFER_BYTES {
            let _ = page_alloc::free(allocation);
            return None;
        }
        Some(Self {
            state: BlkReadSlotState::Free,
            req: BlkReq::default(),
            resp: BlkResp::default(),
            buffer: allocation,
        })
    }
}

/// A submit that failed only because the virtqueue ran out of descriptors
/// is retried after completions drain; everything else is fatal.
enum SubmitFailure {
    QueueFull,
    Fatal(BlockError),
}

fn map_submit_error(error: virtio_drivers::Error) -> SubmitFailure {
    match error {
        virtio_drivers::Error::QueueFull => SubmitFailure::QueueFull,
        _ => SubmitFailure::Fatal(BlockError::InvalidArgument),
    }
}

/// Releases the CPU while waiting for device completions. Called without
/// the block lock. Callers that arrive with interrupts disabled (block
/// cache) cannot context-switch and fall back to a plain spin.
fn wait_for_io() {
    if crate::arch::interrupt::are_disabled() {
        core::hint::spin_loop();
    } else {
        crate::task::yield_now();
    }
}

struct VirtioBlockState<T: Transport + Send + 'static> {
    driver: VirtIOBlk<SudoHal, T>,
    read_slots: Vec<BlkReadSlot>,
}

struct VirtioBlockDevice<T: Transport + Send + 'static> {
    state: IrqSpinLock<VirtioBlockState<T>>,
    block_count: u64,
    read_only: bool,
    _mmio_mapping: Option<crate::vm::KernelIoMapping>,
}

impl<T: Transport + Send + 'static> VirtioBlockDevice<T> {
    fn new(
        driver: VirtIOBlk<SudoHal, T>,
        block_count: u64,
        read_only: bool,
        mmio_mapping: Option<crate::vm::KernelIoMapping>,
    ) -> Self {
        let mut read_slots = Vec::new();
        if read_slots.try_reserve(BLK_READ_SLOTS).is_ok() {
            for _ in 0..BLK_READ_SLOTS {
                let Some(slot) = BlkReadSlot::new() else {
                    break;
                };
                read_slots.push(slot);
            }
        }
        crate::println!(
            "  block read slots: {} x {} KiB",
            read_slots.len(),
            BLK_SLOT_BUFFER_BYTES / 1024,
        );

        Self {
            state: IrqSpinLock::new_with_class(
                VirtioBlockState { driver, read_slots },
                BLK_LOCK,
            ),
            block_count,
            read_only,
            _mmio_mapping: mmio_mapping,
        }
    }

    fn allocation_buffer_mut(
        allocation: &PageAllocation,
        length: usize,
    ) -> Result<&mut [u8], BlockError> {
        if length == 0 || length > allocation.size() {
            return Err(BlockError::InvalidArgument);
        }

        let pointer = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(
            allocation.range().start(),
        )
        .map_err(|_| BlockError::InvalidArgument)?;

        // SAFETY: this device owns the contiguous DMA32 allocation for the
        // kernel lifetime, and the block lock provides exclusive access.
        Ok(unsafe { core::slice::from_raw_parts_mut(pointer, length) })
    }

    /// Pops every slot read the device has finished, in used-ring order,
    /// copying each into its caller and freeing per-request DMA buffers.
    fn drain_reads(state: &mut VirtioBlockState<T>) {
        while let Some(token) = state.driver.peek_used() {
            let Some(index) = state.read_slots.iter().position(|slot| {
                matches!(
                    slot.state,
                    BlkReadSlotState::InFlight { token: pending, .. } if pending == token
                )
            }) else {
                // The head chain belongs to the blocking writer, which drains
                // through this helper before waiting for its own token.
                break;
            };
            let slot = &mut state.read_slots[index];
            let BlkReadSlotState::InFlight { token, len, dst, dma } =
                core::mem::replace(&mut slot.state, BlkReadSlotState::Free)
            else {
                unreachable!("slot token match raced");
            };
            let result = {
                let allocation = match &dma {
                    Some(allocation) => allocation,
                    None => &slot.buffer,
                };
                match Self::allocation_buffer_mut(allocation, len) {
                    Ok(buffer) => {
                        // SAFETY: `req`, `buffer` and `resp` are the exact
                        // buffers submitted for `token`, and the slot has
                        // not been reused since the submit.
                        let complete = unsafe {
                            state
                                .driver
                                .complete_read_blocks(token, &slot.req, buffer, &mut slot.resp)
                        };
                        match complete {
                            Ok(_) => {
                                // SAFETY: `dst` is the submitting caller's
                                // output, which stays valid until this slot
                                // publishes Done. This is its only writer.
                                unsafe {
                                    core::ptr::copy_nonoverlapping(
                                        buffer.as_ptr(),
                                        dst as *mut u8,
                                        len,
                                    );
                                }
                                Ok(())
                            }
                            Err(_) => Err(BlockError::InvalidArgument),
                        }
                    }
                    Err(error) => Err(error),
                }
            };
            if let Some(allocation) = dma {
                let _ = page_alloc::free(allocation);
            }
            slot.state = BlkReadSlotState::Done { result };
        }
    }

    /// Fills one free slot with a read request and submits it to the
    /// device. On success the slot owns the request until it drains.
    fn submit_read(
        state: &mut VirtioBlockState<T>,
        index: usize,
        block: usize,
        dst: usize,
        len: usize,
        big: &mut Option<(PageAllocation, &'static mut [u8])>,
    ) -> Result<(), SubmitFailure> {
        let slot = &mut state.read_slots[index];
        slot.resp = BlkResp::default();
        let token = if len <= slot.buffer.size() {
            let buffer = Self::allocation_buffer_mut(&slot.buffer, len)
                .map_err(SubmitFailure::Fatal)?;
            // SAFETY: the slot's req/resp/buffer stay untouched and the
            // slot is not reused until the token is drained.
            unsafe {
                state
                    .driver
                    .read_blocks_nb(block, &mut slot.req, buffer, &mut slot.resp)
            }
            .map_err(map_submit_error)?
        } else {
            // The oversized DMA32 buffer was allocated before the lock was
            // taken and is handed over only once the submit succeeds.
            let buffer = big
                .as_mut()
                .map(|(_, buffer)| &mut **buffer)
                .expect("oversized read lost its buffer");
            unsafe {
                state
                    .driver
                    .read_blocks_nb(block, &mut slot.req, buffer, &mut slot.resp)
            }
            .map_err(map_submit_error)?
        };
        slot.state = BlkReadSlotState::InFlight {
            token,
            len,
            dst,
            dma: if len <= slot.buffer.size() {
                None
            } else {
                big.take().map(|(allocation, _)| allocation)
            },
        };
        Ok(())
    }

    fn read_dma(
        &self,
        block: usize,
        output: &mut [u8],
    ) -> Result<(), BlockError> {
        // Oversized requests get their own DMA32 buffer, allocated before
        // the block lock is taken so a request never waits on memory while
        // holding the lock.
        let mut big: Option<(PageAllocation, &'static mut [u8])> = None;
        if output.len() > BLK_SLOT_BUFFER_BYTES {
            let (allocation, buffer) = dma_buffer(output.len())?;
            big = Some((allocation, buffer));
        }
        let dst = output.as_mut_ptr() as usize;

        let mut state = self.state.lock();
        let index = loop {
            Self::drain_reads(&mut state);
            if let Some(free) = state
                .read_slots
                .iter()
                .position(|slot| matches!(slot.state, BlkReadSlotState::Free))
            {
                match Self::submit_read(&mut state, free, block, dst, output.len(), &mut big) {
                    Ok(()) => break free,
                    Err(SubmitFailure::Fatal(error)) => {
                        if let Some((allocation, _)) = big {
                            let _ = page_alloc::free(allocation);
                        }
                        return Err(error);
                    }
                    // The virtqueue ran out of descriptors; completions
                    // drain on the next iteration and the submit is retried.
                    Err(SubmitFailure::QueueFull) => {}
                }
            }
            drop(state);
            if crate::arch::interrupt::are_disabled() {
                // Interrupts off: on a uniprocessor no other CPU would ever
                // drain this ring, so re-take the lock and drain in place.
                state = self.state.lock();
                core::hint::spin_loop();
            } else {
                wait_for_io();
                state = self.state.lock();
            }
        };

        let result = loop {
            Self::drain_reads(&mut state);
            let slot_state = &mut state.read_slots[index].state;
            if matches!(slot_state, BlkReadSlotState::Done { .. }) {
                let BlkReadSlotState::Done { result } =
                    core::mem::replace(slot_state, BlkReadSlotState::Free)
                else {
                    unreachable!("slot state checked as Done");
                };
                break result;
            }
            drop(state);
            if crate::arch::interrupt::are_disabled() {
                state = self.state.lock();
                core::hint::spin_loop();
            } else {
                wait_for_io();
                state = self.state.lock();
            }
        };
        result
    }

    fn write_dma(
        &self,
        block: usize,
        input: &[u8],
    ) -> Result<(), BlockError> {
        let (allocation, buffer) = dma_buffer(input.len())?;
        buffer.copy_from_slice(input);
        let mut req = BlkReq::default();
        let mut resp = BlkResp::default();

        let mut state = self.state.lock();
        let token = loop {
            Self::drain_reads(&mut state);
            match unsafe {
                state
                    .driver
                    .write_blocks_nb(block, &mut req, buffer, &mut resp)
            } {
                Ok(token) => break token,
                Err(virtio_drivers::Error::QueueFull) => core::hint::spin_loop(),
                Err(_) => {
                    let _ = page_alloc::free(allocation);
                    return Err(BlockError::InvalidArgument);
                }
            }
        };
        // Writes are rare (builds run on tmpfs), so this keeps the lock and
        // drains read completions while waiting for its own token.
        let result = loop {
            Self::drain_reads(&mut state);
            match state.driver.peek_used() {
                Some(head) if head == token => {
                    // SAFETY: `req`, `buffer` and `resp` are the buffers
                    // submitted above; the used-ring head shows the device
                    // finished this chain and it has not been popped yet.
                    let completed = unsafe {
                        state
                            .driver
                            .complete_write_blocks(token, &req, buffer, &mut resp)
                    };
                    break completed.map_err(|_| BlockError::InvalidArgument);
                }
                _ => core::hint::spin_loop(),
            }
        };
        let _ = page_alloc::free(allocation);
        result
    }
}

impl<T: Transport + Send + 'static> BlockDevice for VirtioBlockDevice<T> {
    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(
        &self,
        block: u64,
        output: &mut [u8],
    ) -> Result<(), BlockError> {
        if output.len() != SECTOR_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let block =
            usize::try_from(block).map_err(|_| BlockError::AddressOverflow)?;
        self.read_dma(block, output)
    }

    fn read_blocks(
        &self,
        block: u64,
        output: &mut [u8],
    ) -> Result<(), BlockError> {
        if output.is_empty() || output.len() % SECTOR_SIZE != 0 {
            return Err(BlockError::BadBlockSize);
        }
        let sectors = u64::try_from(output.len() / SECTOR_SIZE)
            .map_err(|_| BlockError::AddressOverflow)?;
        if block
            .checked_add(sectors)
            .is_none_or(|end| end > self.block_count)
        {
            return Err(BlockError::OutOfRange);
        }
        let block =
            usize::try_from(block).map_err(|_| BlockError::AddressOverflow)?;
        self.read_dma(block, output)
    }

    fn write_block(
        &self,
        block: u64,
        input: &[u8],
    ) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::DeviceReadOnly);
        }
        if input.len() != SECTOR_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let block =
            usize::try_from(block).map_err(|_| BlockError::AddressOverflow)?;
        self.write_dma(block, input)
    }
}

fn dma_buffer(length: usize) -> Result<(PageAllocation, &'static mut [u8]), BlockError> {
    if length == 0 {
        return Err(BlockError::InvalidArgument);
    }
    let pages = length
        .checked_add(PAGE_SIZE - 1)
        .ok_or(BlockError::AddressOverflow)?
        / PAGE_SIZE;
    let rounded_pages = pages
        .checked_next_power_of_two()
        .ok_or(BlockError::AddressOverflow)?;
    let order = rounded_pages.trailing_zeros() as usize;
    let allocation = page_alloc::allocate(order, PageAllocationOptions::dma32_zeroed())
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    let pointer =
        match crate::arch::memory::phys_access::ram_mut_ptr::<u8>(allocation.range().start()) {
            Ok(pointer) => pointer,
            Err(_) => {
                let _ = page_alloc::free(allocation);
                return Err(BlockError::InvalidArgument);
            }
        };
    // SAFETY: the rounded DMA32 allocation covers `length` contiguous bytes
    // and remains owned until the virtio request and copy complete.
    let buffer = unsafe { core::slice::from_raw_parts_mut(pointer, length) };
    Ok((allocation, buffer))
}

pub fn initialize(regions: &MmioRegions, pci_hosts: &PciHostBridges) {
    let mut probed = 0;
    let mut usable = 0;

    crate::println!("virtio:");
    crate::println!("  mmio regions   : {}", regions.len());
    if regions.overflow() != 0 {
        crate::println!("  region overflow: {}", regions.overflow());
    }

    for region in regions.iter() {
        probed += 1;
        match probe_mmio_region(region) {
            Ok(Some(device)) => {
                let name = virtio_block_name(usable);
                match block::register_device(&name, Arc::clone(&device)) {
                    Ok(()) => {
                        crate::println!("  block registry : /dev/{name}");
                        register_compat_partition_alias(&name, &device);
                        usable += 1;
                    }
                    Err(error) => {
                        crate::println!("  block registry : failed ({error:?})");
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                crate::println!(
                    "  mmio {:#018x}: ignored ({})",
                    region.base(),
                    error.as_str(),
                );
            }
        }
    }

    if pci_hosts.overflow() != 0 {
        crate::println!("  pci overflow   : {}", pci_hosts.overflow());
    }
    crate::println!("  pci hosts      : {}", pci_hosts.len());
    for host in pci_hosts.iter() {
        match probe_pci_host(host, usable) {
            Ok(found) => {
                usable += found;
            }
            Err(error) => {
                crate::println!("  pci {}: ignored ({})", host.name(), error.as_str());
            }
        }
    }

    if probed == 0 {
        crate::println!("  mmio probe     : no devices described");
    } else {
        crate::println!("  mmio probe     : {} region(s) checked", probed);
    }
    crate::println!("  block devices  : {}", usable);
}

fn register_compat_partition_alias(name: &str, device: &Arc<dyn BlockDevice>) {
    if name != "vda" || block::open_device("vda2").is_some() {
        return;
    }
    match block::register_device("vda2", Arc::clone(device)) {
        Ok(()) => crate::println!("  block registry : /dev/vda2 -> /dev/vda"),
        Err(error) => crate::println!("  block registry : /dev/vda2 alias failed ({error:?})"),
    }
}

fn virtio_block_name(index: usize) -> alloc::string::String {
    let mut name = alloc::string::String::from("vd");
    let suffix = b'a'.saturating_add(index.min(25) as u8);
    name.push(suffix as char);
    name
}

#[cfg(debug_assertions)]
pub fn verify() {
    let (paddr, vaddr) = <SudoHal as Hal>::dma_alloc(1, BufferDirection::Both, false);
    assert_ne!(paddr, 0, "virtio DMA verifier could not allocate a page");
    assert_eq!(paddr as usize & (PAGE_SIZE - 1), 0);
    assert_eq!(vaddr.as_ptr() as usize & (PAGE_SIZE - 1), 0);

    // SAFETY: this deallocates the page tuple returned by the matching
    // dma_alloc call above, and no references to the page remain.
    let result = unsafe { <SudoHal as Hal>::dma_dealloc(paddr, vaddr, 1, false) };
    assert_eq!(result, 0);

    crate::println!("M15 virtio gate:");
    crate::println!("  vendor driver       : linked");
    crate::println!("  DMA32 allocation    : verified");
    crate::println!("  DMA lifecycle       : verified");
}

fn probe_mmio_region(region: MmioRegion) -> Result<Option<Arc<dyn BlockDevice>>, VirtioProbeError> {
    if region.size() < core::mem::size_of::<VirtIOHeader>() {
        return Err(VirtioProbeError::RegionTooSmall);
    }

    let mapping = crate::vm::ioremap(PhysAddr::new(region.base()), region.size())
        .map_err(|_| VirtioProbeError::MapFailed)?;
    let header = NonNull::new(mapping.virtual_address().get() as *mut VirtIOHeader)
        .ok_or(VirtioProbeError::MapFailed)?;

    // SAFETY: ioremap installed a private kernel device mapping covering this
    // FDT-described MMIO range. The mapping is moved into any live device and
    // otherwise unmapped before returning.
    let transport = match unsafe { MmioTransport::new(header, mapping.size()) } {
        Ok(transport) => transport,
        Err(error) => {
            let _ = crate::vm::iounmap(mapping);
            return match error {
                virtio_drivers::transport::mmio::MmioError::InvalidDeviceID(_) => Ok(None),
                _ => Err(VirtioProbeError::InvalidTransport),
            };
        }
    };

    let device_type = transport.device_type();
    crate::println!(
        "  mmio {:#018x}: {:?} {:?}",
        region.base(),
        device_type,
        transport.version(),
    );

    match device_type {
        DeviceType::Block => {
            let driver = VirtIOBlk::<SudoHal, _>::new(transport)
                .map_err(|_| VirtioProbeError::DriverFailed)?;
            let block_count = driver.capacity();
            let read_only = driver.readonly();
            crate::println!(
                "  block device   : {} sectors, readonly={}",
                block_count,
                read_only,
            );
            Ok(Some(Arc::new(VirtioBlockDevice::new(
                driver,
                block_count,
                read_only,
                Some(mapping),
            ))))
        }
        DeviceType::EntropySource => {
            let driver = virtio_drivers::device::rng::VirtIORng::<SudoHal, _>::new(transport)
                .map_err(|_| VirtioProbeError::DriverFailed)?;
            let rng_device = Arc::new(IrqSpinLock::new_with_class(
                driver,
                crate::lockdep::LockClass::new("virtio.rng", crate::lockdep::LockRank::Vfs, 22),
            ));
            crate::rng::register_hardware_source(alloc::boxed::Box::new(
                move |buf: &mut [u8]| -> usize {
                    rng_device.lock().request_entropy(buf).unwrap_or(0)
                },
            ));
            crate::println!("  rng device     : registered");
            Ok(None)
        }
        DeviceType::Network => {
            let raw = virtio_drivers::device::net::VirtIONetRaw::<
                SudoHal,
                MmioTransport,
                { crate::net::virtio_net::NET_QUEUE_SIZE },
            >::new(transport)
            .map_err(|_| VirtioProbeError::DriverFailed)?;
            let net_dev = crate::net::virtio_net::from_raw(raw, Some(mapping));
            let mac = net_dev.mac_address();
            let name = alloc::format!("eth{}", crate::net::registered_interfaces().len());
            crate::net::register_interface(&name, net_dev)
                .map_err(|_| VirtioProbeError::DriverFailed)?;
            crate::println!(
                "  net device     : {name} MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0],
                mac[1],
                mac[2],
                mac[3],
                mac[4],
                mac[5],
            );
            Ok(None)
        }
        _ => {
            drop(transport);
            crate::vm::iounmap(mapping).map_err(|_| VirtioProbeError::UnmapFailed)?;
            Ok(None)
        }
    }
}

fn probe_pci_host(host: PciHostBridge, first_index: usize) -> Result<usize, VirtioProbeError> {
    let ecam = host.ecam();
    if ecam.size() < myos_mm::PAGE_SIZE {
        return Err(VirtioProbeError::RegionTooSmall);
    }
    let ecam_base = crate::arch::memory::phys_access::mmio_virtual_address(
        ecam.start(),
        Cam::Ecam.size() as usize,
    )
    .map_err(|_| VirtioProbeError::MapFailed)?;
    let mut root = PciRoot::new(
        // SAFETY: the FDT `pci-host-ecam-generic` node declares an ECAM
        // configuration window, and the architecture exposes it through an
        // uncached MMIO alias for the lifetime of the kernel.
        unsafe { MmioCam::new(ecam_base.get() as *mut u8, Cam::Ecam) },
    );
    let mut allocator = PciMemory32Allocator::new(host.mem32())?;
    let mut found = 0;

    crate::println!(
        "  pci {}: ecam={:#018x} mem32={:#018x}..{:#018x} bus={}..{}",
        host.name(),
        ecam.start().get(),
        host.mem32().start().get(),
        host.mem32().end().get(),
        host.first_bus(),
        host.last_bus(),
    );

    for bus in host.first_bus()..=host.last_bus() {
        for (device_function, info) in root.enumerate_bus(bus) {
            let Some(device_type) = virtio_device_type(&info) else {
                continue;
            };
            crate::println!("  pci {device_function}: virtio {device_type:?}");
            allocate_bars(&mut root, device_function, &mut allocator)?;
            match device_type {
                DeviceType::Block => match probe_pci_block(&mut root, device_function) {
                    Ok(device) => {
                        let name = virtio_block_name(first_index + found);
                        match block::register_device(&name, Arc::clone(&device)) {
                            Ok(()) => {
                                crate::println!("  block registry : /dev/{name}");
                                register_compat_partition_alias(&name, &device);
                                found += 1;
                            }
                            Err(error) => {
                                crate::println!("  block registry : failed ({error:?})");
                            }
                        }
                    }
                    Err(error) => {
                        crate::println!("  pci {device_function}: ignored ({})", error.as_str());
                    }
                },
                DeviceType::EntropySource => {
                    if probe_pci_rng(&mut root, device_function).is_ok() {
                        crate::println!("  rng device     : registered");
                    }
                }
                DeviceType::Network => match probe_pci_net(&mut root, device_function) {
                    Ok(name) => {
                        crate::println!("  net device     : {name} registered");
                    }
                    Err(error) => {
                        crate::println!(
                            "  pci {device_function}: net probe failed ({})",
                            error.as_str()
                        );
                    }
                },
                _ => {}
            }
        }
    }

    Ok(found)
}

fn probe_pci_block(
    root: &mut PciRoot<impl ConfigurationAccess>,
    device_function: DeviceFunction,
) -> Result<Arc<dyn BlockDevice>, VirtioProbeError> {
    let transport = PciTransport::new::<SudoHal, _>(root, device_function)
        .map_err(|_| VirtioProbeError::InvalidTransport)?;
    let driver =
        VirtIOBlk::<SudoHal, _>::new(transport).map_err(|_| VirtioProbeError::DriverFailed)?;
    let block_count = driver.capacity();
    let read_only = driver.readonly();
    crate::println!(
        "  block device   : {} sectors, readonly={}",
        block_count,
        read_only,
    );

    Ok(Arc::new(VirtioBlockDevice::new(
        driver,
        block_count,
        read_only,
        None,
    )))
}

fn probe_pci_rng(
    root: &mut PciRoot<impl ConfigurationAccess>,
    device_function: DeviceFunction,
) -> Result<(), VirtioProbeError> {
    let transport = PciTransport::new::<SudoHal, _>(root, device_function)
        .map_err(|_| VirtioProbeError::InvalidTransport)?;
    let driver = virtio_drivers::device::rng::VirtIORng::<SudoHal, _>::new(transport)
        .map_err(|_| VirtioProbeError::DriverFailed)?;
    let rng_device = Arc::new(IrqSpinLock::new_with_class(
        driver,
        crate::lockdep::LockClass::new("virtio.rng", crate::lockdep::LockRank::Vfs, 22),
    ));
    crate::rng::register_hardware_source(alloc::boxed::Box::new(move |buf: &mut [u8]| -> usize {
        rng_device.lock().request_entropy(buf).unwrap_or(0)
    }));
    Ok(())
}

fn probe_pci_net(
    root: &mut PciRoot<impl ConfigurationAccess>,
    device_function: DeviceFunction,
) -> Result<alloc::string::String, VirtioProbeError> {
    let transport = PciTransport::new::<SudoHal, _>(root, device_function)
        .map_err(|_| VirtioProbeError::InvalidTransport)?;
    let raw = virtio_drivers::device::net::VirtIONetRaw::<
        SudoHal,
        PciTransport,
        { crate::net::virtio_net::NET_QUEUE_SIZE },
    >::new(transport)
    .map_err(|_| VirtioProbeError::DriverFailed)?;
    let net_dev = crate::net::virtio_net::from_raw(raw, None);
    let mac = net_dev.mac_address();
    let name = alloc::format!("eth{}", crate::net::registered_interfaces().len());
    crate::net::register_interface(&name, net_dev).map_err(|_| VirtioProbeError::DriverFailed)?;
    crate::println!(
        "  net device     : {name} MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
    );
    Ok(name)
}

struct PciMemory32Allocator {
    cursor: u32,
    end: u32,
}

impl PciMemory32Allocator {
    fn new(range: myos_mm::PhysRange) -> Result<Self, VirtioProbeError> {
        let start = u32::try_from(range.start().get()).map_err(|_| VirtioProbeError::MapFailed)?;
        let end_address = range.end().get();
        let end = u32::try_from(end_address).map_err(|_| VirtioProbeError::MapFailed)?;
        if start >= end {
            return Err(VirtioProbeError::MapFailed);
        }
        Ok(Self { cursor: start, end })
    }

    fn allocate(&mut self, size: u32) -> Result<u32, VirtioProbeError> {
        if size == 0 || !size.is_power_of_two() {
            return Err(VirtioProbeError::InvalidTransport);
        }
        let address = align_up_u32(self.cursor, size).ok_or(VirtioProbeError::MapFailed)?;
        let next = address
            .checked_add(size)
            .ok_or(VirtioProbeError::MapFailed)?;
        if next > self.end {
            return Err(VirtioProbeError::MapFailed);
        }
        self.cursor = next;
        Ok(address)
    }
}

fn align_up_u32(value: u32, alignment: u32) -> Option<u32> {
    Some(value.checked_add(alignment.checked_sub(1)?)? & !(alignment - 1))
}

fn allocate_bars(
    root: &mut PciRoot<impl ConfigurationAccess>,
    device_function: DeviceFunction,
    allocator: &mut PciMemory32Allocator,
) -> Result<(), VirtioProbeError> {
    let bars = root
        .bars(device_function)
        .map_err(|_| VirtioProbeError::InvalidTransport)?;
    for (index, info) in bars.into_iter().enumerate() {
        let Some(info) = info else {
            continue;
        };
        let BarInfo::Memory {
            address_type, size, ..
        } = info
        else {
            continue;
        };
        if size == 0 {
            continue;
        }
        let size = u32::try_from(size).map_err(|_| VirtioProbeError::MapFailed)?;
        let address = allocator.allocate(size)?;
        match address_type {
            MemoryBarType::Width32 => root.set_bar_32(device_function, index as u8, address),
            MemoryBarType::Width64 => root.set_bar_64(device_function, index as u8, address.into()),
            MemoryBarType::Below1MiB => return Err(VirtioProbeError::InvalidTransport),
        }
    }

    root.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VirtioProbeError {
    DriverFailed,
    InvalidTransport,
    MapFailed,
    RegionTooSmall,
    UnmapFailed,
}

impl VirtioProbeError {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DriverFailed => "driver initialization failed",
            Self::InvalidTransport => "invalid transport header",
            Self::MapFailed => "ioremap failed",
            Self::RegionTooSmall => "region too small",
            Self::UnmapFailed => "iounmap failed",
        }
    }
}

#[cfg(target_arch = "riscv64")]
fn virtual_to_physical(address: VirtAddr, size: usize) -> Option<PhysAddr> {
    if range_fits(address, size, crate::arch::memory::layout::DIRECT_MAP) {
        return crate::arch::memory::layout::direct_to_phys(address);
    }
    if range_fits(address, size, crate::arch::memory::layout::KERNEL_IMAGE) {
        return crate::arch::memory::layout::kernel_image_physical_address(address);
    }
    translate_kernel_contiguous(address, size)
}

#[cfg(target_arch = "riscv64")]
fn translate_kernel_contiguous(address: VirtAddr, size: usize) -> Option<PhysAddr> {
    if size == 0 {
        return crate::vm::kernel_translate(address).ok().flatten();
    }
    let end = address.checked_add(size - 1)?;
    let start_phys = crate::vm::kernel_translate(address).ok().flatten()?;
    let end_phys = crate::vm::kernel_translate(end).ok().flatten()?;
    let expected_end = start_phys.checked_add(size - 1)?;
    if expected_end == end_phys {
        Some(start_phys)
    } else {
        None
    }
}

#[cfg(target_arch = "loongarch64")]
fn virtual_to_physical(address: VirtAddr, size: usize) -> Option<PhysAddr> {
    if range_fits(
        address,
        size,
        crate::arch::memory::layout::CACHED_DIRECT_MAP,
    ) {
        return crate::arch::memory::layout::cached_to_phys(address);
    }
    if range_fits(
        address,
        size,
        crate::arch::memory::layout::UNCACHED_DIRECT_MAP,
    ) {
        return crate::arch::memory::layout::uncached_to_phys(address);
    }
    translate_kernel_contiguous(address, size)
}

#[cfg(target_arch = "loongarch64")]
fn translate_kernel_contiguous(address: VirtAddr, size: usize) -> Option<PhysAddr> {
    if size == 0 {
        return crate::vm::kernel_translate(address).ok().flatten();
    }
    let end = address.checked_add(size - 1)?;
    let start_phys = crate::vm::kernel_translate(address).ok().flatten()?;
    let end_phys = crate::vm::kernel_translate(end).ok().flatten()?;
    let expected_end = start_phys.checked_add(size - 1)?;
    if expected_end == end_phys {
        Some(start_phys)
    } else {
        None
    }
}

fn range_fits(address: VirtAddr, size: usize, range: myos_mm::VirtRange) -> bool {
    let Some(end) = address.checked_add(size) else {
        return false;
    };

    range.contains(address) && end.get() <= range.end().get()
}
