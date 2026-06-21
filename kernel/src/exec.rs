// SUDOOS_M16A_ELF_AUXV_PATCH_V1
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use myos_mm::{PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

use crate::process::{Process, Thread};
use crate::user_mm::{UserMm, UserMmRuntimeError};

const AT_NULL: usize = 0;
const AT_PHDR: usize = 3;
const AT_PHENT: usize = 4;
const AT_PHNUM: usize = 5;
const AT_PAGESZ: usize = 6;
const AT_BASE: usize = 7;
const AT_FLAGS: usize = 8;
const AT_ENTRY: usize = 9;
const AT_SECURE: usize = 23;
const AT_RANDOM: usize = 25;
const AT_EXECFN: usize = 31;
const DT_NULL: u64 = 0;
const DT_PLTRELSZ: u64 = 2;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_REL: u64 = 17;
const DT_RELSZ: u64 = 18;
const DT_JMPREL: u64 = 23;

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
const R_RELATIVE: u32 = 3;

#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecError {
    AddressOverflow,
    DynamicInterpreterUnsupported,
    Elf(crate::elf::ElfError),
    Initramfs(crate::initramfs::InitramfsError),
    InvalidStack,
    MetadataOutOfMemory,
    Process(crate::process::ProcessError),
    UserMm(UserMmRuntimeError),
    Vfs(myos_vfs::Errno),
}

impl From<crate::elf::ElfError> for ExecError {
    fn from(error: crate::elf::ElfError) -> Self {
        Self::Elf(error)
    }
}

impl From<crate::initramfs::InitramfsError> for ExecError {
    fn from(error: crate::initramfs::InitramfsError) -> Self {
        Self::Initramfs(error)
    }
}

impl From<UserMmRuntimeError> for ExecError {
    fn from(error: UserMmRuntimeError) -> Self {
        Self::UserMm(error)
    }
}

impl From<crate::process::ProcessError> for ExecError {
    fn from(error: crate::process::ProcessError) -> Self {
        Self::Process(error)
    }
}

impl From<myos_vfs::Errno> for ExecError {
    fn from(error: myos_vfs::Errno) -> Self {
        Self::Vfs(error)
    }
}

pub struct ExecConfig<'a> {
    pub argv: &'a [&'a str],
    pub envp: &'a [&'a str],
    pub stack: VirtRange,
    pub heap_start: VirtAddr,
    pub heap_limit: VirtAddr,
    pub extra_areas: &'a [VmArea],
}

pub struct ExecImage {
    pub process: Arc<Process>,
    pub thread: Arc<Thread>,
}

pub struct PreparedExec {
    pub mm: Box<UserMm>,
    pub entry: VirtAddr,
    pub stack: VirtRange,
    pub stack_pointer: VirtAddr,
}

pub fn kernel_execve_from_initramfs(
    archive: &[u8],
    path: &str,
    config: ExecConfig<'_>,
) -> Result<ExecImage, ExecError> {
    let initramfs = crate::initramfs::Initramfs::parse(archive)?;
    let file = initramfs.lookup_file_follow(path)?;
    exec_elf(file, config)
}

pub fn exec_elf(image: &[u8], config: ExecConfig<'_>) -> Result<ExecImage, ExecError> {
    let prepared = prepare_elf(image, config)?;
    let process = Process::create(prepared.mm);
    if let Err(error) = crate::fs::install_standard_fds(&process) {
        destroy_unique_process(process)?;
        return Err(error.into());
    }
    let thread = match process.create_initial_thread(prepared.entry, prepared.stack) {
        Ok(thread) => thread,
        Err(error) => {
            destroy_unique_process(process)?;
            return Err(error.into());
        }
    };
    thread
        .prepare_stack_pointer(prepared.stack_pointer)
        .expect("exec built an invalid initial user stack pointer");
    crate::process::assert_initial_pair(&process, &thread);
    Ok(ExecImage { process, thread })
}

pub fn prepare_elf(image: &[u8], config: ExecConfig<'_>) -> Result<PreparedExec, ExecError> {
    let elf = crate::elf::parse(image)?;

    // SUDOOS_M16A_CLIPPY_HOTFIX_V1: centralize M16-A dynamic handoff policy and consume the
    // parsed metadata in the real exec path instead of suppressing warnings.
    reject_dynamic_handoff_if_needed(&elf)?;

    let mut areas = Vec::new();
    areas
        .try_reserve(
            elf.areas
                .len()
                .checked_add(config.extra_areas.len())
                .and_then(|count| count.checked_add(1))
                .ok_or(ExecError::AddressOverflow)?,
        )
        .map_err(|_| ExecError::MetadataOutOfMemory)?;
    areas.extend_from_slice(&elf.areas);
    areas.extend_from_slice(config.extra_areas);
    areas.push(VmArea::new(
        config.stack,
        VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
        VmAreaKind::Stack,
    ));

    let mm = build_mm(&areas, config.heap_start, config.heap_limit)?;
    let stack_pointer = match (|| {
        for segment in &elf.segments {
            load_segment(&mm, image, *segment)?;
        }
        apply_static_pie_relocations(&mm, image, &elf)?;
        build_initial_stack(&mm, config.stack, config.argv, config.envp, &elf)
    })() {
        Ok(stack_pointer) => stack_pointer,
        Err(error) => {
            destroy_mm(mm)?;
            return Err(error);
        }
    };
    Ok(PreparedExec {
        mm,
        entry: elf.entry,
        stack: config.stack,
        stack_pointer,
    })
}

// SUDOOS_M16A_CLIPPY_HOTFIX_V1: M16-A records ET_DYN/PT_INTERP/PT_DYNAMIC metadata before the
// full interpreter/relocation/TLS path exists. Linux enters the interpreter for
// dynamic executables; this kernel must therefore fail closed until M16-B can
// load PT_INTERP, apply relocations, set up TLS, and seal RELRO.
fn reject_dynamic_handoff_if_needed(elf: &crate::elf::ElfImage) -> Result<(), ExecError> {
    match elf.kind {
        crate::elf::ElfKind::Executable => {
            if elf.load_bias != 0 {
                return Err(ExecError::Elf(crate::elf::ElfError::InvalidHeader));
            }
        }
        crate::elf::ElfKind::PositionIndependent => {
            if elf.load_bias == 0 {
                return Err(ExecError::Elf(crate::elf::ElfError::InvalidHeader));
            }
        }
    }

    if let Some(dynamic) = elf.dynamic {
        let dynamic_start = dynamic.virtual_address.get();
        let _dynamic_end = dynamic_start
            .checked_add(dynamic.memory_size)
            .ok_or(ExecError::AddressOverflow)?;
        let _dynamic_file_metadata_end = dynamic
            .file_offset
            .checked_add(dynamic.memory_size)
            .ok_or(ExecError::AddressOverflow)?;
    }
    Ok(())
}

fn build_mm(
    areas: &[VmArea],
    heap_start: VirtAddr,
    heap_limit: VirtAddr,
) -> Result<Box<UserMm>, ExecError> {
    let mm = Box::new(UserMm::new(areas)?);
    if let Err(error) = mm.configure_program_break(heap_start, heap_limit) {
        destroy_mm(mm)?;
        return Err(error.into());
    }
    Ok(mm)
}

fn load_segment(
    mm: &UserMm,
    image: &[u8],
    segment: crate::elf::LoadSegment,
) -> Result<(), ExecError> {
    let mut copied = 0;
    while copied < segment.file_size {
        let virtual_address = segment
            .virtual_address
            .checked_add(copied)
            .ok_or(ExecError::AddressOverflow)?;
        let in_page = virtual_address.get() & (PAGE_SIZE - 1);
        let chunk = core::cmp::min(PAGE_SIZE - in_page, segment.file_size - copied);
        let source_offset = segment
            .file_offset
            .checked_add(copied)
            .ok_or(ExecError::AddressOverflow)?;
        loader_copy_to_user_physical(
            mm,
            virtual_address,
            image
                .get(source_offset..source_offset + chunk)
                .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidSegment))?,
        )?;
        copied += chunk;
    }

    let page_start = segment
        .virtual_address
        .get()
        .checked_add(segment.file_size)
        .ok_or(ExecError::AddressOverflow)?;
    let page_end = segment
        .virtual_address
        .get()
        .checked_add(segment.memory_size)
        .ok_or(ExecError::AddressOverflow)?;
    if page_start < page_end && page_start & (PAGE_SIZE - 1) != 0 {
        mm.populate_page(VirtAddr::new(page_start))?;
    }
    Ok(())
}

fn apply_static_pie_relocations(
    mm: &UserMm,
    image: &[u8],
    elf: &crate::elf::ElfImage,
) -> Result<(), ExecError> {
    let Some(dynamic) = elf.dynamic else {
        return Ok(());
    };
    let entries = image
        .get(dynamic.file_offset..dynamic.file_offset + dynamic.memory_size)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader))?;
    let mut rela_vaddr = 0_usize;
    let mut rela_size = 0_usize;
    let mut rela_ent = 24_usize;
    let mut rel_size = 0_usize;
    let mut jmprel = 0_usize;
    let mut pltrel_size = 0_usize;

    for entry in entries.chunks_exact(16) {
        let tag = read_u64(entry, 0)?;
        let value = read_u64(entry, 8)?;
        match tag {
            DT_NULL => break,
            DT_RELA => {
                rela_vaddr = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?
            }
            DT_RELASZ => {
                rela_size = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?
            }
            DT_RELAENT => {
                rela_ent = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?
            }
            DT_REL => {
                let _ = value;
            }
            DT_RELSZ => {
                rel_size = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?
            }
            DT_JMPREL => jmprel = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?,
            DT_PLTRELSZ => {
                pltrel_size = usize::try_from(value).map_err(|_| ExecError::AddressOverflow)?
            }
            _ => {}
        }
    }

    if rel_size != 0 || jmprel != 0 || pltrel_size != 0 {
        return Ok(());
    }
    if rela_size == 0 {
        return Ok(());
    }
    if rela_ent != 24 || !rela_size.is_multiple_of(rela_ent) {
        return Err(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader));
    }

    let rela_addr = rela_vaddr
        .checked_add(elf.load_bias)
        .ok_or(ExecError::AddressOverflow)?;
    let rela_offset = virtual_to_file_offset(elf, VirtAddr::new(rela_addr), rela_size)?;
    let rela_bytes = image
        .get(rela_offset..rela_offset + rela_size)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidSegment))?;

    for entry in rela_bytes.chunks_exact(rela_ent) {
        let raw_offset = read_u64(entry, 0)?;
        let info = read_u64(entry, 8)?;
        let addend = read_i64(entry, 16)?;
        let relocation_type = (info & 0xffff_ffff) as u32;
        let symbol = info >> 32;
        if relocation_type != R_RELATIVE || symbol != 0 {
            return Ok(());
        }
        let destination = usize::try_from(raw_offset)
            .map_err(|_| ExecError::AddressOverflow)?
            .checked_add(elf.load_bias)
            .ok_or(ExecError::AddressOverflow)?;
        let value = (elf.load_bias as i128)
            .checked_add(addend as i128)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or(ExecError::AddressOverflow)?;
        loader_copy_to_user_physical(mm, VirtAddr::new(destination), &value.to_le_bytes())?;
    }

    Ok(())
}

fn virtual_to_file_offset(
    elf: &crate::elf::ElfImage,
    address: VirtAddr,
    size: usize,
) -> Result<usize, ExecError> {
    let end = address
        .get()
        .checked_add(size)
        .ok_or(ExecError::AddressOverflow)?;
    for segment in &elf.segments {
        let start = segment.virtual_address.get();
        let file_end = start
            .checked_add(segment.file_size)
            .ok_or(ExecError::AddressOverflow)?;
        if address.get() >= start && end <= file_end {
            return segment
                .file_offset
                .checked_add(address.get() - start)
                .ok_or(ExecError::AddressOverflow);
        }
    }
    Err(ExecError::Elf(crate::elf::ElfError::InvalidSegment))
}

fn build_initial_stack(
    mm: &UserMm,
    stack: VirtRange,
    argv: &[&str],
    envp: &[&str],
    elf: &crate::elf::ElfImage,
) -> Result<VirtAddr, ExecError> {
    if argv.is_empty() {
        return Err(ExecError::InvalidStack);
    }
    for value in argv.iter().chain(envp.iter()) {
        if value.is_empty() || value.as_bytes().contains(&0) {
            return Err(ExecError::InvalidStack);
        }
    }
    if stack.is_empty() || stack.end().get() & 0xf != 0 {
        return Err(ExecError::InvalidStack);
    }

    let mut cursor = stack.end().get();

    let mut argv_ptrs = Vec::new();
    argv_ptrs
        .try_reserve(argv.len())
        .map_err(|_| ExecError::MetadataOutOfMemory)?;
    for value in argv.iter().rev() {
        argv_ptrs.push(push_stack_string(mm, stack, &mut cursor, value)?);
    }
    argv_ptrs.reverse();

    let mut envp_ptrs = Vec::new();
    envp_ptrs
        .try_reserve(envp.len())
        .map_err(|_| ExecError::MetadataOutOfMemory)?;
    for value in envp.iter().rev() {
        envp_ptrs.push(push_stack_string(mm, stack, &mut cursor, value)?);
    }
    envp_ptrs.reverse();

    let execfn_ptr = argv_ptrs[0];

    let random = build_at_random_bytes(elf.entry, stack, execfn_ptr);
    let random_ptr = push_stack_bytes(mm, stack, &mut cursor, &random)?;

    cursor = align_down(cursor, 16);

    let phdr = elf
        .program_headers
        .map(|info| info.virtual_address.get())
        .unwrap_or(0);
    let phent = elf.program_headers.map(|info| info.entry_size).unwrap_or(0);
    let phnum = elf.program_headers.map(|info| info.count).unwrap_or(0);

    let auxv = [
        (AT_PHDR, phdr),
        (AT_PHENT, phent),
        (AT_PHNUM, phnum),
        (AT_BASE, 0),
        (AT_FLAGS, 0),
        (AT_ENTRY, elf.entry.get()),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_SECURE, 0),
        (AT_RANDOM, random_ptr),
        (AT_EXECFN, execfn_ptr),
    ];

    let mut words = Vec::new();
    words
        .try_reserve(1 + argv_ptrs.len() + 1 + envp_ptrs.len() + 1 + auxv.len() * 2 + 2)
        .map_err(|_| ExecError::MetadataOutOfMemory)?;

    // argc
    words.push(argv_ptrs.len());
    // argv[] + NULL
    for pointer in argv_ptrs {
        words.push(pointer);
    }
    words.push(0);
    for pointer in envp_ptrs {
        words.push(pointer);
    }
    words.push(0);
    for (key, value) in auxv {
        words.push(key);
        words.push(value);
    }
    words.push(AT_NULL);
    words.push(0);

    let words_len = words
        .len()
        .checked_mul(core::mem::size_of::<usize>())
        .ok_or(ExecError::AddressOverflow)?;
    let stack_pointer = align_down(
        cursor
            .checked_sub(words_len)
            .ok_or(ExecError::InvalidStack)?,
        16,
    );
    validate_stack_span(stack, stack_pointer, words_len)?;

    let mut raw_words = Vec::new();
    raw_words
        .try_reserve(words_len)
        .map_err(|_| ExecError::MetadataOutOfMemory)?;
    for word in words {
        raw_words.extend_from_slice(&word.to_le_bytes());
    }
    loader_copy_to_user_physical(mm, VirtAddr::new(stack_pointer), &raw_words)?;

    Ok(VirtAddr::new(stack_pointer))
}

fn push_stack_string(
    mm: &UserMm,
    stack: VirtRange,
    cursor: &mut usize,
    value: &str,
) -> Result<usize, ExecError> {
    let bytes = value.as_bytes();
    let total = bytes
        .len()
        .checked_add(1)
        .ok_or(ExecError::AddressOverflow)?;
    let start = cursor.checked_sub(total).ok_or(ExecError::InvalidStack)?;
    validate_stack_span(stack, start, total)?;
    loader_copy_to_user_physical(mm, VirtAddr::new(start), bytes)?;
    loader_copy_to_user_physical(mm, VirtAddr::new(start + bytes.len()), &[0])?;
    *cursor = start;
    Ok(start)
}

fn push_stack_bytes(
    mm: &UserMm,
    stack: VirtRange,
    cursor: &mut usize,
    bytes: &[u8],
) -> Result<usize, ExecError> {
    let start = cursor
        .checked_sub(bytes.len())
        .ok_or(ExecError::InvalidStack)?;
    validate_stack_span(stack, start, bytes.len())?;
    loader_copy_to_user_physical(mm, VirtAddr::new(start), bytes)?;
    *cursor = start;
    Ok(start)
}

fn validate_stack_span(stack: VirtRange, start: usize, length: usize) -> Result<(), ExecError> {
    if length == 0 {
        return Ok(());
    }
    let end = start
        .checked_add(length)
        .ok_or(ExecError::AddressOverflow)?;
    if end > stack.end().get()
        || !stack.contains(VirtAddr::new(start))
        || !stack.contains(VirtAddr::new(end - 1))
    {
        return Err(ExecError::InvalidStack);
    }
    Ok(())
}

fn build_at_random_bytes(entry: VirtAddr, stack: VirtRange, execfn: usize) -> [u8; 16] {
    // Temporary entropy seed until the kernel has a proper RNG. This is still
    // better than omitting AT_RANDOM because musl/glibc-style runtimes expect
    // the 16-byte object to exist. M16-B should replace the seed with real
    // boot/runtime entropy before any security claim.
    let mut state = (entry.get() as u64)
        ^ ((stack.end().get() as u64).rotate_left(17))
        ^ ((execfn as u64).rotate_right(11))
        ^ 0x9e37_79b9_7f4a_7c15;
    let mut out = [0_u8; 16];
    for chunk in out.chunks_mut(8) {
        state ^= state << 7;
        state ^= state >> 9;
        state = state.wrapping_mul(0xd6e8_feb8_6659_fd93);
        let bytes = state.to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    out
}

fn loader_copy_to_user_physical(
    mm: &UserMm,
    address: VirtAddr,
    bytes: &[u8],
) -> Result<(), ExecError> {
    let mut copied = 0;
    while copied < bytes.len() {
        let current = address
            .checked_add(copied)
            .ok_or(ExecError::AddressOverflow)?;
        let in_page = current.get() & (PAGE_SIZE - 1);
        let chunk = core::cmp::min(PAGE_SIZE - in_page, bytes.len() - copied);
        let physical = mm.populate_page(current)?;
        let destination = crate::arch::memory::phys_access::ram_mut_ptr::<u8>(physical)
            .map_err(|_| UserMmRuntimeError::NotMapped)?;
        // SAFETY: `populate_page` returned the RAM address for this exact user
        // VA and this iteration is bounded to the containing page.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(copied), destination, chunk);
        }
        copied += chunk;
    }
    Ok(())
}

fn destroy_unique_process(process: Arc<Process>) -> Result<(), ExecError> {
    let process = Arc::try_unwrap(process)
        .unwrap_or_else(|_| panic!("exec retained Process after construction failure"));
    process.destroy()?;
    Ok(())
}

fn destroy_mm(mut mm: Box<UserMm>) -> Result<(), ExecError> {
    mm.destroy()?;
    Ok(())
}

fn align_down(value: usize, align: usize) -> usize {
    value & !(align - 1)
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ExecError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader))?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, ExecError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader))?;
    Ok(i64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}
