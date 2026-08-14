// SUDOOS_NEWTEST_P0_ABI_HOTFIX_V2: richer auxv for libc startup probes.
// SUDOOS_M16A_ELF_AUXV_PATCH_V1
// SUDOOS_M16B_DYNAMIC_ELF: PT_INTERP interpreter loading and dynamic-linker handoff.
use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use myos_mm::{PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind};

use crate::process::{Process, Thread};
use crate::user_mm::{UserMm, UserMmRuntimeError};
use crate::{
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

const EXEC_IMAGE_CACHE_LIMIT: usize = 1024 * 1024 * 1024;
static EXEC_IMAGE_CACHE: IrqSpinLock<BTreeMap<ExecCacheKey, Arc<Vec<u8>>>> =
    IrqSpinLock::new_with_class(
        BTreeMap::new(),
        LockClass::new("exec_image_cache", LockRank::Vfs, 1),
    );

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExecCacheKey {
    dev: u64,
    ino: u64,
    size: i64,
    mtime_sec: i64,
    mtime_nsec: i64,
}

impl ExecCacheKey {
    pub const fn from_stat(stat: myos_vfs::Stat) -> Self {
        Self {
            dev: stat.dev,
            ino: stat.ino,
            size: stat.size,
            mtime_sec: stat.mtime_sec,
            mtime_nsec: stat.mtime_nsec,
        }
    }
}

pub fn cached_exec_image(key: ExecCacheKey) -> Option<Arc<Vec<u8>>> {
    EXEC_IMAGE_CACHE.lock().get(&key).cloned()
}

pub fn retain_exec_image(key: ExecCacheKey, image: Vec<u8>) -> Arc<Vec<u8>> {
    let mut cache = EXEC_IMAGE_CACHE.lock();
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    let retained = Arc::new(image);
    let used = cache.values().fold(0_usize, |sum, entry| sum.saturating_add(entry.len()));
    if used.saturating_add(retained.len()) <= EXEC_IMAGE_CACHE_LIMIT {
        cache.insert(key, Arc::clone(&retained));
    }
    retained
}

#[derive(Clone)]
pub struct ExecFileBacking {
    pub file: myos_vfs::ArcFile,
    pub device: u64,
    pub inode: u64,
    pub content_generation: u64,
    pub shared_cache: bool,
}

pub const fn stat_content_generation(stat: myos_vfs::Stat) -> u64 {
    (stat.mtime_sec as u64) ^ (stat.mtime_nsec as u64).rotate_left(32)
}

// Keep the signal trampoline immediately below the production TLS/stack
// reservation. Large fixed-address toolchain executables legitimately cover
// the old low 0x510000 page.
pub const USER_SIGNAL_TRAMPOLINE: usize = 0x0000_003e_ff7f_e000;

#[cfg(target_arch = "riscv64")]
const SIGNAL_TRAMPOLINE_BYTES: &[u8] = &[
    0x93, 0x08, 0xb0, 0x08, // addi a7, zero, 139 (rt_sigreturn)
    0x73, 0x00, 0x00, 0x00, // ecall
];

#[cfg(target_arch = "loongarch64")]
const SIGNAL_TRAMPOLINE_BYTES: &[u8] = &[
    0x0b, 0x2c, 0xc2, 0x02, // addi.d r11, r0, 139 (rt_sigreturn)
    0x00, 0x00, 0x2b, 0x00, // syscall 0
];

const AT_NULL: usize = 0;
const AT_PHDR: usize = 3;
const AT_PHENT: usize = 4;
const AT_PHNUM: usize = 5;
const AT_PAGESZ: usize = 6;
const AT_BASE: usize = 7;
const AT_FLAGS: usize = 8;
const AT_ENTRY: usize = 9;
const AT_UID: usize = 11;
const AT_EUID: usize = 12;
const AT_GID: usize = 13;
const AT_EGID: usize = 14;
const AT_CLKTCK: usize = 17;
const AT_PLATFORM: usize = 15;
const AT_HWCAP: usize = 16;
const AT_HWCAP2: usize = 26;
const AT_SECURE: usize = 23;
const AT_RANDOM: usize = 25;
const AT_EXECFN: usize = 31;
const DT_NULL: u64 = 0;
const DT_PLTRELSZ: u64 = 2;
#[cfg(target_arch = "loongarch64")]
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
#[cfg(target_arch = "loongarch64")]
const DT_SYMENT: u64 = 11;
const DT_REL: u64 = 17;
const DT_RELSZ: u64 = 18;
const DT_JMPREL: u64 = 23;
#[cfg(target_arch = "loongarch64")]
const DT_RELRSZ: u64 = 35;
#[cfg(target_arch = "loongarch64")]
const DT_RELR: u64 = 36;
#[cfg(target_arch = "loongarch64")]
const DT_RELRENT: u64 = 37;

#[cfg(any(target_arch = "riscv64", target_arch = "loongarch64"))]
const R_RELATIVE: u32 = 3;
/// R_LARCH_64 (2): absolute 64-bit relocation.  For symbol=0 this is
/// equivalent to R_LARCH_RELATIVE and writes the addend as the value.
#[cfg(target_arch = "loongarch64")]
const R_ABS64: u32 = 2;

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

impl ExecError {
    /// Human-readable diagnostic tag for rate-limited execve tracing.
    /// Returns a short static string suitable for contest logs.
    pub fn reason(&self) -> &'static str {
        match self {
            ExecError::AddressOverflow => "address-overflow",
            ExecError::DynamicInterpreterUnsupported => "dynamic-interpreter-unsupported",
            ExecError::Elf(e) => e.reason(),
            ExecError::Initramfs(_) => "initramfs-error",
            ExecError::InvalidStack => "invalid-stack",
            ExecError::MetadataOutOfMemory => "metadata-out-of-memory",
            ExecError::Process(_) => "process-error",
            ExecError::UserMm(_) => "user-mm-error",
            ExecError::Vfs(_) => "vfs-error",
        }
    }
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
    /// User-space entry point: for static ELF this is the main binary entry;
    /// for dynamic ELF this is the interpreter (ld-linux) entry.
    pub entry: VirtAddr,
    pub stack: VirtRange,
    pub stack_pointer: VirtAddr,
    /// For dynamically-linked executables: the base address where the
    /// interpreter was loaded. Used for AT_BASE in auxv.
    pub interp_base: Option<VirtAddr>,
    /// For dynamically-linked executables: the main program's entry point.
    /// Used for AT_ENTRY in auxv so ld-linux can transfer control.
    pub main_entry: Option<VirtAddr>,
    /// For dynamically-linked executables: the main program's PHDR info.
    /// Used so ld-linux can find the main binary's program headers via auxv.
    pub main_phdr: Option<(VirtAddr, usize, usize)>,
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
    // Set initial TLS pointer for the main thread.
    // ld-linux dereferences tp/r2 immediately for GOT/TLS access;
    // a NULL tp causes 0x0/0x8/0x18 faults.  Point tp to a safe
    // zero-filled page so ld-linux can bootstrap its own TLS.
    // The exact value is overwritten by TLS_INIT_TP in ld-linux.
    thread
        .prepare_stack_pointer(prepared.stack_pointer)
        .expect("exec built an invalid initial user stack pointer");
    crate::process::assert_initial_pair(&process, &thread);
    Ok(ExecImage { process, thread })
}

/// Fixed load bias for the ELF interpreter (ld-linux). Must not overlap
/// the main-PIE bias (0x4000_0000), the stack, or the user page at 0x400000.
const INTERP_LOAD_BIAS: usize = 0x2000_0000;
const MAX_EXEC_IMAGE: usize = 128 * 1024 * 1024;

pub fn prepare_elf(image: &[u8], config: ExecConfig<'_>) -> Result<PreparedExec, ExecError> {
    prepare_elf_backed(image, None, config)
}

pub fn prepare_elf_backed(
    image: &[u8],
    main_backing: Option<&ExecFileBacking>,
    config: ExecConfig<'_>,
) -> Result<PreparedExec, ExecError> {
    let elf = crate::elf::parse(image)?;

    // Validate ELF kind and load_bias invariants.
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

    // Load the ELF interpreter if this binary has PT_INTERP.
    let interp_image: Option<Arc<Vec<u8>>>;
    let interp_backing: Option<ExecFileBacking>;
    let interp_elf: Option<crate::elf::ElfImage>;
    let interp_entry: Option<VirtAddr>;
    let main_entry: Option<VirtAddr>;
    let interp_base: Option<VirtAddr>;
    let main_phdr: Option<(VirtAddr, usize, usize)>;

    if let Some(interpreter_path) = elf.interpreter.as_ref() {
        #[cfg(target_arch = "riscv64")]
        const SYSTEM_INTERPRETER: &str = "/mnt/sdcard/system-glibc/ld-linux-riscv64-lp64d.so.1";
        #[cfg(target_arch = "loongarch64")]
        const SYSTEM_INTERPRETER: &str = "/mnt/sdcard/system-glibc/ld-linux-loongarch-lp64d.so.1";
        let interpreter_load_path = if crate::fs::stat(SYSTEM_INTERPRETER).is_ok() {
            SYSTEM_INTERPRETER
        } else {
            interpreter_path.as_str()
        };
        // Read the interpreter binary from the VFS.
        let (interp_bytes, backing) = match load_exec_image_from_vfs(interpreter_load_path) {
            Ok(source) => source,
            Err(e) => {
                crate::println!(
                    "exec: interp={} open-failed reason={}",
                    interpreter_load_path,
                    e.reason(),
                );
                return Err(e);
            }
        };
        // Parse interpreter ELF with its own load bias.
        let mut parsed = crate::elf::parse_with_bias(&interp_bytes, INTERP_LOAD_BIAS)?;
        // Validate: interpreter must be ET_DYN (PIE) or ET_EXEC.
        match parsed.kind {
            crate::elf::ElfKind::Executable | crate::elf::ElfKind::PositionIndependent => {}
        }
        if crate::user::oscomp_verbose_user_trace_active() {
            crate::println!(
                "exec: interp={} kind={:?} entry={:#x} bias={:#x}",
                interpreter_load_path,
                parsed.kind,
                parsed.entry.get(),
                INTERP_LOAD_BIAS,
            );
        }
        interp_entry = Some(parsed.entry);
        main_entry = Some(elf.entry);
        interp_base = Some(VirtAddr::new(INTERP_LOAD_BIAS));
        main_phdr = elf
            .program_headers
            .map(|info| (info.virtual_address, info.entry_size, info.count));
        interp_image = Some(interp_bytes);
        interp_backing = Some(backing);
        interp_elf = Some(parsed);
    } else {
        interp_entry = None;
        main_entry = None;
        interp_base = None;
        main_phdr = None;
        interp_image = None;
        interp_backing = None;
        interp_elf = None;
    }

    // Combine areas: main ELF + interpreter ELF (if any) + extra + stack.
    let mut areas = Vec::new();
    let area_count = elf
        .areas
        .len()
        .checked_add(if interp_elf.is_some() {
            interp_elf.as_ref().unwrap().areas.len()
        } else {
            0
        })
        .and_then(|count| count.checked_add(config.extra_areas.len()))
        .and_then(|count| count.checked_add(2)) // signal trampoline + stack
        .ok_or(ExecError::AddressOverflow)?;
    areas
        .try_reserve(area_count)
        .map_err(|_| ExecError::MetadataOutOfMemory)?;
    areas.extend_from_slice(&elf.areas);
    if let Some(interp) = interp_elf.as_ref() {
        areas.extend_from_slice(&interp.areas);
    }
    areas.extend_from_slice(config.extra_areas);
    areas.push(VmArea::new(
        VirtRange::from_bounds(USER_SIGNAL_TRAMPOLINE, USER_SIGNAL_TRAMPOLINE + PAGE_SIZE),
        VmAreaFlags::user_rw().union(VmAreaFlags::EXECUTE),
        VmAreaKind::Anonymous,
    ));
    areas.push(VmArea::new(
        config.stack,
        VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
        VmAreaKind::Stack,
    ));

    let mm = build_mm(&areas, config.heap_start, config.heap_limit)?;

    // Load segments and build stack.
    let stack_pointer = match (|| {
        // Load main ELF segments.
        for segment in &elf.segments {
            load_segment_backed(&mm, image, *segment, main_backing)?;
        }
        // Apply static PIE relocations on the MAIN binary ONLY if it has
        // no PT_INTERP (true static PIE).  Dynamically-linked binaries
        // get their GOT/PLT fixed up by ld-linux at runtime; the kernel
        // must not touch them here.
        let has_interp = elf.interpreter.is_some();
        if !has_interp {
            apply_static_pie_relocations(&mm, image, &elf)?;
        } else if crate::user::oscomp_verbose_user_trace_active() {
            crate::println!(
                "exec-reloc: skip main (has PT_INTERP) rela={}",
                elf.dynamic.map_or(0, |d| d.memory_size),
            );
        }
        // Load interpreter segments if present.
        if let (Some(interp_data), Some(interp)) = (interp_image.as_ref(), interp_elf.as_ref()) {
            for segment in &interp.segments {
                load_segment_backed(&mm, interp_data, *segment, interp_backing.as_ref())?;
            }
            // A PT_INTERP loader is itself a self-relocating static PIE.  The
            // kernel only maps it and supplies AT_BASE; rtld must perform its
            // bootstrap relocation exactly once. Applying the same relative
            // relocation in both the kernel and rtld would add the load bias
            // twice and is not part of the Linux exec contract.
            if crate::user::oscomp_verbose_user_trace_active() {
                crate::println!("exec-reloc: defer interpreter self-relocation");
            }
        }
        mm.copy_to_user(USER_SIGNAL_TRAMPOLINE, SIGNAL_TRAMPOLINE_BYTES)?;
        build_initial_stack(
            &mm,
            config.stack,
            config.argv,
            config.envp,
            &elf,
            interp_base,
            main_entry,
            main_phdr,
        )
    })() {
        Ok(stack_pointer) => stack_pointer,
        Err(error) => {
            destroy_mm(mm)?;
            return Err(error);
        }
    };

    // Entry: interpreter entry if dynamic, else main ELF entry.
    let final_entry = interp_entry.unwrap_or(elf.entry);

    Ok(PreparedExec {
        mm,
        entry: final_entry,
        stack: config.stack,
        stack_pointer,
        interp_base,
        main_entry,
        main_phdr,
    })
}

/// Read an ELF image from the VFS by path. Used for loading the dynamic linker.
fn load_exec_image_from_vfs(path: &str) -> Result<(Arc<Vec<u8>>, ExecFileBacking), ExecError> {
    match crate::fs::open(path, myos_vfs::OpenFlags::O_RDONLY) {
        Ok(file) => {
            let stat = file.fstat().map_err(ExecError::from)?;
            let size = usize::try_from(stat.size).map_err(|_| ExecError::AddressOverflow)?;
            if size > MAX_EXEC_IMAGE {
                return Err(ExecError::AddressOverflow);
            }
            let key = ExecCacheKey::from_stat(stat);
            let cacheable = exec_path_is_immutable(path);
            let backing = ExecFileBacking {
                file: Arc::clone(&file),
                device: stat.dev,
                inode: stat.ino,
                content_generation: stat_content_generation(stat),
                shared_cache: true,
            };
            if cacheable && let Some(image) = cached_exec_image(key) {
                return Ok((image, backing));
            }
            let mut image = Vec::new();
            image
                .try_reserve(size)
                .map_err(|_| ExecError::MetadataOutOfMemory)?;
            image.resize(size, 0);
            let mut output = myos_vfs::MutableIoBuffer::new(&mut image);
            let read = file.read(&mut output).map_err(ExecError::from)?;
            image.truncate(read);
            let image = if cacheable {
                retain_exec_image(key, image)
            } else {
                Arc::new(image)
            };
            Ok((image, backing))
        }
        Err(errno) => Err(ExecError::Vfs(errno)),
    }
}

pub fn exec_path_is_immutable(path: &str) -> bool {
    path.starts_with("/mnt/sdcard/")
        || path.contains("/.rustup/")
        || path.starts_with("/work/tgoskits/")
        || path.starts_with("/tmp/sudoos-buildstorm-bin/")
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
    let source_end = segment
        .file_offset
        .checked_add(segment.file_size)
        .ok_or(ExecError::AddressOverflow)?;
    let source = image
        .get(segment.file_offset..source_end)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidSegment))?;
    mm.load_bytes(segment.virtual_address, source)?;

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

fn load_segment_backed(
    mm: &UserMm,
    image: &[u8],
    segment: crate::elf::LoadSegment,
    backing: Option<&ExecFileBacking>,
) -> Result<(), ExecError> {
    let Some(backing) = backing else {
        return load_segment(mm, image, segment);
    };

    // Writable LOAD segments contain process-private GOT/data and must be
    // materialized before relocation. Immutable text/rodata instead follows
    // the Linux filemap model: register the aligned file interval and fault
    // physical pages into every process from the global page cache.
    if segment.flags.is_writable() {
        return load_segment(mm, image, segment);
    }

    let in_page = segment.virtual_address.get() & (PAGE_SIZE - 1);
    let file_offset = segment
        .file_offset
        .checked_sub(in_page)
        .ok_or(ExecError::AddressOverflow)?;
    let file_length = in_page
        .checked_add(segment.file_size)
        .ok_or(ExecError::AddressOverflow)?
        .min(segment.range.size());
    mm.register_file_mapping(
        segment.range,
        file_offset as u64,
        file_length,
        backing.device,
        backing.inode,
        backing.content_generation,
        backing.shared_cache,
        Arc::clone(&backing.file),
    )?;
    Ok(())
}

#[cfg(target_arch = "riscv64")]
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

    // DT_JMPREL/DT_PLTRELSZ/DT_REL must NOT prevent processing of
    // DT_RELA R_RELATIVE entries.  They indicate the presence of PLT
    // or REL-type relocations which the kernel does not handle; the
    // dynamic linker (ld-linux) resolves those at runtime.
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

    let mut applied: usize = 0;
    let mut skipped: usize = 0;
    for entry in rela_bytes.chunks_exact(rela_ent) {
        let raw_offset = read_u64(entry, 0)?;
        let info = read_u64(entry, 8)?;
        let addend = read_i64(entry, 16)?;
        let relocation_type = (info & 0xffff_ffff) as u32;
        let symbol = info >> 32;
        // R_LARCH_64 (type 2, symbol=0): absolute 64-bit relocation.
        // Writes the addend as an absolute address.  Used by static
        // PIE binaries for GOT entries pointing to global data.
        #[cfg(target_arch = "loongarch64")]
        let is_abs64 = relocation_type == R_ABS64 && symbol == 0;
        #[cfg(not(target_arch = "loongarch64"))]
        let is_abs64 = false;

        if (relocation_type == R_RELATIVE || is_abs64) && symbol == 0 {
            let destination = usize::try_from(raw_offset)
                .map_err(|_| ExecError::AddressOverflow)?
                .checked_add(elf.load_bias)
                .ok_or(ExecError::AddressOverflow)?;
            let value = if is_abs64 {
                // R_LARCH_64 with symbol=0: value = addend (absolute address).
                addend as u64
            } else {
                // R_RELATIVE: value = load_bias + addend.
                u64::try_from(
                    (elf.load_bias as i128)
                        .checked_add(addend as i128)
                        .ok_or(ExecError::AddressOverflow)?,
                )
                .map_err(|_| ExecError::AddressOverflow)?
            };
            loader_copy_to_user_physical(mm, VirtAddr::new(destination), &value.to_le_bytes())?;
            applied += 1;
        } else {
            skipped += 1;
        }
    }
    if (applied > 0 || skipped > 0) && crate::user::oscomp_verbose_user_trace_active() {
        crate::println!(
            "exec-reloc: applied={} skipped={} jmprel={} pltrel={}",
            applied,
            skipped,
            jmprel,
            pltrel_size,
        );
    }

    Ok(())
}


#[cfg(target_arch = "loongarch64")]
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

    let mut symtab_vaddr = 0_usize;
    let mut sym_ent = 24_usize;
    let mut rela_vaddr = 0_usize;
    let mut rela_size = 0_usize;
    let mut rela_ent = 24_usize;
    let mut rel_size = 0_usize;
    let mut relr_vaddr = 0_usize;
    let mut relr_size = 0_usize;
    let mut relr_ent = 8_usize;
    let mut jmprel = 0_usize;
    let mut pltrel_size = 0_usize;

    for entry in entries.chunks_exact(16) {
        let tag = read_u64(entry, 0)?;
        let value = usize::try_from(read_u64(entry, 8)?)
            .map_err(|_| ExecError::AddressOverflow)?;
        match tag {
            DT_NULL => break,
            DT_SYMTAB => symtab_vaddr = value,
            DT_SYMENT => sym_ent = value,
            DT_RELA => rela_vaddr = value,
            DT_RELASZ => rela_size = value,
            DT_RELAENT => rela_ent = value,
            DT_REL => {}
            DT_RELSZ => rel_size = value,
            DT_RELR => relr_vaddr = value,
            DT_RELRSZ => relr_size = value,
            DT_RELRENT => relr_ent = value,
            DT_JMPREL => jmprel = value,
            DT_PLTRELSZ => pltrel_size = value,
            _ => {}
        }
    }

    if rel_size != 0 {
        // ELF64 LoongArch uses RELA/RELR.  Do not consume REL entries with
        // RELA semantics because their addends live at the destination.
        return Err(ExecError::Elf(crate::elf::ElfError::Unsupported));
    }
    if symtab_vaddr != 0 && sym_ent != 24 {
        return Err(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader));
    }

    let relr_applied = apply_loongarch_relr(
        mm,
        image,
        elf,
        relr_vaddr,
        relr_size,
        relr_ent,
    )?;

    let mut rela_applied = 0_usize;
    let mut rela_skipped = 0_usize;
    if rela_size != 0 {
        if rela_vaddr == 0 || rela_ent != 24 || !rela_size.is_multiple_of(rela_ent) {
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
            let destination = usize::try_from(raw_offset)
                .map_err(|_| ExecError::AddressOverflow)?
                .checked_add(elf.load_bias)
                .ok_or(ExecError::AddressOverflow)?;

            let value = if relocation_type == R_RELATIVE && symbol == 0 {
                Some(add_signed_u64(elf.load_bias as u64, addend)?)
            } else if relocation_type == R_ABS64 {
                let symbol_value = if symbol == 0 {
                    Some(0_u64)
                } else {
                    read_loongarch_dynamic_symbol(
                        image,
                        elf,
                        symtab_vaddr,
                        sym_ent,
                        symbol,
                    )?
                };
                match symbol_value {
                    Some(symbol_value) => Some(add_signed_u64(symbol_value, addend)?),
                    None => None,
                }
            } else {
                None
            };

            if let Some(value) = value {
                loader_copy_to_user_physical(
                    mm,
                    VirtAddr::new(destination),
                    &value.to_le_bytes(),
                )?;
                rela_applied += 1;
            } else {
                // Undefined dynamic symbols and PLT relocations are resolved
                // by ld-linux after its self-relocation has completed.
                rela_skipped += 1;
            }
        }
    }

    if (relr_applied != 0 || rela_applied != 0 || rela_skipped != 0)
        && crate::user::oscomp_verbose_user_trace_active()
    {
        crate::println!(
            "exec-reloc-la: rela-applied={} relr-applied={} skipped={} jmprel={} pltrel={}",
            rela_applied,
            relr_applied,
            rela_skipped,
            jmprel,
            pltrel_size,
        );
    }
    Ok(())
}

#[cfg(target_arch = "loongarch64")]
fn apply_loongarch_relr(
    mm: &UserMm,
    image: &[u8],
    elf: &crate::elf::ElfImage,
    relr_vaddr: usize,
    relr_size: usize,
    relr_ent: usize,
) -> Result<usize, ExecError> {
    if relr_size == 0 {
        return Ok(0);
    }
    if relr_vaddr == 0 || relr_ent != 8 || !relr_size.is_multiple_of(8) {
        return Err(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader));
    }

    let table_address = relr_vaddr
        .checked_add(elf.load_bias)
        .ok_or(ExecError::AddressOverflow)?;
    let table_offset = virtual_to_file_offset(elf, VirtAddr::new(table_address), relr_size)?;
    let bytes = image
        .get(table_offset..table_offset + relr_size)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidSegment))?;

    let mut cursor = 0_usize;
    let mut have_cursor = false;
    let mut applied = 0_usize;
    for encoded in bytes.chunks_exact(8) {
        let entry = usize::try_from(read_u64(encoded, 0)?)
            .map_err(|_| ExecError::AddressOverflow)?;
        if entry & 1 == 0 {
            cursor = entry
                .checked_add(elf.load_bias)
                .ok_or(ExecError::AddressOverflow)?;
            apply_one_loongarch_relr(mm, VirtAddr::new(cursor), elf.load_bias)?;
            applied += 1;
            cursor = cursor.checked_add(8).ok_or(ExecError::AddressOverflow)?;
            have_cursor = true;
            continue;
        }
        if !have_cursor {
            return Err(ExecError::Elf(crate::elf::ElfError::InvalidProgramHeader));
        }

        // Bit zero marks a bitmap entry. Bits 1..63 select the next 63
        // machine words beginning at the current cursor.
        let bitmap = entry >> 1;
        for bit in 0..63_usize {
            if bitmap & (1_usize << bit) == 0 {
                continue;
            }
            let delta = bit.checked_mul(8).ok_or(ExecError::AddressOverflow)?;
            let address = cursor
                .checked_add(delta)
                .ok_or(ExecError::AddressOverflow)?;
            apply_one_loongarch_relr(mm, VirtAddr::new(address), elf.load_bias)?;
            applied += 1;
        }
        cursor = cursor
            .checked_add(63 * 8)
            .ok_or(ExecError::AddressOverflow)?;
    }
    Ok(applied)
}

#[cfg(target_arch = "loongarch64")]
fn apply_one_loongarch_relr(
    mm: &UserMm,
    address: VirtAddr,
    load_bias: usize,
) -> Result<(), ExecError> {
    let current = loader_read_u64_physical(mm, address)?;
    let relocated = current
        .checked_add(load_bias as u64)
        .ok_or(ExecError::AddressOverflow)?;
    loader_copy_to_user_physical(mm, address, &relocated.to_le_bytes())
}

#[cfg(target_arch = "loongarch64")]
fn read_loongarch_dynamic_symbol(
    image: &[u8],
    elf: &crate::elf::ElfImage,
    symtab_vaddr: usize,
    sym_ent: usize,
    symbol: u64,
) -> Result<Option<u64>, ExecError> {
    const SHN_UNDEF: u16 = 0;
    const SHN_ABS: u16 = 0xfff1;
    if symtab_vaddr == 0 || sym_ent != 24 {
        return Ok(None);
    }
    let symbol_index = usize::try_from(symbol).map_err(|_| ExecError::AddressOverflow)?;
    let symbol_delta = symbol_index
        .checked_mul(sym_ent)
        .ok_or(ExecError::AddressOverflow)?;
    let symbol_address = symtab_vaddr
        .checked_add(elf.load_bias)
        .and_then(|base| base.checked_add(symbol_delta))
        .ok_or(ExecError::AddressOverflow)?;
    let symbol_offset = virtual_to_file_offset(elf, VirtAddr::new(symbol_address), sym_ent)?;
    let entry = image
        .get(symbol_offset..symbol_offset + sym_ent)
        .ok_or(ExecError::Elf(crate::elf::ElfError::InvalidSegment))?;

    let section = u16::from_le_bytes([entry[6], entry[7]]);
    if section == SHN_UNDEF {
        return Ok(None);
    }
    let value = read_u64(entry, 8)?;
    if section == SHN_ABS {
        return Ok(Some(value));
    }
    value
        .checked_add(elf.load_bias as u64)
        .map(Some)
        .ok_or(ExecError::AddressOverflow)
}

#[cfg(target_arch = "loongarch64")]
fn add_signed_u64(base: u64, addend: i64) -> Result<u64, ExecError> {
    u64::try_from(
        (base as i128)
            .checked_add(addend as i128)
            .ok_or(ExecError::AddressOverflow)?,
    )
    .map_err(|_| ExecError::AddressOverflow)
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
    interp_base: Option<VirtAddr>,
    main_entry: Option<VirtAddr>,
    main_phdr: Option<(VirtAddr, usize, usize)>,
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
    let platform = if cfg!(target_arch = "riscv64") {
        "riscv64"
    } else if cfg!(target_arch = "loongarch64") {
        "loongarch64"
    } else {
        "unknown"
    };
    let platform_ptr = push_stack_string(mm, stack, &mut cursor, platform)?;

    cursor = align_down(cursor, 16);

    // AT_PHDR/AT_PHENT/AT_PHNUM: for dynamically-linked executables the auxv
    // must describe the *main program's* PHDRs, not the interpreter's.
    // ld-linux reads these to find the main binary's DYNAMIC segment.
    let (phdr, phent, phnum) = if let Some((addr, size, count)) = main_phdr {
        (addr.get(), size, count)
    } else {
        let info = elf.program_headers;
        (
            info.map(|i| i.virtual_address.get()).unwrap_or(0),
            info.map(|i| i.entry_size).unwrap_or(0),
            info.map(|i| i.count).unwrap_or(0),
        )
    };

    // AT_BASE: for dynamic ELF this is the interpreter's load base so ld-linux
    // can find its own ELF headers and apply relocations to itself.
    let at_base = interp_base.map(|b| b.get()).unwrap_or(0);

    // AT_ENTRY: for dynamic ELF this is the *main program's* entry point;
    // ld-linux reads this so it can transfer control after relocation.
    let at_entry = main_entry.map(|e| e.get()).unwrap_or(elf.entry.get());

    let auxv = [
        (AT_PHDR, phdr),
        (AT_PHENT, phent),
        (AT_PHNUM, phnum),
        (AT_BASE, at_base),
        (AT_FLAGS, 0),
        (AT_ENTRY, at_entry),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_CLKTCK, 100),
        (AT_PLATFORM, platform_ptr),
        (AT_HWCAP, 0),
        (AT_HWCAP2, 0),
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

#[cfg(target_arch = "loongarch64")]
fn loader_read_u64_physical(mm: &UserMm, address: VirtAddr) -> Result<u64, ExecError> {
    if address.get() & 7 != 0 {
        return Err(ExecError::Elf(crate::elf::ElfError::InvalidAlignment));
    }
    let physical = mm.populate_page(address)?;
    let source = crate::arch::memory::phys_access::ram_ptr::<u8>(physical)
        .map_err(|_| UserMmRuntimeError::NotMapped)?;
    let mut bytes = [0_u8; 8];
    // SAFETY: populate_page returned the RAM byte corresponding to this VA;
    // an aligned 8-byte relocation cannot cross the 4 KiB page boundary.
    unsafe {
        core::ptr::copy_nonoverlapping(source, bytes.as_mut_ptr(), bytes.len());
    }
    Ok(u64::from_le_bytes(bytes))
}

fn loader_copy_to_user_physical(
    mm: &UserMm,
    address: VirtAddr,
    bytes: &[u8],
) -> Result<(), ExecError> {
    mm.load_bytes(address, bytes).map_err(ExecError::from)
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
