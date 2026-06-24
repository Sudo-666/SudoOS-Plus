#![feature(let_chains)]
#![no_std]

extern crate alloc;

use alloc::{string::String, sync::Arc};
use core::array;

use myos_sync::SpinLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(isize)]
pub enum Errno {
    Eperm = 1,
    Enoent = 2,
    Esrch = 3,
    Eio = 5,
    Ebadf = 9,
    Echild = 10,
    Eagain = 11,
    Enomem = 12,
    Eacces = 13,
    Efault = 14,
    Eexist = 17,
    Enodev = 19,
    Ebusy = 16,
    Enotdir = 20,
    Eisdir = 21,
    Enoexec = 8,
    Einval = 22,
    Emfile = 24,
    Enotty = 25,
    Enospc = 28,
    Espipe = 29,
    Epipe = 32,
    Erofs = 30,
    Enosys = 38,
    Enotempty = 39,
    Eloop = 40,
    Enametoolong = 36,
    Eoverflow = 75,
    Eafnosupport = 97,
    Enotsock = 88,
}

impl Errno {
    pub const fn to_isize(self) -> isize {
        -(self as isize)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct OpenFlags(u32);

impl OpenFlags {
    pub const O_RDONLY: Self = Self(0o0);
    pub const O_WRONLY: Self = Self(0o1);
    pub const O_RDWR: Self = Self(0o2);
    pub const O_ACCMODE: Self = Self(0o3);
    pub const O_CREAT: Self = Self(0o100);
    pub const O_EXCL: Self = Self(0o200);
    pub const O_TRUNC: Self = Self(0o1000);
    pub const O_APPEND: Self = Self(0o2000);
    pub const O_NONBLOCK: Self = Self(0o4000);
    pub const O_DIRECTORY: Self = Self(0o200000);
    pub const O_NOFOLLOW: Self = Self(0o400000);
    pub const O_CLOEXEC: Self = Self(0o2000000);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn access_mode(self) -> AccessMode {
        match self.0 & Self::O_ACCMODE.0 {
            0 => AccessMode::ReadOnly,
            1 => AccessMode::WriteOnly,
            2 => AccessMode::ReadWrite,
            _ => AccessMode::Invalid,
        }
    }

    pub const fn is_cloexec(self) -> bool {
        self.contains(Self::O_CLOEXEC)
    }
}

impl core::ops::BitOr for OpenFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for OpenFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for OpenFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
    Invalid,
}

impl AccessMode {
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn is_writable(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FileMode(u32);

impl FileMode {
    pub const S_IFMT: u32 = 0o170000;
    pub const S_IFREG: u32 = 0o100000;
    pub const S_IFDIR: u32 = 0o040000;
    pub const S_IFCHR: u32 = 0o020000;
    pub const S_IFBLK: u32 = 0o060000;
    pub const S_IFLNK: u32 = 0o120000;

    pub const FILE_DEFAULT: Self = Self(Self::S_IFREG | 0o644);
    pub const DIR_DEFAULT: Self = Self(Self::S_IFDIR | 0o755);
    pub const CHAR_DEFAULT: Self = Self(Self::S_IFCHR | 0o666);
    pub const BLOCK_DEFAULT: Self = Self(Self::S_IFBLK | 0o660);
    pub const SYMLINK_DEFAULT: Self = Self(Self::S_IFLNK | 0o777);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn file_type(self) -> FileType {
        match self.0 & Self::S_IFMT {
            Self::S_IFREG => FileType::Regular,
            Self::S_IFDIR => FileType::Directory,
            Self::S_IFCHR => FileType::CharDevice,
            Self::S_IFBLK => FileType::BlockDevice,
            Self::S_IFLNK => FileType::Symlink,
            _ => FileType::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileType {
    Regular,
    Directory,
    CharDevice,
    BlockDevice,
    Symlink,
    Unknown,
}

impl FileType {
    pub const fn dirent_type(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Directory => 4,
            Self::CharDevice => 2,
            Self::BlockDevice => 6,
            Self::Regular => 8,
            Self::Symlink => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PollEvents(u16);

impl PollEvents {
    pub const IN: Self = Self(0x0001);
    pub const PRI: Self = Self(0x0002);
    pub const OUT: Self = Self(0x0004);
    pub const ERR: Self = Self(0x0008);
    pub const HUP: Self = Self(0x0010);
    pub const NVAL: Self = Self(0x0020);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains_any(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekWhence {
    Set,
    Current,
    End,
}

impl SeekWhence {
    pub const fn from_raw(raw: usize) -> Option<Self> {
        match raw {
            0 => Some(Self::Set),
            1 => Some(Self::Current),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub size: i64,
    pub blksize: i32,
    pub blocks: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
}

impl Stat {
    pub const fn zeroed() -> Self {
        Self {
            dev: 0,
            ino: 0,
            mode: 0,
            nlink: 0,
            uid: 0,
            gid: 0,
            rdev: 0,
            size: 0,
            blksize: 4096,
            blocks: 0,
            atime_sec: 0,
            atime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            ctime_sec: 0,
            ctime_nsec: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct IoBuffer<'a> {
    data: &'a [u8],
}

impl<'a> IoBuffer<'a> {
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub const fn as_bytes(self) -> &'a [u8] {
        self.data
    }

    pub const fn len(self) -> usize {
        self.data.len()
    }

    pub const fn is_empty(self) -> bool {
        self.data.is_empty()
    }
}

pub struct MutableIoBuffer<'a> {
    data: &'a mut [u8],
    filled: usize,
}

impl<'a> MutableIoBuffer<'a> {
    pub const fn new(data: &'a mut [u8]) -> Self {
        Self { data, filled: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.filled
    }

    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let count = bytes.len().min(self.remaining());
        self.data[self.filled..self.filled + count].copy_from_slice(&bytes[..count]);
        self.filled += count;
        count
    }

    pub const fn len(&self) -> usize {
        self.filled
    }

    pub const fn is_empty(&self) -> bool {
        self.filled == 0
    }

    pub fn filled_bytes(&self) -> &[u8] {
        &self.data[..self.filled]
    }
}

pub struct DirEntry<'a> {
    pub ino: u64,
    pub offset: i64,
    pub file_type: FileType,
    pub name: &'a str,
}

pub fn emit_dirent64(buf: &mut MutableIoBuffer<'_>, entry: DirEntry<'_>) -> Result<bool, Errno> {
    if entry.name.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    let raw_len = 19_usize
        .checked_add(entry.name.len())
        .and_then(|len| len.checked_add(1))
        .ok_or(Errno::Eoverflow)?;
    let record_len = align_up(raw_len, 8).ok_or(Errno::Eoverflow)?;
    let record_len_u16 = u16::try_from(record_len).map_err(|_| Errno::Eoverflow)?;
    if buf.remaining() < record_len {
        return if buf.is_empty() {
            Err(Errno::Einval)
        } else {
            Ok(false)
        };
    }

    let mut header = [0_u8; 19];
    header[0..8].copy_from_slice(&entry.ino.to_ne_bytes());
    header[8..16].copy_from_slice(&entry.offset.to_ne_bytes());
    header[16..18].copy_from_slice(&record_len_u16.to_ne_bytes());
    header[18] = entry.file_type.dirent_type();
    debug_assert_eq!(buf.push(&header), header.len());
    debug_assert_eq!(buf.push(entry.name.as_bytes()), entry.name.len());
    debug_assert_eq!(buf.push(&[0]), 1);

    const ZEROES: [u8; 8] = [0; 8];
    let padding = record_len - raw_len;
    debug_assert_eq!(buf.push(&ZEROES[..padding]), padding);
    Ok(true)
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two());
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

pub trait FileOperations: Send + Sync + 'static {
    fn read(&self, _file: &File, _buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Einval)
    }

    fn write(&self, _file: &File, _buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Einval)
    }

    fn seek(&self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        file.seek_position(offset, whence, None)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        Err(Errno::Einval)
    }

    fn truncate(&self, _file: &File, _length: u64) -> Result<(), Errno> {
        Err(Errno::Einval)
    }

    fn readdir(&self, _file: &File, _buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Enotdir)
    }

    fn ioctl(&self, _file: &File, _cmd: usize, _arg: usize) -> Result<usize, Errno> {
        Err(Errno::Enotty)
    }

    fn poll(&self, file: &File, requested: PollEvents) -> PollEvents {
        let mut ready = PollEvents::empty();
        if file.flags().access_mode().is_readable() {
            ready = ready.union(PollEvents::IN);
        }
        if file.flags().access_mode().is_writable() {
            ready = ready.union(PollEvents::OUT);
        }
        ready.intersect(requested)
    }

    fn sync(&self, _file: &File) -> Result<(), Errno> {
        Ok(())
    }

    fn release(&self, _file: &File) {}
}

struct FileState {
    position: u64,
}

pub struct File {
    flags: OpenFlags,
    path: Option<String>,
    ops: Arc<dyn FileOperations>,
    state: SpinLock<FileState>,
}

pub type ArcFile = Arc<File>;

impl File {
    pub fn new(flags: OpenFlags, ops: Arc<dyn FileOperations>) -> ArcFile {
        Arc::new(Self {
            flags,
            path: None,
            ops,
            state: SpinLock::new(FileState { position: 0 }),
        })
    }

    pub fn new_with_path(flags: OpenFlags, path: String, ops: Arc<dyn FileOperations>) -> ArcFile {
        Arc::new(Self {
            flags,
            path: Some(path),
            ops,
            state: SpinLock::new(FileState { position: 0 }),
        })
    }

    pub const fn flags(&self) -> OpenFlags {
        self.flags
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn read(&self, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        if !self.flags.access_mode().is_readable() {
            return Err(Errno::Ebadf);
        }
        self.ops.read(self, buf)
    }

    pub fn write(&self, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        if !self.flags.access_mode().is_writable() {
            return Err(Errno::Ebadf);
        }
        self.ops.write(self, buf)
    }

    pub fn seek(&self, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        self.ops.seek(self, offset, whence)
    }

    pub fn fstat(&self) -> Result<Stat, Errno> {
        self.ops.fstat(self)
    }

    pub fn truncate(&self, length: u64) -> Result<(), Errno> {
        if !self.flags.access_mode().is_writable() {
            return Err(Errno::Ebadf);
        }
        self.ops.truncate(self, length)
    }

    pub fn readdir(&self, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        self.ops.readdir(self, buf)
    }

    pub fn ioctl(&self, cmd: usize, arg: usize) -> Result<usize, Errno> {
        self.ops.ioctl(self, cmd, arg)
    }

    pub fn poll(&self, requested: PollEvents) -> PollEvents {
        self.ops.poll(self, requested)
    }

    pub fn sync(&self) -> Result<(), Errno> {
        self.ops.sync(self)
    }

    pub fn position(&self) -> u64 {
        self.state.lock().position
    }

    pub fn with_position<R>(&self, f: impl FnOnce(&mut u64) -> R) -> R {
        let mut state = self.state.lock();
        f(&mut state.position)
    }

    pub fn seek_position(
        &self,
        offset: i64,
        whence: SeekWhence,
        end: Option<u64>,
    ) -> Result<u64, Errno> {
        self.with_position(|position| {
            let base = match whence {
                SeekWhence::Set => 0,
                SeekWhence::Current => i64::try_from(*position).map_err(|_| Errno::Eoverflow)?,
                SeekWhence::End => {
                    i64::try_from(end.ok_or(Errno::Espipe)?).map_err(|_| Errno::Eoverflow)?
                }
            };
            let next = base.checked_add(offset).ok_or(Errno::Eoverflow)?;
            if next < 0 {
                return Err(Errno::Einval);
            }
            *position = u64::try_from(next).map_err(|_| Errno::Eoverflow)?;
            Ok(*position)
        })
    }
}

impl Drop for File {
    fn drop(&mut self) {
        self.ops.release(self);
    }
}

pub struct FileDescriptor {
    file: ArcFile,
    close_on_exec: bool,
}

pub struct FileTable<const MAX_FDS: usize> {
    slots: [Option<FileDescriptor>; MAX_FDS],
}

impl<const MAX_FDS: usize> FileTable<MAX_FDS> {
    pub fn new() -> Self {
        Self {
            slots: array::from_fn(|_| None),
        }
    }

    pub const fn capacity(&self) -> usize {
        MAX_FDS
    }

    pub fn open_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn allocate_fd(&mut self, file: ArcFile, close_on_exec: bool) -> Result<usize, Errno> {
        self.allocate_fd_from(file, close_on_exec, 0)
    }

    pub fn allocate_fd_from(
        &mut self,
        file: ArcFile,
        close_on_exec: bool,
        min_fd: usize,
    ) -> Result<usize, Errno> {
        let fd = self.find_free_fd(min_fd)?;
        self.slots[fd] = Some(FileDescriptor {
            file,
            close_on_exec,
        });
        Ok(fd)
    }

    pub fn replace_fd(
        &mut self,
        fd: usize,
        file: ArcFile,
        close_on_exec: bool,
    ) -> Result<(), Errno> {
        let _ = self.replace_fd_take(fd, file, close_on_exec)?;
        Ok(())
    }

    pub fn replace_fd_take(
        &mut self,
        fd: usize,
        file: ArcFile,
        close_on_exec: bool,
    ) -> Result<Option<ArcFile>, Errno> {
        let slot = self.slots.get_mut(fd).ok_or(Errno::Ebadf)?;
        let old = slot.take().map(|descriptor| descriptor.file);
        *slot = Some(FileDescriptor {
            file,
            close_on_exec,
        });
        Ok(old)
    }

    pub fn get_file(&self, fd: usize) -> Result<ArcFile, Errno> {
        self.slots
            .get(fd)
            .and_then(|slot| slot.as_ref())
            .map(|descriptor| Arc::clone(&descriptor.file))
            .ok_or(Errno::Ebadf)
    }

    pub fn fd_flags(&self, fd: usize) -> Result<u32, Errno> {
        self.slots
            .get(fd)
            .and_then(|slot| slot.as_ref())
            .map(|descriptor| u32::from(descriptor.close_on_exec))
            .ok_or(Errno::Ebadf)
    }

    pub fn set_close_on_exec(&mut self, fd: usize, close_on_exec: bool) -> Result<(), Errno> {
        let descriptor = self
            .slots
            .get_mut(fd)
            .and_then(|slot| slot.as_mut())
            .ok_or(Errno::Ebadf)?;
        descriptor.close_on_exec = close_on_exec;
        Ok(())
    }

    pub fn file_flags(&self, fd: usize) -> Result<OpenFlags, Errno> {
        self.slots
            .get(fd)
            .and_then(|slot| slot.as_ref())
            .map(|descriptor| descriptor.file.flags())
            .ok_or(Errno::Ebadf)
    }

    pub fn close(&mut self, fd: usize) -> Result<(), Errno> {
        let _ = self.take_fd(fd)?;
        Ok(())
    }

    pub fn take_fd(&mut self, fd: usize) -> Result<ArcFile, Errno> {
        let slot = self.slots.get_mut(fd).ok_or(Errno::Ebadf)?;
        slot.take()
            .map(|descriptor| descriptor.file)
            .ok_or(Errno::Ebadf)
    }

    pub fn dup_from(
        &mut self,
        old_fd: usize,
        min_fd: usize,
        cloexec: bool,
    ) -> Result<usize, Errno> {
        let file = self.get_file(old_fd)?;
        self.allocate_fd_from(file, cloexec, min_fd)
    }

    pub fn dup_to(&mut self, old_fd: usize, new_fd: usize, cloexec: bool) -> Result<usize, Errno> {
        let (fd, _) = self.dup_to_take(old_fd, new_fd, cloexec)?;
        Ok(fd)
    }

    pub fn dup_to_take(
        &mut self,
        old_fd: usize,
        new_fd: usize,
        cloexec: bool,
    ) -> Result<(usize, Option<ArcFile>), Errno> {
        let file = self.get_file(old_fd)?;
        let old = self.replace_fd_take(new_fd, file, cloexec)?;
        Ok((new_fd, old))
    }

    pub fn close_on_exec(&mut self) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|descriptor| descriptor.close_on_exec)
            {
                *slot = None;
            }
        }
    }

    pub fn take_close_on_exec(&mut self, output: &mut alloc::vec::Vec<ArcFile>) {
        for slot in &mut self.slots {
            if slot
                .as_ref()
                .is_some_and(|descriptor| descriptor.close_on_exec)
                && let Some(descriptor) = slot.take()
            {
                output.push(descriptor.file);
            }
        }
    }

    pub fn fork_clone(&self) -> Self {
        Self {
            slots: array::from_fn(|index| {
                self.slots[index].as_ref().map(|descriptor| FileDescriptor {
                    file: Arc::clone(&descriptor.file),
                    close_on_exec: descriptor.close_on_exec,
                })
            }),
        }
    }

    fn find_free_fd(&self, min_fd: usize) -> Result<usize, Errno> {
        if min_fd >= MAX_FDS {
            return Err(Errno::Emfile);
        }
        for fd in min_fd..MAX_FDS {
            if self.slots[fd].is_none() {
                return Ok(fd);
            }
        }
        Err(Errno::Emfile)
    }
}

impl<const MAX_FDS: usize> Default for FileTable<MAX_FDS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MemoryFile {
        data: SpinLock<alloc::vec::Vec<u8>>,
    }

    impl MemoryFile {
        fn new() -> Self {
            Self {
                data: SpinLock::new(alloc::vec::Vec::new()),
            }
        }
    }

    impl FileOperations for MemoryFile {
        fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
            file.with_position(|position| {
                let data = self.data.lock();
                let start = *position as usize;
                if start >= data.len() {
                    return Ok(0);
                }
                let copied = buf.push(&data[start..]);
                *position += copied as u64;
                Ok(copied)
            })
        }

        fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
            file.with_position(|position| {
                let mut data = self.data.lock();
                let start = *position as usize;
                let end = start + buf.len();
                if end > data.len() {
                    data.resize(end, 0);
                }
                data[start..end].copy_from_slice(buf.as_bytes());
                *position = end as u64;
                Ok(buf.len())
            })
        }
    }

    fn memory_file(flags: OpenFlags) -> ArcFile {
        File::new(flags, Arc::new(MemoryFile::new()))
    }

    #[test]
    fn dup_shares_open_file_position() {
        let mut table: FileTable<8> = FileTable::new();
        let fd0 = table
            .allocate_fd(memory_file(OpenFlags::O_RDWR), false)
            .unwrap();
        let fd1 = table.dup_from(fd0, 0, false).unwrap();

        table
            .get_file(fd0)
            .unwrap()
            .write(&IoBuffer::new(b"abc"))
            .unwrap();
        assert_eq!(table.get_file(fd1).unwrap().position(), 3);
    }

    #[test]
    fn file_table_reuses_lowest_closed_fd() {
        let mut table: FileTable<4> = FileTable::new();
        assert_eq!(
            table
                .allocate_fd(memory_file(OpenFlags::O_RDONLY), false)
                .unwrap(),
            0,
        );
        assert_eq!(
            table
                .allocate_fd(memory_file(OpenFlags::O_RDONLY), false)
                .unwrap(),
            1,
        );
        table.close(0).unwrap();
        assert_eq!(
            table
                .allocate_fd(memory_file(OpenFlags::O_RDONLY), false)
                .unwrap(),
            0,
        );
    }

    #[test]
    fn access_mode_is_enforced() {
        let file = memory_file(OpenFlags::O_RDONLY);
        assert_eq!(file.write(&IoBuffer::new(b"x")), Err(Errno::Ebadf));
    }

    #[test]
    fn fd_flags_and_dup_to_are_tracked() {
        let mut table: FileTable<4> = FileTable::new();
        let fd0 = table
            .allocate_fd(memory_file(OpenFlags::O_RDWR), true)
            .unwrap();
        assert_eq!(table.fd_flags(fd0).unwrap(), 1);
        table.set_close_on_exec(fd0, false).unwrap();
        assert_eq!(table.fd_flags(fd0).unwrap(), 0);

        let fd2 = table.dup_to(fd0, 2, true).unwrap();
        assert_eq!(fd2, 2);
        assert_eq!(table.fd_flags(fd2).unwrap(), 1);
        table
            .get_file(fd0)
            .unwrap()
            .write(&IoBuffer::new(b"xy"))
            .unwrap();
        assert_eq!(table.get_file(fd2).unwrap().position(), 2);
    }

    #[test]
    fn dirent64_emission_respects_record_boundaries() {
        let mut bytes = [0_u8; 64];
        let mut buffer = MutableIoBuffer::new(&mut bytes);
        assert!(
            emit_dirent64(
                &mut buffer,
                DirEntry {
                    ino: 7,
                    offset: 1,
                    file_type: FileType::Regular,
                    name: "name",
                },
            )
            .unwrap()
        );
        assert_eq!(buffer.filled_bytes()[0..8], 7_u64.to_ne_bytes());
        assert!(
            buffer
                .filled_bytes()
                .windows(4)
                .any(|window| window == b"name")
        );

        let mut tiny = [0_u8; 8];
        let mut tiny_buffer = MutableIoBuffer::new(&mut tiny);
        assert_eq!(
            emit_dirent64(
                &mut tiny_buffer,
                DirEntry {
                    ino: 1,
                    offset: 1,
                    file_type: FileType::Directory,
                    name: ".",
                },
            ),
            Err(Errno::Einval),
        );
    }
}
