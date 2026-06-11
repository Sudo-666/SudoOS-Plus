use myos_mm::{
    MappingOptions, MappingOptionsError, MemoryType, PAGE_SIZE, PhysAddr, PhysFrame, VirtAddr,
};

use crate::memory::layout;

const VALID: u64 = 1 << 0;
const DIRTY: u64 = 1 << 1;

const PLV_SHIFT: usize = 2;
const PLV_MASK: u64 = 0b11 << PLV_SHIFT;

const PLV_KERNEL: u64 = 0;
const PLV_USER: u64 = 3;

const MAT_SHIFT: usize = 4;
const MAT_MASK: u64 = 0b11 << MAT_SHIFT;

const MAT_STRONG_UNCACHED: u64 = 0 << MAT_SHIFT;

const MAT_COHERENT_CACHED: u64 = 1 << MAT_SHIFT;

const MAT_WEAK_UNCACHED: u64 = 2 << MAT_SHIFT;

const GLOBAL: u64 = 1 << 6;

/*
 * Linux/LoongArch 软件管理位。
 */
const PRESENT: u64 = 1 << 7;
const WRITE: u64 = 1 << 8;
const MODIFIED: u64 = 1 << 9;

const PHYSICAL_ADDRESS_BITS: usize = 48;

const PHYSICAL_PAGE_MASK: u64 = ((1_u64 << PHYSICAL_ADDRESS_BITS) - 1) & !((PAGE_SIZE as u64) - 1);

const NO_READ: u64 = 1 << 61;
const NO_EXECUTE: u64 = 1 << 62;
const RESTRICTED_PLV: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableEntryError {
    InvalidMappingOptions(MappingOptionsError),

    PhysicalAddressOutOfRange { address: PhysAddr },

    TableNotDirectMappable { address: PhysAddr },
}

impl From<MappingOptionsError> for PageTableEntryError {
    fn from(error: MappingOptionsError) -> Self {
        Self::InvalidMappingOptions(error)
    }
}

/// LoongArch 最末级叶 PTE。
///
/// 此格式与 TLBELO 的硬件字段兼容，并包含 MyOS 使用的
/// PRESENT/WRITE/MODIFIED 软件状态位。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct LeafPageTableEntry(u64);

impl LeafPageTableEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn new(frame: PhysFrame, options: MappingOptions) -> Result<Self, PageTableEntryError> {
        options.validate()?;

        let address = encode_physical_address(frame.start_address())?;

        let mut raw = address | VALID | PRESENT | encode_memory_type(options.memory_type());

        let permissions = options.permissions();

        if !permissions.is_readable() {
            raw |= NO_READ;
        }

        if permissions.is_writable() {
            /*
             * 当前尚未实现 dirty fault 追踪，
             * 因此初始可写映射直接设置硬件 D 和软件状态位。
             */
            raw |= DIRTY | WRITE | MODIFIED;
        }

        if !permissions.is_executable() {
            raw |= NO_EXECUTE;
        }

        let plv = if options.is_user() {
            PLV_USER
        } else {
            PLV_KERNEL
        };

        raw |= plv << PLV_SHIFT;

        if options.is_global() {
            raw |= GLOBAL;
        }

        Ok(Self(raw))
    }

    pub const fn is_valid(self) -> bool {
        self.0 & VALID != 0
    }

    pub const fn is_present(self) -> bool {
        self.0 & PRESENT != 0
    }

    pub const fn is_readable(self) -> bool {
        self.0 & NO_READ == 0
    }

    pub const fn is_writable(self) -> bool {
        self.0 & WRITE != 0
    }

    pub const fn is_executable(self) -> bool {
        self.0 & NO_EXECUTE == 0
    }

    pub const fn is_global(self) -> bool {
        self.0 & GLOBAL != 0
    }

    pub const fn is_user(self) -> bool {
        ((self.0 & PLV_MASK) >> PLV_SHIFT) == PLV_USER
    }

    pub const fn is_restricted_plv(self) -> bool {
        self.0 & RESTRICTED_PLV != 0
    }

    pub const fn physical_address(self) -> PhysAddr {
        PhysAddr::new((self.0 & PHYSICAL_PAGE_MASK) as usize)
    }

    pub const fn frame(self) -> Option<PhysFrame> {
        if !self.is_present() {
            return None;
        }

        PhysFrame::from_start_address(self.physical_address())
    }

    /// 不存在的全局 PTE。
    ///
    /// LoongArch 的 TLB 以相邻奇偶页组成一对。为保证内核全局映射
    /// 旁边的空 PTE 不会清除整对 TLB 项的 global 属性，Linux 也会
    /// 使用只带 GLOBAL 位的空内核 PTE。
    pub const fn invalid_global() -> Self {
        Self(GLOBAL)
    }
}

/// LoongArch 上级目录项。
///
/// 与叶 PTE 不同，这里保存下一级页表的 cached DMW 地址。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TablePointerEntry(u64);

impl TablePointerEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub fn new(next_table: PhysFrame) -> Result<Self, PageTableEntryError> {
        let physical = next_table.start_address();

        let virtual_address = layout::phys_to_cached(physical)
            .ok_or(PageTableEntryError::TableNotDirectMappable { address: physical })?;

        if !virtual_address.is_aligned(PAGE_SIZE) {
            return Err(PageTableEntryError::TableNotDirectMappable { address: physical });
        }

        Ok(Self(virtual_address.get() as u64))
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn next_table_frame(self) -> Option<PhysFrame> {
        if self.is_empty() {
            return None;
        }

        let virtual_address = VirtAddr::new(self.0 as usize);

        let physical = layout::cached_to_phys(virtual_address)?;

        PhysFrame::from_start_address(physical)
    }
}

fn encode_physical_address(address: PhysAddr) -> Result<u64, PageTableEntryError> {
    let value = address.get() as u64;

    if value & !PHYSICAL_PAGE_MASK != 0 {
        return Err(PageTableEntryError::PhysicalAddressOutOfRange { address });
    }

    Ok(value)
}

const fn encode_memory_type(memory_type: MemoryType) -> u64 {
    match memory_type {
        MemoryType::Normal => MAT_COHERENT_CACHED,

        MemoryType::Device => MAT_STRONG_UNCACHED,

        MemoryType::Uncached => MAT_WEAK_UNCACHED,
    }
}

pub(super) fn validate() {
    let frame = PhysFrame::from_start_address(PhysAddr::new(0x0020_0000))
        .expect("test frame must be aligned");

    let pointer = TablePointerEntry::new(frame).expect("valid table pointer rejected");

    assert!(!pointer.is_empty());

    assert_eq!(pointer.next_table_frame(), Some(frame),);

    let code = LeafPageTableEntry::new(frame, MappingOptions::kernel_code())
        .expect("valid code PTE rejected");

    assert!(code.is_valid());
    assert!(code.is_present());
    assert!(code.is_readable());
    assert!(code.is_executable());
    assert!(!code.is_writable());
    assert!(code.is_global());

    let data = LeafPageTableEntry::new(frame, MappingOptions::kernel_data())
        .expect("valid data PTE rejected");

    assert!(data.is_valid());
    assert!(data.is_readable());
    assert!(data.is_writable());
    assert!(!data.is_executable());

    let device = LeafPageTableEntry::new(frame, MappingOptions::kernel_device())
        .expect("valid device PTE rejected");

    assert!(device.is_readable());
    assert!(device.is_writable());
    assert!(!device.is_executable());

    assert_eq!(device.raw() & MAT_MASK, MAT_STRONG_UNCACHED,);
}
