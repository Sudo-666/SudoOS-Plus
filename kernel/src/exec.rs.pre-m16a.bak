use alloc::{boxed::Box, sync::Arc, vec::Vec};

use myos_mm::{PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

use crate::process::{Process, Thread};
use crate::user_mm::{UserMm, UserMmRuntimeError};

const AT_NULL: usize = 0;
const AT_PAGESZ: usize = 6;
const AT_ENTRY: usize = 9;

#[derive(Debug)]
#[allow(dead_code)]
pub enum ExecError {
    AddressOverflow,
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
    pub argv0: &'a str,
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
    let file = initramfs.lookup(path)?;
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
    for segment in &elf.segments {
        load_segment(&mm, image, *segment)?;
    }
    let stack_pointer = build_initial_stack(&mm, config.stack, config.argv0, elf.entry)?;
    Ok(PreparedExec {
        mm,
        entry: elf.entry,
        stack: config.stack,
        stack_pointer,
    })
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

fn build_initial_stack(
    mm: &UserMm,
    stack: VirtRange,
    argv0: &str,
    entry: VirtAddr,
) -> Result<VirtAddr, ExecError> {
    if stack.is_empty() || stack.end().get() & 0xf != 0 {
        return Err(ExecError::InvalidStack);
    }
    let argv0_bytes = argv0.as_bytes();
    let string_len = argv0_bytes
        .len()
        .checked_add(1)
        .ok_or(ExecError::AddressOverflow)?;
    let string_start = stack
        .end()
        .get()
        .checked_sub(string_len)
        .ok_or(ExecError::InvalidStack)?;
    let aligned_strings = align_down(string_start, 16);
    let words = [
        1,
        string_start,
        0,
        0,
        AT_PAGESZ,
        PAGE_SIZE,
        AT_ENTRY,
        entry.get(),
        AT_NULL,
        0,
    ];
    let words_len = words
        .len()
        .checked_mul(core::mem::size_of::<usize>())
        .ok_or(ExecError::AddressOverflow)?;
    let stack_pointer = aligned_strings
        .checked_sub(words_len)
        .ok_or(ExecError::InvalidStack)?;
    if !stack.contains(VirtAddr::new(stack_pointer)) {
        return Err(ExecError::InvalidStack);
    }

    let mut raw_words = [0_u8; 10 * core::mem::size_of::<usize>()];
    for (index, word) in words.iter().enumerate() {
        let start = index * core::mem::size_of::<usize>();
        raw_words[start..start + core::mem::size_of::<usize>()]
            .copy_from_slice(&word.to_le_bytes());
    }
    loader_copy_to_user_physical(mm, VirtAddr::new(stack_pointer), &raw_words)?;
    loader_copy_to_user_physical(mm, VirtAddr::new(string_start), argv0_bytes)?;
    loader_copy_to_user_physical(mm, VirtAddr::new(string_start + argv0_bytes.len()), &[0])?;
    Ok(VirtAddr::new(stack_pointer))
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

        // SAFETY: `populate_page` returned the RAM address for this user VA and
        // this iteration is bounded to the containing page.
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
