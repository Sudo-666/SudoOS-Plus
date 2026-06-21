use myos_mm::{MappingOptions, MappingOptionsError, MemoryType, PAGE_SHIFT, PhysAddr, PhysFrame};

const VALID: u64 = 1 << 0;
const READ: u64 = 1 << 1;
const WRITE: u64 = 1 << 2;
const EXECUTE: u64 = 1 << 3;
const USER: u64 = 1 << 4;
const GLOBAL: u64 = 1 << 5;
const ACCESSED: u64 = 1 << 6;
const DIRTY: u64 = 1 << 7;

const LEAF_MASK: u64 = READ | WRITE | EXECUTE;

const PPN_SHIFT: usize = 10;
const PPN_BITS: usize = 44;

const PPN_VALUE_MASK: u64 = (1_u64 << PPN_BITS) - 1;

const PPN_MASK: u64 = PPN_VALUE_MASK << PPN_SHIFT;

/// Sv39 能表达最多 56 位物理地址。
const PHYSICAL_ADDRESS_BITS: usize = PAGE_SHIFT + PPN_BITS;

const PHYSICAL_ADDRESS_LIMIT: u64 = 1_u64 << PHYSICAL_ADDRESS_BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageTableEntryError {
    InvalidMappingOptions(MappingOptionsError),

    UnsupportedMemoryType,

    PhysicalAddressOutOfRange { address: PhysAddr },
}

impl From<MappingOptionsError> for PageTableEntryError {
    fn from(error: MappingOptionsError) -> Self {
        Self::InvalidMappingOptions(error)
    }
}

/// RISC-V Sv39 页表项。
///
/// 同一个格式既用于目录项，也用于叶表项。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    /// 创建指向下一级页表的非叶表项。
    pub fn table(next_table: PhysFrame) -> Result<Self, PageTableEntryError> {
        let ppn = encode_frame(next_table)?;

        /*
         * 非叶表项只设置 V。
         *
         * R/W/X/U/A/D 等位保持为零。
         */
        Ok(Self(ppn | VALID))
    }

    /// 创建 4 KiB 叶表项。
    pub fn leaf(frame: PhysFrame, options: MappingOptions) -> Result<Self, PageTableEntryError> {
        options.validate()?;

        /*
         * 基础 RISC-V PTE 没有普通 cache mode 字段。
         *
         * Device 区域的属性由平台 PMA 决定，因此不需要额外位。
         * 将普通 RAM 映射为 Uncached 则需要 Svpbmt 等扩展，
         * 当前暂不支持。
         */
        if matches!(options.memory_type(), MemoryType::Uncached) {
            return Err(PageTableEntryError::UnsupportedMemoryType);
        }

        let mut raw = encode_frame(frame)? | VALID | ACCESSED;

        let permissions = options.permissions();

        if permissions.is_readable() {
            raw |= READ;
        }

        if permissions.is_writable() {
            raw |= WRITE | DIRTY;
        }

        if permissions.is_executable() {
            raw |= EXECUTE;
        }

        if options.is_user() {
            raw |= USER;
        }

        if options.is_global() {
            raw |= GLOBAL;
        }

        Ok(Self(raw))
    }

    pub const fn is_valid(self) -> bool {
        self.0 & VALID != 0
    }

    pub const fn is_leaf(self) -> bool {
        self.is_valid() && self.0 & LEAF_MASK != 0
    }

    pub const fn is_table(self) -> bool {
        self.is_valid() && self.0 & LEAF_MASK == 0
    }

    pub const fn is_user(self) -> bool {
        self.0 & USER != 0
    }

    pub const fn is_global(self) -> bool {
        self.0 & GLOBAL != 0
    }

    pub const fn is_readable(self) -> bool {
        self.0 & READ != 0
    }

    pub const fn is_writable(self) -> bool {
        self.0 & WRITE != 0
    }

    pub const fn is_executable(self) -> bool {
        self.0 & EXECUTE != 0
    }

    pub const fn physical_address(self) -> PhysAddr {
        let ppn = (self.0 & PPN_MASK) >> PPN_SHIFT;

        PhysAddr::new((ppn as usize) << PAGE_SHIFT)
    }

    pub const fn frame(self) -> Option<PhysFrame> {
        if !self.is_valid() {
            return None;
        }

        PhysFrame::from_start_address(self.physical_address())
    }
}

fn encode_frame(frame: PhysFrame) -> Result<u64, PageTableEntryError> {
    let address = frame.start_address();

    let physical = address.get() as u64;

    if physical >= PHYSICAL_ADDRESS_LIMIT {
        return Err(PageTableEntryError::PhysicalAddressOutOfRange { address });
    }

    let ppn = physical >> PAGE_SHIFT;

    Ok((ppn << PPN_SHIFT) & PPN_MASK)
}

pub(super) fn validate() {
    let frame = PhysFrame::from_start_address(PhysAddr::new(0x8020_0000))
        .expect("test frame must be aligned");

    let table = PageTableEntry::table(frame).expect("valid table entry rejected");

    assert!(table.is_valid());
    assert!(table.is_table());
    assert!(!table.is_leaf());

    assert_eq!(table.frame(), Some(frame),);

    let code = PageTableEntry::leaf(frame, MappingOptions::kernel_code())
        .expect("valid code PTE rejected");

    assert!(code.is_leaf());
    assert!(code.is_readable());
    assert!(code.is_executable());
    assert!(!code.is_writable());
    assert!(code.is_global());

    let data = PageTableEntry::leaf(frame, MappingOptions::kernel_data())
        .expect("valid data PTE rejected");

    assert!(data.is_leaf());
    assert!(data.is_readable());
    assert!(data.is_writable());
    assert!(!data.is_executable());
}
