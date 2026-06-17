//! 文件描述符表（Linux 风格）。
//!
//! 参照 Linux `fs/file_table.c`、`fs/open.c`、`fs/read_write.c`。
//!
//! 提供：
//! - `FileOperations` trait：read/write/close（参照 `struct file_operations`）
//! - `File`：带引用计数的文件对象（参照 `struct file`）
//! - `FileTable`：每进程 fd 数组（参照 `struct files_struct`）
//! - fd 分配与回收（参照 `alloc_fd` / `__close_fd`）

use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Leak detection: number of live File objects.
static LIVE_FILES: AtomicUsize = AtomicUsize::new(0);

/// Assert no File objects have leaked.
pub fn assert_no_leaks() {
    let count = LIVE_FILES.load(Ordering::Acquire);
    assert_eq!(count, 0, "M12 leaked {} File object(s)", count);
}

// ---------------------------------------------------------------------------
// FileOperations trait（参照 Linux `struct file_operations`）
// ---------------------------------------------------------------------------

/// 文件操作抽象（参照 Linux `struct file_operations`）。
///
/// 对于 pipe、socket、设备文件等不同文件类型，各自实现此 trait。
pub trait FileOperations: Send + Sync {
    /// 从文件读取数据。
    /// 返回读取的字节数，0 表示 EOF。
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError>;

    /// 向文件写入数据。
    /// 返回写入的字节数。
    fn write(&self, buf: &[u8]) -> Result<usize, FileError>;

    /// 关闭文件（释放资源）。
    /// 当引用计数归零时调用。
    fn close(&self) {}
}

// ---------------------------------------------------------------------------
// FileError
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileError {
    /// 操作被阻塞（如 pipe 为空且无写入端）
    WouldBlock,
    /// 管道破裂（写入已关闭的 pipe）
    BrokenPipe,
    /// 无效参数
    Invalid,
    /// 资源不足
    NoMemory,
}

impl FileError {
    /// 转换为 Linux errno。
    pub const fn to_errno(self) -> isize {
        match self {
            FileError::WouldBlock => -(11isize), // EAGAIN
            FileError::BrokenPipe => -(13isize),  // EPIPE
            FileError::Invalid => -(22isize),      // EINVAL
            FileError::NoMemory => -(12isize),     // ENOMEM
        }
    }
}

// ---------------------------------------------------------------------------
// OpenFlags
// ---------------------------------------------------------------------------

/// 文件打开标志（简化版，参照 Linux `O_RDONLY` / `O_WRONLY` / `O_RDWR` 等）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenFlags(pub u32);

impl OpenFlags {
    pub const RDONLY: Self = Self(0);
    pub const WRONLY: Self = Self(1);
    pub const RDWR: Self = Self(2);
    pub const NONBLOCK: Self = Self(1 << 11); // O_NONBLOCK
    pub const CLOEXEC: Self = Self(1 << 19);  // O_CLOEXEC

    pub const fn is_readable(self) -> bool {
        self.0 & 3 != Self::WRONLY.0
    }

    pub const fn is_writable(self) -> bool {
        self.0 & 3 != Self::RDONLY.0
    }
}

// ---------------------------------------------------------------------------
// File（参照 Linux `struct file`）
// ---------------------------------------------------------------------------

/// 文件对象（参照 Linux `struct file`）。
///
/// `dup` / `fork` increase the reference count; the final `close` calls
/// `FileOperations::close()` and decrements the leak counter.
pub struct File {
    /// Shared file description — Arc tracks the reference count.
    inner: Arc<FileInner>,
    /// Per-open flags (O_CLOEXEC, etc.).
    flags: OpenFlags,
}

struct FileInner {
    ops: Arc<dyn FileOperations>,
}

impl File {
    pub fn new(ops: Arc<dyn FileOperations>, flags: OpenFlags) -> Self {
        LIVE_FILES.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: Arc::new(FileInner { ops }),
            flags,
        }
    }

    pub fn flags(&self) -> OpenFlags {
        self.flags
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        self.inner.ops.read(buf)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize, FileError> {
        self.inner.ops.write(buf)
    }

    /// Number of open references to this file description.
    pub fn refcount(&self) -> usize {
        Arc::strong_count(&self.inner)
    }
}

impl Clone for File {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            flags: self.flags,
        }
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // When the last File reference to this inner is dropped,
        // call close() and report the release to the leak tracker.
        if Arc::strong_count(&self.inner) == 1 {
            self.inner.ops.close();
            LIVE_FILES.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// ---------------------------------------------------------------------------
// FileTable（参照 Linux `struct files_struct`）
// ---------------------------------------------------------------------------

const FD_TABLE_SIZE: usize = 64;

/// 文件描述符表（参照 Linux `struct files_struct`）。
///
/// 每个进程拥有一个独立的 `FileTable`。
/// `fork` 时会克隆此表（增加引用计数）。
#[derive(Clone)]
pub struct FileTable {
    fds: Vec<Option<File>>,
}

impl FileTable {
    pub fn new() -> Self {
        Self {
            fds: Vec::with_capacity(FD_TABLE_SIZE),
        }
    }

    /// 分配一个文件描述符（参照 Linux `alloc_fd`）。
    ///
    /// 返回最小的可用 fd 编号。
    pub fn alloc_fd(&mut self, file: File) -> Option<usize> {
        // 从 0 开始查找第一个空位
        for (fd, slot) in self.fds.iter().enumerate() {
            if slot.is_none() {
                self.fds[fd] = Some(file);
                return Some(fd);
            }
        }

        // 没有空闲位置，在尾部追加
        if self.fds.len() < FD_TABLE_SIZE {
            self.fds.push(Some(file));
            Some(self.fds.len() - 1)
        } else {
            None // EMFILE
        }
    }

    /// 获取文件描述符对应的文件（增加引用计数）。
    pub fn get_file(&self, fd: usize) -> Option<File> {
        self.fds.get(fd).and_then(|slot| slot.clone())
    }

    /// 关闭文件描述符（参照 Linux `__close_fd`）。
    ///
    /// 减少引用计数，引用计数归零时释放资源。
    pub fn close_fd(&mut self, fd: usize) -> bool {
        if fd >= self.fds.len() {
            return false;
        }
        self.fds[fd].take().is_some()
    }

    /// Take a file descriptor out of the table without dropping it.
    /// The caller is responsible for dropping the File (which may trigger
    /// close() callbacks that acquire lower-rank locks like WaitQueue).
    pub fn take_fd(&mut self, fd: usize) -> Option<File> {
        if fd >= self.fds.len() {
            return None;
        }
        self.fds[fd].take()
    }

    /// 复制文件描述符（参照 Linux `dup_fd` / `dup3`）。
    ///
    /// 新 fd 指向与旧 fd 相同的文件对象（引用计数 +1）。
    pub fn dup_fd(&mut self, old_fd: usize, new_fd: usize) -> Result<usize, FileError> {
        let file = self.get_file(old_fd).ok_or(FileError::Invalid)?;

        if new_fd >= FD_TABLE_SIZE {
            return Err(FileError::Invalid);
        }

        // 如果 new_fd 已打开，先关闭
        self.close_fd(new_fd);

        // 扩容到足够大
        while self.fds.len() <= new_fd {
            self.fds.push(None);
        }

        self.fds[new_fd] = Some(file);
        Ok(new_fd)
    }

    /// 关闭所有文件描述符（进程退出时调用）。
    pub fn close_all(&mut self) {
        for slot in self.fds.iter_mut() {
            *slot = None;
        }
    }

    /// Take all files out of the table (for lock-safe teardown).
    pub fn take_all(&mut self) -> alloc::vec::Vec<Option<File>> {
        let mut result = alloc::vec::Vec::new();
        core::mem::swap(&mut result, &mut self.fds);
        result
    }

    /// fd 数量。
    pub fn len(&self) -> usize {
        self.fds.iter().filter(|slot| slot.is_some()).count()
    }
}
