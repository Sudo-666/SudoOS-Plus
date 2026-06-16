use crate::{Errno, IoBuffer, MutableIoBuffer, SeekWhence, Stat};

/// Returned from `FileOperations::readdir()` to signal the end of a directory.
///
/// If `push()` returns `None`, the directory has been fully read; the caller
/// should not call `readdir()` again until the file position is reset.
pub struct ReadDirEntries<'a> {
    buffer: &'a mut [u8],
    filled: usize,
    done: bool,
}

impl<'a> ReadDirEntries<'a> {
    /// Create a new collection backed by the given user-space buffer.
    pub const fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            filled: 0,
            done: false,
        }
    }

    /// Number of bytes written so far.
    pub fn written(&self) -> usize {
        self.filled
    }

    /// Mark the directory as fully consumed.
    pub fn mark_done(&mut self) {
        self.done = true;
    }

    /// Try to append a single directory entry.
    ///
    /// Returns `Some(())` on success, or `None` if the buffer does not have
    /// enough space — the caller should stop and return `written()` bytes.
    pub fn push(&mut self, entry: &crate::DirEntry) -> Option<()> {
        if self.done {
            return None;
        }
        let remaining = &mut self.buffer[self.filled..];
        let n = entry.write_to(remaining)?;
        self.filled += n;
        Some(())
    }
}

/// Poll readiness status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PollStatus {
    pub readable: bool,
    pub writable: bool,
    pub error: bool,
}

impl PollStatus {
    pub const fn empty() -> Self {
        Self {
            readable: false,
            writable: false,
            error: false,
        }
    }
}

/// File operations vtable — analogous to Linux `struct file_operations`.
///
/// Every open file has a `Box<dyn FileOperations>` that implements the
/// behaviour for read, write, seek, etc.  Filesystem implementations
/// provide concrete types that implement this trait.
///
/// # Thread safety
///
/// The trait requires `Send` so that files can be transferred between
/// CPUs.  The kernel crate is responsible for serialising concurrent
/// access (typically via a per-inode or per-file lock).
pub trait FileOperations: Send + 'static {
    /// Read data from the file starting at the current file position.
    ///
    /// Returns the number of bytes read, or `0` for EOF.  The file
    /// position is advanced by the returned byte count.
    fn read(&self, file: &crate::File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno>;

    /// Write data to the file at the current file position.
    ///
    /// Returns the number of bytes written.  For regular files the file
    /// position is advanced; for devices the interpretation is driver-specific.
    fn write(&mut self, file: &crate::File, buf: &IoBuffer<'_>) -> Result<usize, Errno>;

    /// Reposition the file offset.
    ///
    /// Returns the new absolute offset on success.
    fn seek(&mut self, file: &crate::File, offset: i64, whence: SeekWhence) -> Result<u64, Errno>;

    /// Called when the last reference to the file is dropped (`close()`).
    ///
    /// The default implementation is a no-op.
    fn release(&mut self, _file: &crate::File) {}

    /// Device-specific control operation.
    ///
    /// Returns `-ENOTTY` by default for regular files.
    fn ioctl(&mut self, _file: &crate::File, _cmd: u64, _arg: usize) -> Result<usize, Errno> {
        Err(Errno::ENOTTY)
    }

    /// Fill a `Stat` struct with the file's metadata.
    fn fstat(&self, file: &crate::File) -> Result<Stat, Errno>;

    /// Prepare a file-backed memory mapping.
    ///
    /// Returns an opaque object identifier that is stored in
    /// `VmAreaKind::FileBacked.object`.  The page-fault handler uses this
    /// ID to locate the filesystem object when resolving `LoadFile` faults.
    ///
    /// Returns `-ENODEV` by default (not supported by this file).
    fn mmap(&self, _file: &crate::File, _offset: u64) -> Result<u64, Errno> {
        Err(Errno::ENODEV)
    }

    /// Flush buffered data to storage.
    ///
    /// For tmpfs this is a no-op.  For on-disk filesystems this may trigger
    /// a writeback of dirty pages.
    fn fsync(&mut self, _file: &crate::File) -> Result<(), Errno> {
        Ok(())
    }

    /// Read directory entries from an open directory file.
    ///
    /// Appends `DirEntry` records to `entries` until the buffer is full or
    /// the directory is exhausted.  Returns the number of bytes written.
    ///
    /// Returns `-ENOTDIR` by default (for non-directory files).
    fn readdir(
        &self,
        _file: &crate::File,
        _entries: &mut ReadDirEntries<'_>,
    ) -> Result<usize, Errno> {
        Err(Errno::ENOTDIR)
    }

    /// Check poll readiness.
    ///
    /// Returns readiness flags for `poll`/`select`/`epoll` integration.
    fn poll(&self, _file: &crate::File) -> Result<PollStatus, Errno> {
        Ok(PollStatus {
            readable: true,
            writable: true,
            error: false,
        })
    }
}
