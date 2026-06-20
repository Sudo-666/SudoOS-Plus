use alloc::{sync::Arc, vec::Vec};
use core::ptr::NonNull;

use myos_mm::{PAGE_SIZE, PageAllocation, PhysAddr, VirtAddr};
use virtio_drivers::{
    BufferDirection, Hal,
    device::blk::{SECTOR_SIZE, VirtIOBlk},
    transport::{
        DeviceType, Transport,
        mmio::{MmioTransport, VirtIOHeader},
    },
};

use crate::{
    block::{self, BlockDevice, BlockError},
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
    page_alloc::{self, PageAllocationOptions},
};

const MAX_MMIO_REGIONS: usize = 32;
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

struct VirtioBlockDevice {
    driver: IrqSpinLock<VirtIOBlk<SudoHal, MmioTransport<'static>>>,
    block_count: u64,
    read_only: bool,
    _mapping: crate::vm::KernelIoMapping,
}

impl BlockDevice for VirtioBlockDevice {
    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if output.len() != SECTOR_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let block = usize::try_from(block).map_err(|_| BlockError::AddressOverflow)?;
        self.driver
            .lock()
            .read_blocks(block, output)
            .map_err(|_| BlockError::InvalidArgument)
    }

    fn write_block(&self, block: u64, input: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::DeviceReadOnly);
        }
        if input.len() != SECTOR_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let block = usize::try_from(block).map_err(|_| BlockError::AddressOverflow)?;
        self.driver
            .lock()
            .write_blocks(block, input)
            .map_err(|_| BlockError::InvalidArgument)
    }
}

pub fn initialize(regions: &MmioRegions) {
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
                match block::register_device(&name, device) {
                    Ok(()) => {
                        crate::println!("  block registry : /dev/{name}");
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

    if probed == 0 {
        crate::println!("  mmio probe     : no devices described");
    } else {
        crate::println!("  mmio probe     : {} region(s) checked", probed);
    }
    crate::println!("  block devices  : {}", usable);
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

    if device_type != DeviceType::Block {
        drop(transport);
        crate::vm::iounmap(mapping).map_err(|_| VirtioProbeError::UnmapFailed)?;
        return Ok(None);
    }

    let driver =
        VirtIOBlk::<SudoHal, _>::new(transport).map_err(|_| VirtioProbeError::DriverFailed)?;
    let block_count = driver.capacity();
    let read_only = driver.readonly();
    crate::println!(
        "  block device   : {} sectors, readonly={}",
        block_count,
        read_only,
    );

    Ok(Some(Arc::new(VirtioBlockDevice {
        driver: IrqSpinLock::new_with_class(driver, BLK_LOCK),
        block_count,
        read_only,
        _mapping: mapping,
    })))
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
    None
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
    None
}

fn range_fits(address: VirtAddr, size: usize, range: myos_mm::VirtRange) -> bool {
    let Some(end) = address.checked_add(size) else {
        return false;
    };

    range.contains(address) && end.get() <= range.end().get()
}
