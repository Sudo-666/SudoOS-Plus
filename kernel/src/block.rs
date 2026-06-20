use alloc::{string::String, sync::Arc, vec::Vec};

use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const CACHE_LOCK: LockClass = LockClass::new("block.cache", LockRank::Vfs, 10);
const BLOCK_LOCK: LockClass = LockClass::new("block.device", LockRank::Vfs, 11);
const REGISTRY_LOCK: LockClass = LockClass::new("block.registry", LockRank::Vfs, 2);
const DEFAULT_CACHE_BLOCKS: usize = 8;
const DEFAULT_PAGE_CACHE_PAGES: usize = 8;

static BLOCK_DEVICES: IrqSpinLock<Vec<RegisteredBlockDevice>> =
    IrqSpinLock::new_with_class(Vec::new(), REGISTRY_LOCK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockError {
    AddressOverflow,
    BadBlockSize,
    BufferTooSmall,
    DeviceReadOnly,
    InvalidArgument,
    MetadataOutOfMemory,
    OutOfRange,
}

#[derive(Clone)]
pub struct RegisteredBlockDevice {
    name: String,
    device: Arc<dyn BlockDevice>,
}

impl RegisteredBlockDevice {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn device(&self) -> Arc<dyn BlockDevice> {
        Arc::clone(&self.device)
    }
}

pub trait BlockDevice: Send + Sync + 'static {
    fn block_size(&self) -> usize;
    fn block_count(&self) -> u64;
    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError>;
    fn write_block(&self, block: u64, input: &[u8]) -> Result<(), BlockError>;

    fn flush(&self) -> Result<(), BlockError> {
        Ok(())
    }

    fn size_bytes(&self) -> Result<u64, BlockError> {
        let block_size = u64::try_from(self.block_size()).map_err(|_| BlockError::BadBlockSize)?;
        self.block_count()
            .checked_mul(block_size)
            .ok_or(BlockError::AddressOverflow)
    }
}

pub fn register_device(name: &str, device: Arc<dyn BlockDevice>) -> Result<(), BlockError> {
    validate_device_name(name)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    stored_name.push_str(name);

    let mut devices = BLOCK_DEVICES.lock();
    if devices.iter().any(|entry| entry.name == stored_name) {
        return Err(BlockError::InvalidArgument);
    }
    devices
        .try_reserve(1)
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    devices.push(RegisteredBlockDevice {
        name: stored_name,
        device,
    });
    Ok(())
}

#[cfg(debug_assertions)]
pub fn unregister_device(name: &str) -> Result<(), BlockError> {
    let mut devices = BLOCK_DEVICES.lock();
    let index = devices
        .iter()
        .position(|entry| entry.name == name)
        .ok_or(BlockError::InvalidArgument)?;
    devices.remove(index);
    Ok(())
}

pub fn open_device(name: &str) -> Option<Arc<dyn BlockDevice>> {
    BLOCK_DEVICES
        .lock()
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| Arc::clone(&entry.device))
}

pub fn registered_devices() -> Result<Vec<RegisteredBlockDevice>, BlockError> {
    let devices = BLOCK_DEVICES.lock();
    let mut snapshot = Vec::new();
    snapshot
        .try_reserve(devices.len())
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    for device in devices.iter() {
        snapshot.push(device.clone());
    }
    Ok(snapshot)
}

pub fn read_at(
    device: &Arc<dyn BlockDevice>,
    offset: u64,
    output: &mut [u8],
) -> Result<usize, BlockError> {
    transfer_at(device, offset, output, None)
}

pub fn write_at(
    device: &Arc<dyn BlockDevice>,
    offset: u64,
    input: &[u8],
) -> Result<usize, BlockError> {
    let mut written = 0;
    let mut cursor = offset;
    let block_size = device.block_size();
    validate_device_geometry(device.as_ref())?;
    let size = device.size_bytes()?;
    if offset >= size {
        return Ok(0);
    }
    let mut scratch = Vec::new();
    scratch
        .try_reserve(block_size)
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    scratch.resize(block_size, 0);

    while written < input.len() && cursor < size {
        let block = cursor / block_size as u64;
        let block_offset = (cursor % block_size as u64) as usize;
        let count = (block_size - block_offset)
            .min(input.len() - written)
            .min((size - cursor) as usize);
        device.read_block(block, &mut scratch)?;
        scratch[block_offset..block_offset + count]
            .copy_from_slice(&input[written..written + count]);
        device.write_block(block, &scratch)?;
        written += count;
        cursor += count as u64;
    }
    Ok(written)
}

struct CachedBlock {
    block: u64,
    valid: bool,
    dirty: bool,
    age: u64,
    data: Vec<u8>,
}

pub struct BufferCache {
    device: Arc<dyn BlockDevice>,
    state: IrqSpinLock<BufferCacheState>,
}

struct BufferCacheState {
    block_size: usize,
    tick: u64,
    entries: Vec<CachedBlock>,
}

impl BufferCache {
    pub fn new(device: Arc<dyn BlockDevice>, capacity: usize) -> Result<Self, BlockError> {
        let block_size = device.block_size();
        if block_size == 0 || !block_size.is_power_of_two() {
            return Err(BlockError::BadBlockSize);
        }
        let capacity = capacity.max(1);
        let mut entries = Vec::new();
        entries
            .try_reserve(capacity)
            .map_err(|_| BlockError::MetadataOutOfMemory)?;
        for _ in 0..capacity {
            let mut data = Vec::new();
            data.try_reserve(block_size)
                .map_err(|_| BlockError::MetadataOutOfMemory)?;
            data.resize(block_size, 0);
            entries.push(CachedBlock {
                block: 0,
                valid: false,
                dirty: false,
                age: 0,
                data,
            });
        }
        Ok(Self {
            device,
            state: IrqSpinLock::new_with_class(
                BufferCacheState {
                    block_size,
                    tick: 1,
                    entries,
                },
                CACHE_LOCK,
            ),
        })
    }

    pub fn read(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        self.validate_block_buffer(block, output)?;
        let mut state = self.state.lock();
        let index = self.get_or_fill_locked(&mut state, block)?;
        output.copy_from_slice(&state.entries[index].data);
        Ok(())
    }

    pub fn write(&self, block: u64, input: &[u8]) -> Result<(), BlockError> {
        self.validate_block_buffer(block, input)?;
        let mut state = self.state.lock();
        let index = self.get_or_fill_locked(&mut state, block)?;
        state.entries[index].data.copy_from_slice(input);
        state.entries[index].dirty = true;
        Ok(())
    }

    pub fn flush(&self) -> Result<(), BlockError> {
        let mut state = self.state.lock();
        for entry in &mut state.entries {
            if entry.valid && entry.dirty {
                self.device.write_block(entry.block, &entry.data)?;
                entry.dirty = false;
            }
        }
        self.device.flush()?;
        Ok(())
    }

    fn validate_block_buffer(&self, block: u64, buffer: &[u8]) -> Result<(), BlockError> {
        let block_size = self.device.block_size();
        if buffer.len() != block_size {
            return Err(BlockError::BufferTooSmall);
        }
        if block >= self.device.block_count() {
            return Err(BlockError::OutOfRange);
        }
        Ok(())
    }

    fn get_or_fill_locked(
        &self,
        state: &mut BufferCacheState,
        block: u64,
    ) -> Result<usize, BlockError> {
        if let Some(index) = state
            .entries
            .iter()
            .position(|entry| entry.valid && entry.block == block)
        {
            state.tick = state.tick.wrapping_add(1);
            state.entries[index].age = state.tick;
            return Ok(index);
        }

        let victim = state
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, entry)| (entry.valid, entry.age))
            .map(|(index, _)| index)
            .ok_or(BlockError::InvalidArgument)?;
        if state.entries[victim].valid && state.entries[victim].dirty {
            let entry = &state.entries[victim];
            self.device.write_block(entry.block, &entry.data)?;
        }
        state.entries[victim].block = block;
        state.entries[victim].valid = true;
        state.entries[victim].dirty = false;
        state.tick = state.tick.wrapping_add(1);
        state.entries[victim].age = state.tick;
        self.device
            .read_block(block, &mut state.entries[victim].data[..state.block_size])?;
        Ok(victim)
    }
}

pub struct PageCache {
    device: Arc<dyn BlockDevice>,
    buffer: BufferCache,
}

impl PageCache {
    pub fn new(device: Arc<dyn BlockDevice>, capacity_pages: usize) -> Result<Self, BlockError> {
        validate_device_geometry(device.as_ref())?;
        let blocks_per_page = blocks_per_page(device.block_size())?;
        let capacity = capacity_pages
            .max(1)
            .checked_mul(blocks_per_page)
            .ok_or(BlockError::AddressOverflow)?;
        Ok(Self {
            device: device.clone(),
            buffer: BufferCache::new(device, capacity)?,
        })
    }

    pub fn read_page(&self, page: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if output.len() != myos_mm::PAGE_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        let blocks_per_page = blocks_per_page(self.device.block_size())?;
        let first_block = page
            .checked_mul(blocks_per_page as u64)
            .ok_or(BlockError::AddressOverflow)?;
        for index in 0..blocks_per_page {
            let start = index
                .checked_mul(self.device.block_size())
                .ok_or(BlockError::AddressOverflow)?;
            self.buffer.read(
                first_block + index as u64,
                &mut output[start..start + self.device.block_size()],
            )?;
        }
        Ok(())
    }

    pub fn write_page(&self, page: u64, input: &[u8]) -> Result<(), BlockError> {
        if input.len() != myos_mm::PAGE_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        let blocks_per_page = blocks_per_page(self.device.block_size())?;
        let first_block = page
            .checked_mul(blocks_per_page as u64)
            .ok_or(BlockError::AddressOverflow)?;
        for index in 0..blocks_per_page {
            let start = index
                .checked_mul(self.device.block_size())
                .ok_or(BlockError::AddressOverflow)?;
            self.buffer.write(
                first_block + index as u64,
                &input[start..start + self.device.block_size()],
            )?;
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<(), BlockError> {
        self.buffer.flush()
    }
}

pub struct MemoryBlockDevice {
    block_size: usize,
    block_count: u64,
    read_only: bool,
    data: IrqSpinLock<Vec<u8>>,
}

impl MemoryBlockDevice {
    pub fn new(block_size: usize, block_count: u64) -> Result<Self, BlockError> {
        if block_size == 0 || !block_size.is_power_of_two() || block_count == 0 {
            return Err(BlockError::BadBlockSize);
        }
        let bytes = usize::try_from(block_count)
            .ok()
            .and_then(|blocks| blocks.checked_mul(block_size))
            .ok_or(BlockError::AddressOverflow)?;
        let mut data = Vec::new();
        data.try_reserve(bytes)
            .map_err(|_| BlockError::MetadataOutOfMemory)?;
        data.resize(bytes, 0);
        Ok(Self {
            block_size,
            block_count,
            read_only: false,
            data: IrqSpinLock::new_with_class(data, BLOCK_LOCK),
        })
    }

    fn byte_range(&self, block: u64) -> Result<core::ops::Range<usize>, BlockError> {
        if block >= self.block_count {
            return Err(BlockError::OutOfRange);
        }
        let start = usize::try_from(block)
            .ok()
            .and_then(|block| block.checked_mul(self.block_size))
            .ok_or(BlockError::AddressOverflow)?;
        let end = start
            .checked_add(self.block_size)
            .ok_or(BlockError::AddressOverflow)?;
        Ok(start..end)
    }
}

impl BlockDevice for MemoryBlockDevice {
    fn block_size(&self) -> usize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if output.len() != self.block_size {
            return Err(BlockError::BufferTooSmall);
        }
        let range = self.byte_range(block)?;
        output.copy_from_slice(&self.data.lock()[range]);
        Ok(())
    }

    fn write_block(&self, block: u64, input: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::DeviceReadOnly);
        }
        if input.len() != self.block_size {
            return Err(BlockError::BufferTooSmall);
        }
        let range = self.byte_range(block)?;
        self.data.lock()[range].copy_from_slice(input);
        Ok(())
    }
}

fn validate_device_name(name: &str) -> Result<(), BlockError> {
    if name.is_empty()
        || name.len() > 31
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(BlockError::InvalidArgument);
    }
    Ok(())
}

fn validate_device_geometry(device: &dyn BlockDevice) -> Result<(), BlockError> {
    let block_size = device.block_size();
    if block_size == 0 || !block_size.is_power_of_two() || block_size > myos_mm::PAGE_SIZE {
        return Err(BlockError::BadBlockSize);
    }
    if device.block_count() == 0 {
        return Err(BlockError::OutOfRange);
    }
    Ok(())
}

fn blocks_per_page(block_size: usize) -> Result<usize, BlockError> {
    if block_size == 0
        || !block_size.is_power_of_two()
        || !myos_mm::PAGE_SIZE.is_multiple_of(block_size)
    {
        return Err(BlockError::BadBlockSize);
    }
    Ok(myos_mm::PAGE_SIZE / block_size)
}

fn transfer_at(
    device: &Arc<dyn BlockDevice>,
    offset: u64,
    output: &mut [u8],
    _unused: Option<&[u8]>,
) -> Result<usize, BlockError> {
    validate_device_geometry(device.as_ref())?;
    let size = device.size_bytes()?;
    if offset >= size {
        return Ok(0);
    }
    let block_size = device.block_size();
    let mut scratch = Vec::new();
    scratch
        .try_reserve(block_size)
        .map_err(|_| BlockError::MetadataOutOfMemory)?;
    scratch.resize(block_size, 0);

    let mut done = 0;
    let mut cursor = offset;
    while done < output.len() && cursor < size {
        let block = cursor / block_size as u64;
        let block_offset = (cursor % block_size as u64) as usize;
        let count = (block_size - block_offset)
            .min(output.len() - done)
            .min((size - cursor) as usize);
        device.read_block(block, &mut scratch)?;
        output[done..done + count].copy_from_slice(&scratch[block_offset..block_offset + count]);
        done += count;
        cursor += count as u64;
    }
    Ok(done)
}

#[cfg(debug_assertions)]
pub fn verify() {
    let device = Arc::new(MemoryBlockDevice::new(512, 16).expect("memory block device failed"));
    let byte_device: Arc<dyn BlockDevice> = device.clone();
    let cache =
        BufferCache::new(device.clone(), DEFAULT_CACHE_BLOCKS).expect("buffer cache failed");

    assert_eq!(
        device.size_bytes().expect("block device size overflowed"),
        8192,
    );

    let mut block = [0_u8; 512];
    block[0..8].copy_from_slice(b"blkcache");
    cache.write(2, &block).expect("buffer cache write failed");

    let mut readback = [0_u8; 512];
    cache
        .read(2, &mut readback)
        .expect("buffer cache read failed");
    assert_eq!(&readback[0..8], b"blkcache");

    let mut backing = [0_u8; 512];
    device
        .read_block(2, &mut backing)
        .expect("memory block backing read failed");
    assert_eq!(
        &backing[0..8],
        &[0; 8],
        "dirty buffer reached backing store before flush",
    );
    cache.flush().expect("buffer cache flush failed");
    device
        .read_block(2, &mut backing)
        .expect("memory block backing read after flush failed");
    assert_eq!(&backing[0..8], b"blkcache");

    assert_eq!(cache.read(16, &mut readback), Err(BlockError::OutOfRange));
    assert_eq!(
        cache.write(1, &readback[..511]),
        Err(BlockError::BufferTooSmall),
    );

    let page_cache = PageCache::new(device.clone(), DEFAULT_PAGE_CACHE_PAGES)
        .expect("page cache creation failed");
    let mut page = [0_u8; myos_mm::PAGE_SIZE];
    page[1024..1032].copy_from_slice(b"pagecach");
    page_cache
        .write_page(0, &page)
        .expect("page cache write failed");
    page_cache.flush().expect("page cache flush failed");
    let mut page_readback = [0_u8; myos_mm::PAGE_SIZE];
    page_cache
        .read_page(0, &mut page_readback)
        .expect("page cache read failed");
    assert_eq!(&page_readback[1024..1032], b"pagecach");

    assert_eq!(
        write_at(&byte_device, 509, b"cross-block").expect("block byte write failed"),
        b"cross-block".len(),
    );
    let mut byte_readback = [0_u8; 11];
    assert_eq!(
        read_at(&byte_device, 509, &mut byte_readback).expect("block byte read failed"),
        byte_readback.len(),
    );
    assert_eq!(&byte_readback, b"cross-block");

    register_device("m15mem", device.clone()).expect("block registry insert failed");
    assert!(open_device("m15mem").is_some());
    assert!(
        registered_devices()
            .expect("block registry snapshot failed")
            .iter()
            .any(|device| device.name() == "m15mem")
    );
    unregister_device("m15mem").expect("block registry remove failed");

    crate::println!("M15 block layer gate:");
    crate::println!("  block trait          : verified");
    crate::println!("  buffer cache         : verified");
    crate::println!("  page cache           : verified");
    crate::println!("  dirty flush          : verified");
    crate::println!("  byte range I/O       : verified");
    crate::println!("  block registry       : verified");
    crate::println!("  bounds checking      : verified");
}
