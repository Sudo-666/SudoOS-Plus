//! ELF 加载器（Linux 风格）。
//!
//! 参照 Linux `fs/binfmt_elf.c`。
//!
//! 提供：
//! - ELF64 头解析
//! - PT_LOAD 段映射（参照 `elf_map`）
//! - 用户栈布局（argc / argv / envp / auxv，参照 `create_elf_tables`）
//! - `load_elf()`：完整加载流程
//!
//! 仅支持静态链接的 ELF64 可执行文件（ET_EXEC）。
//! 不支持共享库（ET_DYN）、解释器（PT_INTERP）。

use myos_mm::{
    PAGE_SIZE, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind,
};

use crate::user_mm::UserMm;

// ---------------------------------------------------------------------------
// ELF64 常量
// ---------------------------------------------------------------------------

/// ELF 魔数：`\x7fELF`
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];

/// 64-bit ELF。
const ELFCLASS64: u8 = 2;

/// 小端字节序。
const ELFDATA2LSB: u8 = 1;

/// 可执行文件类型。
const ET_EXEC: u16 = 2;

/// RISC-V 架构。
const EM_RISCV: u16 = 243;
/// LoongArch 架构。
const EM_LOONGARCH: u16 = 258;

/// PT_LOAD 段类型。
const PT_LOAD: u32 = 1;

/// PF_X (可执行)。
const PF_X: u32 = 1;
/// PF_W (可写)。
const PF_W: u32 = 2;
/// PF_R (可读)。
const PF_R: u32 = 4;

/// 辅助向量条目。
const AT_NULL: usize = 0;
const AT_PAGESZ: usize = 6;
const AT_PHDR: usize = 3;
const AT_PHNUM: usize = 4;
const AT_ENTRY: usize = 9;
const AT_RANDOM: usize = 25;

// ---------------------------------------------------------------------------
// ELF64 头结构
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Elf64ProgramHeader {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

/// ELF 加载结果。
pub struct ElfLoadInfo {
    /// 程序入口地址。
    pub entry: VirtAddr,
    /// 程序头表地址（用于 auxv）。
    pub phdr: VirtAddr,
    /// 程序头数量。
    pub phnum: u16,
    /// 程序 break 结束地址（BSS 之后）。
    pub brk_end: VirtAddr,
}

// ---------------------------------------------------------------------------
// ELF 加载（参照 Linux `load_elf_binary`）
// ---------------------------------------------------------------------------

/// 加载 ELF 可执行文件（参照 Linux `load_elf_binary`）。
///
/// # 参数
/// - `data`：ELF 文件内容的字节切片。
/// - `user_mm`：待填充的用户地址空间。
///
/// # 返回
/// 成功时返回 [`ElfLoadInfo`]，包含入口地址和 auxv 所需信息。
pub fn load_elf(
    data: &[u8],
    user_mm: &UserMm,
) -> Result<ElfLoadInfo, ElfError> {
    let header = parse_header(data)?;
    let phdrs = parse_program_headers(data, &header)?;

    // 映射 PT_LOAD 段
    let mut brk_end = VirtAddr::new(0);
    let mut phdr_addr = VirtAddr::new(0);

    for phdr in &phdrs {
        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = VirtAddr::new(phdr.p_vaddr as usize);
        let _filesz = phdr.p_filesz as usize;
        let memsz = phdr.p_memsz as usize;
        let flags = phdr.p_flags;

        // 计算页对齐的映射范围
        let segment_start = vaddr.align_down(PAGE_SIZE).ok_or(ElfError::InvalidAddress)?;
        let segment_end = vaddr
            .checked_add(memsz)
            .ok_or(ElfError::InvalidAddress)?;
        let segment_end_aligned = segment_end.align_up(PAGE_SIZE).ok_or(ElfError::InvalidAddress)?;
        let segment_range = VirtRange::new(segment_start, segment_end_aligned)
            .ok_or(ElfError::InvalidAddress)?;

        // 确定 VMA 标志
        let mut vma_flags = VmAreaFlags::empty();
        if flags & PF_R != 0 {
            vma_flags = vma_flags.union(VmAreaFlags::READ);
        }
        if flags & PF_W != 0 {
            vma_flags = vma_flags.union(VmAreaFlags::WRITE);
        }
        if flags & PF_X != 0 {
            vma_flags = vma_flags.union(VmAreaFlags::EXECUTE);
        }
        vma_flags = vma_flags.union(VmAreaFlags::USER);
        vma_flags = vma_flags.union(VmAreaFlags::PRIVATE);

        let area = VmArea::new(segment_range, vma_flags, VmAreaKind::Anonymous);
        user_mm
            .map_fixed_area(area)
            .map_err(|_| ElfError::SegmentMapFailed)?;

        // 追踪 brk 结束位置
        if segment_end_aligned.get() > brk_end.get() {
            brk_end = segment_end_aligned;
        }

        // 追踪 PHDR 地址（第一个 PT_LOAD 段的 p_vaddr + 偏移 PHDR）
        if phdr_addr.get() == 0 && phdr.p_offset <= header.e_phoff {
            phdr_addr = VirtAddr::new(
                vaddr.get() + (header.e_phoff as usize - phdr.p_offset as usize),
            );
        }
    }

    let entry = VirtAddr::new(header.e_entry as usize);

    Ok(ElfLoadInfo {
        entry,
        phdr: phdr_addr,
        phnum: header.e_phnum,
        brk_end,
    })
}

// ---------------------------------------------------------------------------
// ELF 解析辅助
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ElfError {
    InvalidMagic,
    Not64Bit,
    NotLittleEndian,
    NotExecutable,
    WrongArchitecture,
    NoProgramHeaders,
    InvalidAddress,
    SegmentMapFailed,
    ReadError,
}

fn parse_header(data: &[u8]) -> Result<Elf64Header, ElfError> {
    if data.len() < core::mem::size_of::<Elf64Header>() {
        return Err(ElfError::ReadError);
    }

    // SAFETY: ELF header 是 POD 结构，从已验证长度的字节切片转换是安全的。
    let header: &Elf64Header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    // 验证魔数
    if header.e_ident[0..4] != ELF_MAGIC {
        return Err(ElfError::InvalidMagic);
    }

    // 验证 64-bit
    if header.e_ident[4] != ELFCLASS64 {
        return Err(ElfError::Not64Bit);
    }

    // 验证小端
    if header.e_ident[5] != ELFDATA2LSB {
        return Err(ElfError::NotLittleEndian);
    }

    // 验证可执行文件
    if header.e_type != ET_EXEC {
        return Err(ElfError::NotExecutable);
    }

    // 验证架构
    match header.e_machine {
        #[cfg(target_arch = "riscv64")]
        EM_RISCV => {}
        #[cfg(target_arch = "loongarch64")]
        EM_LOONGARCH => {}
        _ => return Err(ElfError::WrongArchitecture),
    }

    // 验证有程序头
    if header.e_phnum == 0 {
        return Err(ElfError::NoProgramHeaders);
    }

    Ok(*header)
}

fn parse_program_headers(
    data: &[u8],
    header: &Elf64Header,
) -> Result<alloc::vec::Vec<Elf64ProgramHeader>, ElfError> {
    use alloc::vec::Vec;

    let phoff = header.e_phoff as usize;
    let phentsize = header.e_phentsize as usize;
    let phnum = header.e_phnum as usize;

    if phentsize != core::mem::size_of::<Elf64ProgramHeader>() {
        return Err(ElfError::ReadError);
    }

    let end = phoff
        .checked_add(phnum.checked_mul(phentsize).ok_or(ElfError::ReadError)?)
        .ok_or(ElfError::ReadError)?;

    if end > data.len() {
        return Err(ElfError::ReadError);
    }

    let mut phdrs = Vec::with_capacity(phnum);

    for i in 0..phnum {
        let offset = phoff + i * phentsize;
        // SAFETY: 已验证范围在 data 内。
        let phdr: &Elf64ProgramHeader =
            unsafe { &*(data.as_ptr().add(offset) as *const Elf64ProgramHeader) };
        phdrs.push(*phdr);
    }

    Ok(phdrs)
}

// ---------------------------------------------------------------------------
// 用户栈设置（参照 Linux `create_elf_tables`）
// ---------------------------------------------------------------------------

/// 在用户栈上布局 argc、argv、envp、auxv（参照 Linux `create_elf_tables`）。
///
/// 栈布局（从高地址到低地址）：
///
/// ```text
/// 高地址 (栈顶)
/// ┌──────────────────────┐
/// │  [字符串区域]         │ ← argv 和 envp 字符串
/// │  [AT_RANDOM 16 bytes]│
/// │  [AT_PLATFORM]        │
/// ├──────────────────────┤
/// │  [AT_NULL (0, 0)]    │ ← auxv 终止符
/// │  [auxv[N-1]]          │
/// │  ...                  │
/// │  [auxv[0]]            │
/// ├──────────────────────┤
/// │  [NULL]               │ ← envp 终止符
/// │  [envp[M-1]]          │
/// │  ...                  │
/// │  [envp[0]]            │
/// ├──────────────────────┤
/// │  [NULL]               │ ← argv 终止符
/// │  [argv[N-1]]          │
/// │  ...                  │
/// │  [argv[0]]            │
/// ├──────────────────────┤
/// │  [argc]               │ ← 新 sp（16 字节对齐）
/// └──────────────────────┘ 低地址
/// ```
///
/// 返回新的用户栈指针（指向 argc）。
pub fn setup_user_stack(
    sp: VirtAddr,
    entry: VirtAddr,
    phdr: VirtAddr,
    phnum: u16,
    argv: &[alloc::vec::Vec<u8>],
    envp: &[alloc::vec::Vec<u8>],
) -> VirtAddr {
    let ptr_size = core::mem::size_of::<usize>();

    // AT_RANDOM：16 字节伪随机数
    let random_bytes: [u8; 16] = [
        0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe, 0xba, 0xbe,
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    ];
    let platform = if cfg!(target_arch = "riscv64") {
        b"riscv64\0".as_slice()
    } else {
        b"loongarch64\0".as_slice()
    };

    // 压入字符串数据（从底到顶）
    let mut data: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    let mut arg_addrs: alloc::vec::Vec<usize> = alloc::vec::Vec::new();
    let mut env_addrs: alloc::vec::Vec<usize> = alloc::vec::Vec::new();

    // envp 字符串（先压入，更靠近栈顶）
    for env in envp.iter().rev() {
        env_addrs.push(data.len());
        data.extend_from_slice(env);
        data.push(0);
    }

    // argv 字符串
    for arg in argv.iter().rev() {
        arg_addrs.push(data.len());
        data.extend_from_slice(arg);
        data.push(0);
    }

    // AT_RANDOM
    let random_off = data.len();
    data.extend_from_slice(&random_bytes);

    // AT_PLATFORM
    let plat_off = data.len();
    data.extend_from_slice(platform);

    // 对齐
    let string_total = ((data.len() + 15) & !15) + ptr_size;

    // 指针表
    let argc = argv.len();
    let envc = envp.len();
    let auxv_entries = 7; // AT_PAGESZ, AT_PHDR, AT_PHNUM, AT_ENTRY, AT_RANDOM, AT_PLATFORM, AT_NULL
    let ptr_count = 1 + argc + 1 + envc + 1 + auxv_entries * 2; // argc + argv + NULL + envp + NULL + auxv
    let table_size = ptr_count * ptr_size;
    let total = string_total + table_size;

    let sp_base = sp.get().checked_sub(total).expect("user stack overflow");
    let sp_aligned = (sp_base + 15) & !15;

    let stack = unsafe {
        core::slice::from_raw_parts_mut(sp_aligned as *mut u8, total)
    };

    // 字符串数据
    let data_base = sp_aligned;
    stack[..data.len()].copy_from_slice(&data);

    // 指针表
    let table_base = data_base + ((data.len() + 15) & !15);
    let mut pos = table_base + table_size;

    // 从右向左写指针
    let mut push = |val: usize| {
        pos -= ptr_size;
        stack[pos - sp_aligned..pos + ptr_size - sp_aligned]
            .copy_from_slice(&val.to_ne_bytes());
    };

    // AT_NULL
    push(AT_NULL);
    push(0);

    // AT_PLATFORM
    push(data_base + plat_off);
    push(15); // AT_PLATFORM is 15 (not standard, but works for now)

    // AT_RANDOM
    push(data_base + random_off);
    push(AT_RANDOM);

    // AT_ENTRY
    push(entry.get());
    push(AT_ENTRY);

    // AT_PHNUM
    push(phnum as usize);
    push(AT_PHNUM);

    // AT_PHDR
    push(phdr.get());
    push(AT_PHDR);

    // AT_PAGESZ
    push(PAGE_SIZE);
    push(AT_PAGESZ);

    // envp NULL terminator
    push(0);

    // envp pointers
    for &off in env_addrs.iter() {
        push(data_base + off);
    }

    // argv NULL terminator
    push(0);

    // argv pointers (reverse of arg_addrs to get correct order)
    for &off in arg_addrs.iter() {
        push(data_base + off);
    }

    // argc
    push(argc);

    VirtAddr::new(pos)
}
