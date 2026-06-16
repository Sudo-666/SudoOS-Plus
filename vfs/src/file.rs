use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{Errno, FileMode, IoBuffer, MutableIoBuffer, OpenFlags, SeekWhence, Stat};

/// An open file — analogous to Linux `struct file`.
///
/// Represents a single open file descriptor.  Multiple file descriptors
/// (via `dup`) can share the same `File` object; each `ArcFile` handle
/// increments `f_count`.
pub struct File {
    /// Access mode (read/write permission of this open instance).
    f_mode: FileMode,
    /// Current seek position (updated by `read`/`write`/`seek`).
    f_pos: AtomicU64,
    /// Open flags (O_RDONLY, O_APPEND, O_CLOEXEC, etc.).
    f_flags: OpenFlags,
    /// Reference count (shared between `dup`'d descriptors).
    f_count: AtomicU32,
    /// File operations vtable.
    f_op: Option<alloc::boxed::Box<dyn crate::FileOperations>>,
    /// Associated dentry (for path information, inode access).
    f_dentry: crate::DentryRef,
}

impl File {
    pub fn new(
        mode: FileMode,
        flags: OpenFlags,
        ops: alloc::boxed::Box<dyn crate::FileOperations>,
        dentry: crate::DentryRef,
    ) -> Self {
        Self {
            f_mode: mode,
            f_pos: AtomicU64::new(0),
            f_flags: flags,
            f_count: AtomicU32::new(1),
            f_op: Some(ops),
            f_dentry: dentry,
        }
    }

    // --- Field accessors ---

    pub const fn f_mode(&self) -> FileMode {
        self.f_mode
    }

    pub const fn f_flags(&self) -> OpenFlags {
        self.f_flags
    }

    pub fn f_pos(&self) -> u64 {
        self.f_pos.load(Ordering::Acquire)
    }

    pub fn set_f_pos(&self, pos: u64) {
        self.f_pos.store(pos, Ordering::Release);
    }

    pub fn advance_f_pos(&self, n: usize) -> u64 {
        self.f_pos.fetch_add(n as u64, Ordering::AcqRel) + n as u64
    }

    pub fn f_dentry(&self) -> &crate::DentryRef {
        &self.f_dentry
    }

    pub fn f_count(&self) -> u32 {
        self.f_count.load(Ordering::Acquire)
    }

    pub fn f_op(&self) -> Option<&dyn crate::FileOperations> {
        self.f_op.as_ref().map(|b| b.as_ref())
    }

    pub fn f_op_mut(&mut self) -> Option<&mut dyn crate::FileOperations> {
        self.f_op.as_mut().map(|b| b.as_mut())
    }

    // --- Reference counting ---

    fn inc_ref(&self) -> u32 {
        self.f_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn dec_ref(&self) -> u32 {
        self.f_count.fetch_sub(1, Ordering::AcqRel) - 1
    }

    // --- Convenience wrappers that delegate to f_op ---

    /// Read data from the file via `f_op->read()`.
    pub fn read(&self, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        let Some(op) = self.f_op() else {
            return Err(Errno::EBADF);
        };
        op.read(self, buf)
    }

    /// Write data to the file via `f_op->write()`.
    ///
    /// Uses a raw pointer to avoid the Rust borrow checker preventing
    /// `&mut self.f_op` and `&self` from coexisting in the same call.
    pub fn write(&mut self, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        let op_ptr: *mut dyn crate::FileOperations = match &mut self.f_op {
            Some(op) => op.as_mut(),
            None => return Err(Errno::EBADF),
        };
        // SAFETY: f_op is `Some` and the pointer is valid for the duration
        // of the call.  The caller serialises access via external locking.
        unsafe { (*op_ptr).write(self, buf) }
    }

    /// Reposition the file offset via `f_op->seek()`.
    pub fn seek(&mut self, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        let op_ptr: *mut dyn crate::FileOperations = match &mut self.f_op {
            Some(op) => op.as_mut(),
            None => return Err(Errno::EBADF),
        };
        // SAFETY: f_op is `Some` and the pointer is valid for the duration
        // of the call.
        unsafe { (*op_ptr).seek(self, offset, whence) }
    }

    /// Fill a `Stat` struct via `f_op->fstat()`.
    pub fn fstat(&self) -> Result<Stat, Errno> {
        let Some(op) = self.f_op() else {
            return Err(Errno::EBADF);
        };
        op.fstat(self)
    }

    /// Device control operation via `f_op->ioctl()`.
    pub fn ioctl(&mut self, cmd: u64, arg: usize) -> Result<usize, Errno> {
        let op_ptr: *mut dyn crate::FileOperations = match &mut self.f_op {
            Some(op) => op.as_mut(),
            None => return Err(Errno::EBADF),
        };
        // SAFETY: f_op is `Some` and the pointer is valid for the duration
        // of the call.
        unsafe { (*op_ptr).ioctl(self, cmd, arg) }
    }

    /// Read directory entries via `f_op->readdir()`.
    pub fn readdir(&self, entries: &mut crate::ReadDirEntries<'_>) -> Result<usize, Errno> {
        let Some(op) = self.f_op() else {
            return Err(Errno::EBADF);
        };
        op.readdir(self, entries)
    }

    /// Poll readiness via `f_op->poll()`.
    pub fn poll(&self) -> Result<crate::PollStatus, Errno> {
        let Some(op) = self.f_op() else {
            return Err(Errno::EBADF);
        };
        op.poll(self)
    }
}

/// Reference-counted handle to a heap-allocated `File`.
///
/// `dup()` clones the handle incrementing the reference count;
/// `close()` drops the handle and calls `release()` when the
/// count reaches zero.
pub struct ArcFile {
    ptr: *const File,
}

#[allow(clippy::should_implement_trait)]
impl ArcFile {
    pub fn new(file: File) -> Self {
        Self {
            ptr: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(file)),
        }
    }

    pub fn as_ref(&self) -> &File {
        // SAFETY: pointer is valid as long as any ArcFile exists.
        unsafe { &*self.ptr }
    }

    pub fn as_mut(&mut self) -> &mut File {
        // SAFETY: &mut self guarantees exclusive access.
        unsafe { &mut *(self.ptr as *mut File) }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.ptr, other.ptr)
    }
}

impl Clone for ArcFile {
    fn clone(&self) -> Self {
        self.as_ref().inc_ref();
        Self { ptr: self.ptr }
    }
}

impl Drop for ArcFile {
    fn drop(&mut self) {
        let file = self.as_ref();
        let prev = file.dec_ref();
        if prev == 0 {
            // SAFETY: this is the last reference, so no concurrent access
            // is possible.  Temporarily take the f_op to call release(),
            // then put it back before the Box::from_raw frees everything.
            let file_mut: &mut File = unsafe { &mut *(self.ptr as *mut File) };
            if let Some(mut op) = file_mut.f_op.take() {
                op.release(file_mut);
                file_mut.f_op = Some(op);
            }
            // SAFETY: last reference, no concurrent access.
            unsafe {
                drop(alloc::boxed::Box::from_raw(self.ptr as *mut File));
            }
        }
    }
}

// SAFETY: ArcFile's refcount is AtomicU32 — the underlying File is
// safe to access from any thread as long as references are managed correctly.
unsafe impl Send for ArcFile {}
// SAFETY: &ArcFile only provides &File (immutable access), all mutable
// state is behind AtomicU32/AtomicU64.
unsafe impl Sync for ArcFile {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ArcInode, Dentry, DentryRef, FileMode, Inode, InodeId,
        MutableIoBuffer, OpenFlags, SeekWhence, Stat,
    };
    use alloc::boxed::Box;

    struct TestFileOps {
        data: alloc::vec::Vec<u8>,
    }

    impl crate::FileOperations for TestFileOps {
        fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
            let offset = file.f_pos() as usize;
            if offset >= self.data.len() {
                return Ok(0);
            }
            let n = buf.remaining().min(self.data.len() - offset);
            buf.fill(&self.data[offset..offset + n]);
            file.advance_f_pos(n);
            Ok(n)
        }

        fn write(&mut self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
            let offset = file.f_pos() as usize;
            let n = buf.len();
            if offset + n > self.data.len() {
                self.data.resize(offset + n, 0);
            }
            self.data[offset..offset + n].copy_from_slice(buf.as_bytes());
            file.advance_f_pos(n);
            Ok(n)
        }

        fn seek(&mut self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
            let base = match whence {
                SeekWhence::Set => 0i64,
                SeekWhence::Current => file.f_pos() as i64,
                SeekWhence::End => self.data.len() as i64,
            };
            let new_pos = base.checked_add(offset).ok_or(Errno::EINVAL)?;
            if new_pos < 0 {
                return Err(Errno::EINVAL);
            }
            file.set_f_pos(new_pos as u64);
            Ok(new_pos as u64)
        }

        fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
            Ok(Stat::zeroed())
        }
    }

    fn make_test_file() -> ArcFile {
        make_test_file_with_capacity(100)
    }

    fn make_test_file_empty() -> ArcFile {
        make_test_file_with_capacity(0)
    }

    fn make_test_file_with_capacity(size: usize) -> ArcFile {
        let inode = ArcInode::new(Inode::new(InodeId::new(1), FileMode::FILE_DEFAULT, false));
        let dentry = DentryRef::new(Dentry::new("test", Some(inode), None, None));
        ArcFile::new(File::new(
            FileMode::FILE_DEFAULT,
            OpenFlags::O_RDWR,
            Box::new(TestFileOps { data: alloc::vec![0u8; size] }),
            dentry,
        ))
    }

    #[test]
    fn file_read_write() {
        let mut file = make_test_file();

        // Write "hello" at offset 0
        let buf = IoBuffer::new(b"hello");
        file.as_mut().write(&buf).unwrap();
        assert_eq!(file.as_ref().f_pos(), 5);

        // Seek to 0 and read back — file has 100 bytes (5 of "hello", 95 zeros)
        file.as_mut().seek(0, SeekWhence::Set).unwrap();
        let mut data = [0u8; 10];
        let mut mbuf = MutableIoBuffer::new(&mut data);
        let n = file.as_ref().read(&mut mbuf).unwrap();
        assert_eq!(n, 10);
        assert_eq!(&mbuf.filled_bytes()[..5], b"hello");
    }

    #[test]
    fn file_seek_set_current_end() {
        let mut file = make_test_file();

        // Write 100 bytes
        let buf = IoBuffer::new(&[b'A'; 100]);
        file.as_mut().write(&buf).unwrap();
        assert_eq!(file.as_ref().f_pos(), 100);

        // SEEK_SET to 50
        file.as_mut().seek(50, SeekWhence::Set).unwrap();
        assert_eq!(file.as_ref().f_pos(), 50);

        // SEEK_CUR +10 → 60
        file.as_mut().seek(10, SeekWhence::Current).unwrap();
        assert_eq!(file.as_ref().f_pos(), 60);

        // SEEK_END -10 → 90
        file.as_mut().seek(-10, SeekWhence::End).unwrap();
        assert_eq!(file.as_ref().f_pos(), 90);
    }

    #[test]
    fn file_seek_before_zero_rejected() {
        let mut file = make_test_file();
        assert!(file.as_mut().seek(-1, SeekWhence::Set).is_err());
    }

    #[test]
    fn arc_file_refcount() {
        let f1 = make_test_file();
        assert_eq!(f1.as_ref().f_count(), 1);

        let f2 = f1.clone();
        assert_eq!(f1.as_ref().f_count(), 2);

        drop(f2);
        assert_eq!(f1.as_ref().f_count(), 1);
    }

    #[test]
    fn file_eof_returns_zero() {
        // Empty file at offset 0 → EOF
        let file = make_test_file_empty();
        let mut data = [0u8; 10];
        let mut mbuf = MutableIoBuffer::new(&mut data);
        let n = file.as_ref().read(&mut mbuf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn write_past_end_extends_file() {
        let mut file = make_test_file_empty();
        // Write at offset 5 (past current EOF which is 0)
        file.as_mut().seek(5, SeekWhence::Set).unwrap();
        let buf = IoBuffer::new(b"world");
        let n = file.as_mut().write(&buf).unwrap();
        assert_eq!(n, 5);
        assert_eq!(file.as_ref().f_pos(), 10);

        // Read back from 0
        file.as_mut().seek(0, SeekWhence::Set).unwrap();
        let mut data = [0u8; 10];
        let mut mbuf = MutableIoBuffer::new(&mut data);
        let n = file.as_ref().read(&mut mbuf).unwrap();
        assert_eq!(n, 10);
        assert_eq!(&mbuf.filled_bytes()[..5], &[0u8; 5]); // gap filled with zeros
        assert_eq!(&mbuf.filled_bytes()[5..], b"world");
    }
}

