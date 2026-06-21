use core::{ptr::read_volatile, slice};

use crate::{FdtError, MemoryRegion};

const FDT_MAGIC: u32 = 0xd00d_feed;

const FDT_HEADER_SIZE: usize = 40;

/// 防止损坏的启动参数制造超大切片。
const MAX_FDT_SIZE: usize = 16 * 1024 * 1024;

/// FDT 标准 header。
#[derive(Clone, Copy, Debug)]
pub struct FdtHeader {
    total_size: usize,

    structure_offset: usize,
    strings_offset: usize,
    reservation_map_offset: usize,

    version: u32,
    last_compatible_version: u32,
    boot_cpu_id: u32,

    strings_size: usize,
    structure_size: usize,
}

impl FdtHeader {
    pub const fn total_size(&self) -> usize {
        self.total_size
    }

    pub const fn structure_offset(&self) -> usize {
        self.structure_offset
    }

    pub const fn strings_offset(&self) -> usize {
        self.strings_offset
    }

    pub const fn reservation_map_offset(&self) -> usize {
        self.reservation_map_offset
    }

    pub const fn version(&self) -> u32 {
        self.version
    }

    pub const fn last_compatible_version(&self) -> u32 {
        self.last_compatible_version
    }

    pub const fn boot_cpu_id(&self) -> u32 {
        self.boot_cpu_id
    }

    pub const fn strings_size(&self) -> usize {
        self.strings_size
    }

    pub const fn structure_size(&self) -> usize {
        self.structure_size
    }
}

/// 经过最小结构验证的 FDT blob。
///
/// 它不拥有数据，只借用引导器提供的内存。
pub struct FdtBlob<'a> {
    bytes: &'a [u8],
    header: FdtHeader,
}

impl<'a> FdtBlob<'a> {
    /// 从已经存在的字节切片解析 FDT。
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, FdtError> {
        let header = parse_header(bytes)?;

        if bytes.len() < header.total_size {
            return Err(FdtError::Truncated {
                declared: header.total_size,
                available: bytes.len(),
            });
        }

        validate_header(&header)?;

        Ok(Self {
            bytes: &bytes[..header.total_size],
            header,
        })
    }

    pub const fn header(&self) -> &FdtHeader {
        &self.header
    }

    pub const fn total_size(&self) -> usize {
        self.header.total_size
    }

    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn memory_reservations(&self) -> MemoryReservationIter<'a> {
        MemoryReservationIter {
            bytes: self.bytes,
            offset: self.header.reservation_map_offset(),
            end: self.header.structure_offset(),
            finished: false,
        }
    }
}

impl FdtBlob<'static> {
    /// 从当前地址空间中已经映射的虚拟指针创建 FDT。
    ///
    /// # Safety
    ///
    /// 调用者必须保证整个 FDT 范围可读，并且映射在返回值的
    /// 生命周期内保持有效。
    pub unsafe fn from_ptr(pointer: *const u8) -> Result<Self, FdtError> {
        let base = pointer as usize;

        base.checked_add(FDT_HEADER_SIZE)
            .ok_or(FdtError::AddressOverflow)?;

        let mut prefix = [0_u8; FDT_HEADER_SIZE];

        for (offset, byte) in prefix.iter_mut().enumerate() {
            let address = base.checked_add(offset).ok_or(FdtError::AddressOverflow)?;

            // SAFETY:
            // 调用者保证 pointer 指向可读的 FDT header。
            *byte = unsafe { read_volatile(address as *const u8) };
        }

        let header = parse_header(&prefix)?;

        base.checked_add(header.total_size)
            .ok_or(FdtError::AddressOverflow)?;

        // SAFETY:
        // 调用者保证整个 FDT 范围均已映射且在返回值存活期间有效。
        let bytes = unsafe { slice::from_raw_parts(pointer, header.total_size) };

        Self::from_bytes(bytes)
    }
}

fn parse_header(bytes: &[u8]) -> Result<FdtHeader, FdtError> {
    if bytes.len() < FDT_HEADER_SIZE {
        return Err(FdtError::HeaderTooSmall);
    }

    let magic = read_be_u32(bytes, 0);

    if magic != FDT_MAGIC {
        return Err(FdtError::InvalidMagic { found: magic });
    }

    let total_size = read_be_u32(bytes, 4) as usize;

    if total_size < FDT_HEADER_SIZE {
        return Err(FdtError::TotalSizeTooSmall { size: total_size });
    }

    if total_size > MAX_FDT_SIZE {
        return Err(FdtError::TotalSizeTooLarge { size: total_size });
    }

    Ok(FdtHeader {
        total_size,

        structure_offset: read_be_u32(bytes, 8) as usize,

        strings_offset: read_be_u32(bytes, 12) as usize,

        reservation_map_offset: read_be_u32(bytes, 16) as usize,

        version: read_be_u32(bytes, 20),

        last_compatible_version: read_be_u32(bytes, 24),

        boot_cpu_id: read_be_u32(bytes, 28),

        strings_size: read_be_u32(bytes, 32) as usize,

        structure_size: read_be_u32(bytes, 36) as usize,
    })
}

fn validate_header(header: &FdtHeader) -> Result<(), FdtError> {
    if !range_is_inside(
        header.structure_offset(),
        header.structure_size(),
        header.total_size(),
    ) {
        return Err(FdtError::InvalidStructureRange);
    }

    if !range_is_inside(
        header.strings_offset(),
        header.strings_size(),
        header.total_size(),
    ) {
        return Err(FdtError::InvalidStringsRange);
    }

    let reservation_offset = header.reservation_map_offset();

    let reservation_end = header.structure_offset();

    if !reservation_offset.is_multiple_of(8)
        || reservation_offset >= reservation_end
        || reservation_end > header.total_size()
        || reservation_end - reservation_offset < 16
    {
        return Err(FdtError::InvalidReservationMap);
    }

    Ok(())
}

/// FDT memory reservation block 迭代器。
///
/// 每一项包含两个大端 u64：
///
/// - 物理起始地址；
/// - 区域大小。
///
/// `(0, 0)` 表示结束。
pub struct MemoryReservationIter<'a> {
    bytes: &'a [u8],
    offset: usize,
    end: usize,
    finished: bool,
}

impl Iterator for MemoryReservationIter<'_> {
    type Item = Result<MemoryRegion, FdtError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            let Some(entry_end) = self.offset.checked_add(16) else {
                self.finished = true;
                return Some(Err(FdtError::AddressOverflow));
            };

            if entry_end > self.end || entry_end > self.bytes.len() {
                self.finished = true;

                return Some(Err(FdtError::UnterminatedReservationMap));
            }

            let address = read_be_u64(self.bytes, self.offset);

            let size = read_be_u64(self.bytes, self.offset + 8);

            self.offset = entry_end;

            if address == 0 && size == 0 {
                self.finished = true;
                return None;
            }

            /*
             * 非终止的零长度项没有实际保留效果。
             */
            if size == 0 {
                continue;
            }

            let start = match usize::try_from(address) {
                Ok(start) => start,

                Err(_) => {
                    self.finished = true;

                    return Some(Err(FdtError::AddressOverflow));
                }
            };

            let size = match usize::try_from(size) {
                Ok(size) => size,

                Err(_) => {
                    self.finished = true;

                    return Some(Err(FdtError::AddressOverflow));
                }
            };

            if start.checked_add(size).is_none() {
                self.finished = true;

                return Some(Err(FdtError::AddressOverflow));
            }

            return Some(Ok(MemoryRegion::new(start, size)));
        }
    }
}

fn read_be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn range_is_inside(offset: usize, size: usize, total: usize) -> bool {
    offset.checked_add(size).is_some_and(|end| end <= total)
}

fn read_be_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
