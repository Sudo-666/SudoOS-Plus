// SUDOOS_M16A_ELF_AUXV_PATCH_V1
use alloc::{string::String, vec::Vec};
use myos_mm::{PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

const EI_CLASS: usize = 4;
const EI_DATA: usize = 5;
const EI_VERSION: usize = 6;
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

#[cfg(target_arch = "riscv64")]
const TARGET_MACHINE: u16 = 243;
#[cfg(target_arch = "loongarch64")]
const TARGET_MACHINE: u16 = 258;

const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_PHDR: u32 = 6;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

pub const ELF_HEADER_LEN: usize = 64;
pub const PROGRAM_HEADER_LEN: usize = 56;
const MAX_PHDRS: usize = 32;
const MAX_INTERP_PATH: usize = 256;

// Conservative fixed load-bias for the first M16-A ET_DYN stage.
// Later M16-B should replace this with the mmap gap allocator so the dynamic
// linker, main PIE, stack, brk, and shared objects cannot collide.
const ET_DYN_LOAD_BIAS: usize = 0x4000_0000;

#[derive(Debug, PartialEq, Eq)]
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

impl ElfError {
    /// Short diagnostic tag for rate-limited execve tracing.
    pub fn reason(&self) -> &'static str {
        match self {
            ElfError::AddressOverflow => "elf-address-overflow",
            ElfError::InvalidAlignment => "elf-invalid-alignment",
            ElfError::InvalidHeader => "elf-invalid-header",
            ElfError::InvalidMachine => "elf-invalid-machine",
            ElfError::InvalidProgramHeader => "elf-bad-program-header",
            ElfError::InvalidSegment => "elf-invalid-segment",
            ElfError::NoLoadSegments => "elf-no-load-segments",
            ElfError::OutOfMemory => "elf-out-of-memory",
            ElfError::Unsupported => "elf-unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfKind {
    Executable,
    PositionIndependent,
}

#[derive(Clone, Copy)]
pub struct LoadSegment {
    pub range: VirtRange,
    pub flags: VmAreaFlags,
    pub virtual_address: VirtAddr,
    pub memory_size: usize,
    pub file_offset: usize,
    pub file_size: usize,
}

#[derive(Clone, Copy)]
pub struct ProgramHeaderInfo {
    pub virtual_address: VirtAddr,
    pub entry_size: usize,
    pub count: usize,
}

#[derive(Clone, Copy)]
pub struct DynamicInfo {
    pub virtual_address: VirtAddr,
    pub memory_size: usize,
    pub file_offset: usize,
}

pub struct ElfImage {
    pub kind: ElfKind,
    pub entry: VirtAddr,
    pub load_bias: usize,
    pub program_headers: Option<ProgramHeaderInfo>,
    #[allow(dead_code)]
    pub interpreter: Option<String>,
    pub dynamic: Option<DynamicInfo>,
    pub areas: Vec<VmArea>,
    pub segments: Vec<LoadSegment>,
}

/// Parse an ELF file using a custom load bias instead of the default
/// ET_DYN_LOAD_BIAS. This is used by the dynamic-linker loader so the
/// interpreter and the main PIE can occupy non-overlapping address ranges.
pub fn parse_with_bias(image: &[u8], load_bias: usize) -> Result<ElfImage, ElfError> {
    parse_impl(image, Some(load_bias))
}

pub fn parse(image: &[u8]) -> Result<ElfImage, ElfError> {
    parse_impl(image, None)
}

fn parse_impl(image: &[u8], bias_override: Option<usize>) -> Result<ElfImage, ElfError> {
    let header = image.get(..ELF_HEADER_LEN).ok_or(ElfError::InvalidHeader)?;
    if header.get(..4) != Some(b"\x7fELF")
        || header[EI_CLASS] != ELFCLASS64
        || header[EI_DATA] != ELFDATA2LSB
        || header[EI_VERSION] != EV_CURRENT
    {
        return Err(ElfError::InvalidHeader);
    }

    let file_type = read_u16(header, 16)?;
    let (kind, load_bias) = match file_type {
        ET_EXEC => (ElfKind::Executable, 0),
        ET_DYN => {
            let bias = bias_override.unwrap_or(ET_DYN_LOAD_BIAS);
            (ElfKind::PositionIndependent, bias)
        }
        _ => return Err(ElfError::Unsupported),
    };

    if read_u16(header, 18)? != target_machine() {
        return Err(ElfError::InvalidMachine);
    }
    if read_u32(header, 20)? != EV_CURRENT as u32 {
        return Err(ElfError::InvalidHeader);
    }

    let raw_entry = read_u64(header, 24)? as usize;
    let entry = VirtAddr::new(apply_load_bias(raw_entry, load_bias)?);
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

    let phdr_length = phentsize
        .checked_mul(phnum)
        .ok_or(ElfError::AddressOverflow)?;
    let phdr_end = phoff
        .checked_add(phdr_length)
        .ok_or(ElfError::AddressOverflow)?;
    if phdr_end > image.len() {
        return Err(ElfError::InvalidProgramHeader);
    }

    let mut areas = Vec::new();
    let mut segments = Vec::new();
    let mut program_headers = None;
    let mut interpreter = None;
    let mut dynamic = None;

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

        let segment_type = read_u32(phdr, 0)?;
        let raw_flags = read_u32(phdr, 4)?;
        let file_offset = read_u64(phdr, 8)? as usize;
        let virtual_address = read_u64(phdr, 16)? as usize;
        let file_size = read_u64(phdr, 32)? as usize;
        let memory_size = read_u64(phdr, 40)? as usize;
        let align = read_u64(phdr, 48)? as usize;

        match segment_type {
            PT_LOAD => {
                validate_segment(
                    image,
                    file_offset,
                    virtual_address,
                    file_size,
                    memory_size,
                    align,
                )?;
                let adjusted_virtual_address = apply_load_bias(virtual_address, load_bias)?;
                let start = align_down(adjusted_virtual_address, PAGE_SIZE);
                let segment_end = adjusted_virtual_address
                    .checked_add(memory_size)
                    .ok_or(ElfError::AddressOverflow)?;
                let end = align_up(segment_end, PAGE_SIZE).ok_or(ElfError::AddressOverflow)?;
                let range = VirtRange::from_bounds(start, end);
                if !crate::arch::memory::layout::USER_RANGE.contains_range(range) {
                    return Err(ElfError::InvalidSegment);
                }

                let flags = segment_flags(raw_flags)?;
                let area = VmArea::new(range, flags, VmAreaKind::FileBacked {
                    object: 1,
                    offset: align_down(file_offset, PAGE_SIZE) as u64,
                });
                reject_area_overlap(&areas, area.range())?;

                areas.try_reserve(1).map_err(|_| ElfError::OutOfMemory)?;
                segments.try_reserve(1).map_err(|_| ElfError::OutOfMemory)?;
                areas.push(area);
                segments.push(LoadSegment {
                    range,
                    flags,
                    virtual_address: VirtAddr::new(adjusted_virtual_address),
                    memory_size,
                    file_offset,
                    file_size,
                });

                if program_headers.is_none() && phoff >= file_offset {
                    let file_end = file_offset
                        .checked_add(file_size)
                        .ok_or(ElfError::AddressOverflow)?;
                    if phdr_end <= file_end {
                        let delta = phoff - file_offset;
                        let phdr_virtual = adjusted_virtual_address
                            .checked_add(delta)
                            .ok_or(ElfError::AddressOverflow)?;
                        let phdr_address = VirtAddr::new(phdr_virtual);
                        if crate::arch::memory::layout::USER_RANGE.contains(phdr_address) {
                            program_headers = Some(ProgramHeaderInfo {
                                virtual_address: phdr_address,
                                entry_size: phentsize,
                                count: phnum,
                            });
                        }
                    }
                }
            }
            PT_INTERP => {
                if interpreter.is_some() || memory_size < file_size {
                    return Err(ElfError::InvalidProgramHeader);
                }
                interpreter = Some(read_interpreter(image, file_offset, file_size)?);
            }
            PT_PHDR => {
                if program_headers.is_some() || memory_size < phdr_length {
                    return Err(ElfError::InvalidProgramHeader);
                }
                let phdr_virtual = VirtAddr::new(apply_load_bias(virtual_address, load_bias)?);
                if !crate::arch::memory::layout::USER_RANGE.contains(phdr_virtual) {
                    return Err(ElfError::InvalidProgramHeader);
                }
                program_headers = Some(ProgramHeaderInfo {
                    virtual_address: phdr_virtual,
                    entry_size: phentsize,
                    count: phnum,
                });
            }
            PT_DYNAMIC => {
                if dynamic.is_some() || memory_size == 0 {
                    return Err(ElfError::InvalidProgramHeader);
                }
                let dynamic_virtual = VirtAddr::new(apply_load_bias(virtual_address, load_bias)?);
                if !crate::arch::memory::layout::USER_RANGE.contains(dynamic_virtual) {
                    return Err(ElfError::InvalidProgramHeader);
                }
                dynamic = Some(DynamicInfo {
                    virtual_address: dynamic_virtual,
                    memory_size,
                    file_offset,
                });
            }
            _ => {}
        }
    }

    if segments.is_empty() {
        return Err(ElfError::NoLoadSegments);
    }

    Ok(ElfImage {
        kind,
        entry,
        load_bias,
        program_headers,
        interpreter,
        dynamic,
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

fn read_interpreter(
    image: &[u8],
    file_offset: usize,
    file_size: usize,
) -> Result<String, ElfError> {
    if file_size == 0 || file_size > MAX_INTERP_PATH {
        return Err(ElfError::InvalidProgramHeader);
    }
    let bytes = image
        .get(file_offset..file_offset + file_size)
        .ok_or(ElfError::InvalidProgramHeader)?;
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(ElfError::InvalidProgramHeader)?;
    if nul == 0 {
        return Err(ElfError::InvalidProgramHeader);
    }
    let path = core::str::from_utf8(&bytes[..nul]).map_err(|_| ElfError::InvalidProgramHeader)?;
    let mut result = String::new();
    result
        .try_reserve(path.len())
        .map_err(|_| ElfError::OutOfMemory)?;
    result.push_str(path);
    Ok(result)
}

fn reject_area_overlap(areas: &[VmArea], range: VirtRange) -> Result<(), ElfError> {
    for area in areas {
        let existing = area.range();
        if range.start().get() < existing.end().get() && existing.start().get() < range.end().get()
        {
            return Err(ElfError::InvalidSegment);
        }
    }
    Ok(())
}

fn segment_flags(raw: u32) -> Result<VmAreaFlags, ElfError> {
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
    write_u32(header, 20, EV_CURRENT as u32);
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

fn apply_load_bias(value: usize, load_bias: usize) -> Result<usize, ElfError> {
    value
        .checked_add(load_bias)
        .ok_or(ElfError::AddressOverflow)
}
