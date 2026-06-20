use alloc::vec::Vec;

use myos_mm::{PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

const EI_CLASS: usize = 4;
const EI_DATA: usize = 5;
const EI_VERSION: usize = 6;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;

const ET_EXEC: u16 = 2;
#[cfg(target_arch = "riscv64")]
const TARGET_MACHINE: u16 = 243;
#[cfg(target_arch = "loongarch64")]
const TARGET_MACHINE: u16 = 258;

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

const ELF_HEADER_LEN: usize = 64;
const PROGRAM_HEADER_LEN: usize = 56;
const MAX_PHDRS: usize = 16;

#[derive(Debug)]
pub enum ElfError {
    AddressOverflow,
    InvalidAlignment,
    InvalidHeader,
    InvalidMachine,
    InvalidProgramHeader,
    InvalidSegment,
    NoLoadSegments,
    OutOfMemory,
    Unsupported,
}

#[derive(Clone, Copy)]
pub struct LoadSegment {
    pub virtual_address: VirtAddr,
    pub memory_size: usize,
    pub file_offset: usize,
    pub file_size: usize,
}

pub struct ElfImage {
    pub entry: VirtAddr,
    pub areas: Vec<VmArea>,
    pub segments: Vec<LoadSegment>,
}

pub fn parse(image: &[u8]) -> Result<ElfImage, ElfError> {
    let header = image.get(..ELF_HEADER_LEN).ok_or(ElfError::InvalidHeader)?;
    if header.get(..4) != Some(b"\x7fELF")
        || header[EI_CLASS] != ELFCLASS64
        || header[EI_DATA] != ELFDATA2LSB
        || header[EI_VERSION] != EV_CURRENT
    {
        return Err(ElfError::InvalidHeader);
    }
    if read_u16(header, 16)? != ET_EXEC {
        return Err(ElfError::Unsupported);
    }
    if read_u16(header, 18)? != target_machine() {
        return Err(ElfError::InvalidMachine);
    }
    if read_u32(header, 20)? != 1 {
        return Err(ElfError::InvalidHeader);
    }

    let entry = VirtAddr::new(read_u64(header, 24)? as usize);
    if !crate::arch::memory::layout::USER_RANGE.contains(entry) {
        return Err(ElfError::InvalidHeader);
    }
    let phoff = read_u64(header, 32)? as usize;
    let ehsize = read_u16(header, 52)? as usize;
    let phentsize = read_u16(header, 54)? as usize;
    let phnum = read_u16(header, 56)? as usize;
    if ehsize != ELF_HEADER_LEN
        || phentsize != PROGRAM_HEADER_LEN
        || phnum == 0
        || phnum > MAX_PHDRS
    {
        return Err(ElfError::InvalidProgramHeader);
    }

    let phdr_bytes = phentsize
        .checked_mul(phnum)
        .and_then(|length| phoff.checked_add(length))
        .ok_or(ElfError::AddressOverflow)?;
    if phdr_bytes > image.len() {
        return Err(ElfError::InvalidProgramHeader);
    }

    let mut areas = Vec::new();
    let mut segments = Vec::new();
    for index in 0..phnum {
        let offset = phoff
            .checked_add(
                index
                    .checked_mul(phentsize)
                    .ok_or(ElfError::AddressOverflow)?,
            )
            .ok_or(ElfError::AddressOverflow)?;
        let phdr = image
            .get(offset..offset + PROGRAM_HEADER_LEN)
            .ok_or(ElfError::InvalidProgramHeader)?;
        if read_u32(phdr, 0)? != PT_LOAD {
            continue;
        }
        let raw_flags = read_u32(phdr, 4)?;
        let file_offset = read_u64(phdr, 8)? as usize;
        let virtual_address = read_u64(phdr, 16)? as usize;
        let file_size = read_u64(phdr, 32)? as usize;
        let memory_size = read_u64(phdr, 40)? as usize;
        let align = read_u64(phdr, 48)? as usize;
        validate_segment(
            image,
            file_offset,
            virtual_address,
            file_size,
            memory_size,
            align,
        )?;

        let start = align_down(virtual_address, PAGE_SIZE);
        let end = align_up(
            virtual_address
                .checked_add(memory_size)
                .ok_or(ElfError::AddressOverflow)?,
            PAGE_SIZE,
        )
        .ok_or(ElfError::AddressOverflow)?;
        let range = VirtRange::from_bounds(start, end);
        if !crate::arch::memory::layout::USER_RANGE.contains_range(range) {
            return Err(ElfError::InvalidSegment);
        }

        let flags = segment_flags(raw_flags)?;
        let area = VmArea::new(
            range,
            flags,
            VmAreaKind::FileBacked {
                object: 1,
                offset: align_down(file_offset, PAGE_SIZE) as u64,
            },
        );
        areas.try_reserve(1).map_err(|_| ElfError::OutOfMemory)?;
        segments.try_reserve(1).map_err(|_| ElfError::OutOfMemory)?;
        areas.push(area);
        segments.push(LoadSegment {
            virtual_address: VirtAddr::new(virtual_address),
            memory_size,
            file_offset,
            file_size,
        });
    }

    if segments.is_empty() {
        return Err(ElfError::NoLoadSegments);
    }
    Ok(ElfImage {
        entry,
        areas,
        segments,
    })
}

pub fn build_static_exec(
    entry: VirtAddr,
    code: &[u8],
    data_vaddr: VirtAddr,
) -> Result<Vec<u8>, ElfError> {
    if code.is_empty() || code.len() > PAGE_SIZE {
        return Err(ElfError::InvalidSegment);
    }
    let code_offset = PAGE_SIZE;
    let data_offset = PAGE_SIZE * 2;
    let mut image = Vec::new();
    image
        .try_reserve(PAGE_SIZE * 3)
        .map_err(|_| ElfError::OutOfMemory)?;
    image.resize(PAGE_SIZE, 0);
    write_elf_header(&mut image[..ELF_HEADER_LEN], entry);
    write_program_header(
        &mut image[ELF_HEADER_LEN..ELF_HEADER_LEN + PROGRAM_HEADER_LEN],
        PF_R | PF_X,
        code_offset,
        align_down(entry.get(), PAGE_SIZE),
        code.len(),
        PAGE_SIZE,
    );
    write_program_header(
        &mut image[ELF_HEADER_LEN + PROGRAM_HEADER_LEN..ELF_HEADER_LEN + PROGRAM_HEADER_LEN * 2],
        PF_R | PF_W,
        data_offset,
        data_vaddr.get(),
        PAGE_SIZE,
        PAGE_SIZE,
    );
    image.extend_from_slice(code);
    image.resize(data_offset, 0);
    image.resize(data_offset + PAGE_SIZE, 0);
    Ok(image)
}

fn validate_segment(
    image: &[u8],
    file_offset: usize,
    virtual_address: usize,
    file_size: usize,
    memory_size: usize,
    align: usize,
) -> Result<(), ElfError> {
    if memory_size == 0 || file_size > memory_size {
        return Err(ElfError::InvalidSegment);
    }
    file_offset
        .checked_add(file_size)
        .filter(|end| *end <= image.len())
        .ok_or(ElfError::InvalidSegment)?;
    virtual_address
        .checked_add(memory_size)
        .ok_or(ElfError::AddressOverflow)?;
    if align > 1 {
        if !align.is_power_of_two() {
            return Err(ElfError::InvalidAlignment);
        }
        if (file_offset & (align - 1)) != (virtual_address & (align - 1)) {
            return Err(ElfError::InvalidAlignment);
        }
    }
    Ok(())
}

fn segment_flags(raw: u32) -> Result<VmAreaFlags, ElfError> {
    if raw & PF_W != 0 && raw & PF_X != 0 {
        return Err(ElfError::InvalidSegment);
    }
    let mut flags = VmAreaFlags::USER.union(VmAreaFlags::PRIVATE);
    if raw & PF_R != 0 || raw & (PF_W | PF_X) != 0 {
        flags = flags.union(VmAreaFlags::READ);
    }
    if raw & PF_W != 0 {
        flags = flags.union(VmAreaFlags::WRITE);
    }
    if raw & PF_X != 0 {
        flags = flags.union(VmAreaFlags::EXECUTE);
    }
    Ok(flags)
}

fn write_elf_header(header: &mut [u8], entry: VirtAddr) {
    header[..4].copy_from_slice(b"\x7fELF");
    header[EI_CLASS] = ELFCLASS64;
    header[EI_DATA] = ELFDATA2LSB;
    header[EI_VERSION] = EV_CURRENT;
    write_u16(header, 16, ET_EXEC);
    write_u16(header, 18, target_machine());
    write_u32(header, 20, 1);
    write_u64(header, 24, entry.get() as u64);
    write_u64(header, 32, ELF_HEADER_LEN as u64);
    write_u16(header, 52, ELF_HEADER_LEN as u16);
    write_u16(header, 54, PROGRAM_HEADER_LEN as u16);
    write_u16(header, 56, 2);
}

fn write_program_header(
    header: &mut [u8],
    flags: u32,
    offset: usize,
    vaddr: usize,
    file_size: usize,
    memory_size: usize,
) {
    write_u32(header, 0, PT_LOAD);
    write_u32(header, 4, flags);
    write_u64(header, 8, offset as u64);
    write_u64(header, 16, vaddr as u64);
    write_u64(header, 24, 0);
    write_u64(header, 32, file_size as u64);
    write_u64(header, 40, memory_size as u64);
    write_u64(header, 48, PAGE_SIZE as u64);
}

const fn target_machine() -> u16 {
    TARGET_MACHINE
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ElfError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or(ElfError::InvalidHeader)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ElfError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or(ElfError::InvalidHeader)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ElfError> {
    let bytes = bytes
        .get(offset..offset + 8)
        .ok_or(ElfError::InvalidHeader)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}
