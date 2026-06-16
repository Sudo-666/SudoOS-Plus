use crate::{ArcFile, OpenFlags};

/// Error returned by `FileTable` operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileTableError {
    /// The process has reached its file descriptor limit (`EMFILE`).
    TooManyFiles,
    /// The given fd is not currently open (`EBADF`).
    BadDescriptor,
    /// No space available in the table (internal capacity limit).
    NoSpace,
}

/// A single slot in the file descriptor table.
pub struct FileSlot {
    file: ArcFile,
    close_on_exec: bool,
}

/// Per-process file descriptor table — analogous to Linux `struct files_struct`.
///
/// File descriptors are small integers that index into `slots`.  The table
/// capacity is fixed at compile time via the `MAX_FDS` const generic.
/// Simple linear scan is used for fd allocation (sufficient for typical
/// `MAX_FDS` values like 128 or 256).
pub struct FileTable<const MAX_FDS: usize> {
    slots: [Option<FileSlot>; MAX_FDS],
    next_fd: usize,
}

impl<const MAX_FDS: usize> Default for FileTable<MAX_FDS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const MAX_FDS: usize> FileTable<MAX_FDS> {
    /// Create an empty file descriptor table.
    pub const fn new() -> Self {
        const EMPTY_SLOT: Option<FileSlot> = None;
        Self {
            slots: [EMPTY_SLOT; MAX_FDS],
            next_fd: 0,
        }
    }

    /// Total capacity of the table.
    pub const fn capacity(&self) -> usize {
        MAX_FDS
    }

    /// Number of currently open file descriptors.
    pub fn open_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// Allocate the lowest available fd number and insert `file`.
    ///
    /// Returns the new fd on success, or `TooManyFiles` if no slots
    /// are available.
    pub fn allocate_fd(
        &mut self,
        file: ArcFile,
        close_on_exec: bool,
    ) -> Result<usize, FileTableError> {
        let fd = self.find_free_fd(self.next_fd)?;

        self.slots[fd] = Some(FileSlot {
            file,
            close_on_exec,
        });
        self.next_fd = (fd + 1) % MAX_FDS;

        Ok(fd)
    }

    /// Allocate a specific fd, used by `dup3()` or `F_DUPFD`.
    ///
    /// If `fd` is already in use, it is silently closed first (matching
    /// Linux `dup2`/`dup3` behaviour).
    pub fn allocate_fd_at(
        &mut self,
        fd: usize,
        file: ArcFile,
        close_on_exec: bool,
    ) -> Result<usize, FileTableError> {
        if fd >= MAX_FDS {
            return Err(FileTableError::TooManyFiles);
        }

        if self.is_allocated(fd) {
            self.close(fd);
        }

        self.slots[fd] = Some(FileSlot {
            file,
            close_on_exec,
        });
        Ok(fd)
    }

    /// Get a reference to the file at `fd`.
    pub fn get_file(&self, fd: usize) -> Option<&ArcFile> {
        if fd >= MAX_FDS {
            return None;
        }
        self.slots[fd].as_ref().map(|s| &s.file)
    }

    /// Get a mutable reference to the file at `fd`.
    pub fn get_file_mut(&mut self, fd: usize) -> Option<&mut ArcFile> {
        if fd >= MAX_FDS {
            return None;
        }
        self.slots[fd].as_mut().map(|s| &mut s.file)
    }

    /// Close the file descriptor `fd`, returning the `FileSlot` if it was open.
    ///
    /// The caller is responsible for calling `release()` on the file
    /// (which happens automatically when the `ArcFile` is dropped).
    pub fn close(&mut self, fd: usize) -> Option<FileSlot> {
        if fd >= MAX_FDS || !self.is_allocated(fd) {
            return None;
        }
        self.slots[fd].take()
    }

    /// Duplicate `old_fd` to the lowest available fd >= `new_fd_min`.
    ///
    /// Returns the new fd number, or `TooManyFiles` if no slot is available.
    pub fn dup(
        &mut self,
        old_fd: usize,
        new_fd_min: usize,
        cloexec: bool,
    ) -> Result<usize, FileTableError> {
        let file = self
            .get_file(old_fd)
            .ok_or(FileTableError::BadDescriptor)?
            .clone();

        let fd = self.find_free_fd(new_fd_min)?;
        self.slots[fd] = Some(FileSlot {
            file,
            close_on_exec: cloexec,
        });
        self.next_fd = (fd + 1) % MAX_FDS;
        Ok(fd)
    }

    /// Set the close-on-exec flag for `fd`.
    pub fn set_cloexec(&mut self, fd: usize) -> Result<(), FileTableError> {
        let slot = self
            .slots
            .get_mut(fd)
            .and_then(|s| s.as_mut())
            .ok_or(FileTableError::BadDescriptor)?;
        slot.close_on_exec = true;
        Ok(())
    }

    /// Clear the close-on-exec flag for `fd`.
    pub fn clear_cloexec(&mut self, fd: usize) -> Result<(), FileTableError> {
        let slot = self
            .slots
            .get_mut(fd)
            .and_then(|s| s.as_mut())
            .ok_or(FileTableError::BadDescriptor)?;
        slot.close_on_exec = false;
        Ok(())
    }

    /// Check whether `fd` has the close-on-exec flag set.
    pub fn get_cloexec(&self, fd: usize) -> Result<bool, FileTableError> {
        self.slots
            .get(fd)
            .and_then(|s| s.as_ref())
            .map(|s| s.close_on_exec)
            .ok_or(FileTableError::BadDescriptor)
    }

    /// Close all file descriptors that have the close-on-exec flag set.
    pub fn close_all_cloexec(&mut self) {
        for fd in 0..MAX_FDS {
            if let Some(slot) = &self.slots[fd]
                && slot.close_on_exec
            {
                self.slots[fd] = None;
            }
        }
    }

    /// Get file status flags (for `F_GETFL`).
    pub fn get_flags(&self, fd: usize) -> Result<OpenFlags, FileTableError> {
        self.get_file(fd)
            .map(|f| f.as_ref().f_flags())
            .ok_or(FileTableError::BadDescriptor)
    }

    // --- Internal helpers ---

    fn is_allocated(&self, fd: usize) -> bool {
        fd < MAX_FDS && self.slots[fd].is_some()
    }

    fn find_free_fd(&self, start: usize) -> Result<usize, FileTableError> {
        for i in 0..MAX_FDS {
            let fd = (start + i) % MAX_FDS;
            if !self.is_allocated(fd) {
                return Ok(fd);
            }
        }
        Err(FileTableError::TooManyFiles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Dentry, DentryRef, Errno, File, FileMode, Inode, InodeId, MutableIoBuffer};
    use alloc::boxed::Box;

    /// A minimal stub FileOperations for testing the file table.
    struct StubFile;

    impl crate::FileOperations for StubFile {
        fn read(
            &self,
            _file: &crate::File,
            _buf: &mut MutableIoBuffer<'_>,
        ) -> Result<usize, Errno> {
            Err(Errno::EINVAL)
        }
        fn write(
            &mut self,
            _file: &crate::File,
            _buf: &crate::IoBuffer<'_>,
        ) -> Result<usize, Errno> {
            Err(Errno::EINVAL)
        }
        fn seek(
            &mut self,
            _file: &crate::File,
            _offset: i64,
            _whence: crate::SeekWhence,
        ) -> Result<u64, Errno> {
            Err(Errno::EINVAL)
        }
        fn fstat(&self, _file: &crate::File) -> Result<crate::Stat, Errno> {
            Ok(crate::Stat::zeroed())
        }
    }

    fn make_test_file() -> ArcFile {
        let inode =
            crate::ArcInode::new(Inode::new(InodeId::new(1), FileMode::FILE_DEFAULT, false));
        let dentry = DentryRef::new(Dentry::new("test", Some(inode), None, None));
        ArcFile::new(File::new(
            FileMode::FILE_DEFAULT,
            OpenFlags::O_RDWR,
            Box::new(StubFile),
            dentry,
        ))
    }

    #[test]
    fn allocate_and_get() {
        let mut table: FileTable<8> = FileTable::new();
        let file = make_test_file();

        let fd = table.allocate_fd(file, false).unwrap();
        assert_eq!(fd, 0);
        assert!(table.get_file(fd).is_some());
    }

    #[test]
    fn close_removes_entry() {
        let mut table: FileTable<8> = FileTable::new();
        let fd = table.allocate_fd(make_test_file(), false).unwrap();
        assert!(table.get_file(fd).is_some());

        let slot = table.close(fd).unwrap();
        assert!(table.get_file(fd).is_none());
        drop(slot);
    }

    #[test]
    fn dup_shares_file() {
        let mut table: FileTable<8> = FileTable::new();
        let original = table.allocate_fd(make_test_file(), false).unwrap();

        let duped = table.dup(original, 0, false).unwrap();
        assert_ne!(original, duped);
        assert!(table
            .get_file(original)
            .unwrap()
            .ptr_eq(table.get_file(duped).unwrap()));
    }

    #[test]
    fn bad_fd_returns_none() {
        let table: FileTable<8> = FileTable::new();
        assert!(table.get_file(0).is_none());
    }

    #[test]
    fn capacity_exhaustion() {
        let mut table: FileTable<3> = FileTable::new();
        table.allocate_fd(make_test_file(), false).unwrap();
        table.allocate_fd(make_test_file(), false).unwrap();
        table.allocate_fd(make_test_file(), false).unwrap();

        assert!(matches!(
            table.allocate_fd(make_test_file(), false),
            Err(FileTableError::TooManyFiles),
        ));
    }

    #[test]
    fn cloexec_flag() {
        let mut table: FileTable<8> = FileTable::new();
        let fd = table.allocate_fd(make_test_file(), true).unwrap();

        assert!(table.get_cloexec(fd).unwrap());
        table.clear_cloexec(fd).unwrap();
        assert!(!table.get_cloexec(fd).unwrap());
        table.set_cloexec(fd).unwrap();
        assert!(table.get_cloexec(fd).unwrap());
    }

    #[test]
    fn close_all_cloexec() {
        let mut table: FileTable<8> = FileTable::new();
        let keep = table.allocate_fd(make_test_file(), false).unwrap();
        let drop_fd = table.allocate_fd(make_test_file(), true).unwrap();

        table.close_all_cloexec();
        assert!(table.get_file(keep).is_some());
        assert!(table.get_file(drop_fd).is_none());
    }

    #[test]
    fn allocate_fd_at_specific_slot() {
        let mut table: FileTable<8> = FileTable::new();
        let fd = table.allocate_fd_at(5, make_test_file(), false).unwrap();
        assert_eq!(fd, 5);
        assert!(table.get_file(5).is_some());
        // Slots 0-4 should be empty
        for i in 0..5 {
            assert!(table.get_file(i).is_none());
        }
    }

    #[test]
    fn allocate_fd_at_replaces_existing() {
        let mut table: FileTable<8> = FileTable::new();
        let _first = table.allocate_fd(make_test_file(), false).unwrap();
        // Replace fd 0 with a new file
        let new_file = make_test_file();
        let fd = table.allocate_fd_at(0, new_file, false).unwrap();
        assert_eq!(fd, 0);
        assert!(table.get_file(0).is_some());
    }

    #[test]
    fn allocate_fd_at_out_of_range() {
        let mut table: FileTable<4> = FileTable::new();
        assert!(matches!(
            table.allocate_fd_at(5, make_test_file(), false),
            Err(FileTableError::TooManyFiles),
        ));
    }

    #[test]
    fn dup_with_min_fd() {
        let mut table: FileTable<8> = FileTable::new();
        let original = table.allocate_fd(make_test_file(), false).unwrap();
        assert_eq!(original, 0);

        // dup with min_fd=5 should get fd 5
        let duped = table.dup(original, 5, true).unwrap();
        assert_eq!(duped, 5);
        assert!(table.get_cloexec(duped).unwrap());
    }

    #[test]
    fn dup_bad_fd() {
        let mut table: FileTable<8> = FileTable::new();
        assert!(matches!(
            table.dup(99, 0, false),
            Err(FileTableError::BadDescriptor),
        ));
    }

    #[test]
    fn open_count_tracks_files() {
        let mut table: FileTable<8> = FileTable::new();
        assert_eq!(table.open_count(), 0);
        table.allocate_fd(make_test_file(), false).unwrap();
        assert_eq!(table.open_count(), 1);
        table.allocate_fd(make_test_file(), false).unwrap();
        assert_eq!(table.open_count(), 2);
        table.close(0);
        assert_eq!(table.open_count(), 1);
    }

    #[test]
    fn reuse_fd_after_close() {
        let mut table: FileTable<8> = FileTable::new();
        table.allocate_fd(make_test_file(), false).unwrap(); // fd 0
        table.allocate_fd(make_test_file(), false).unwrap(); // fd 1
        table.close(0);
        // next_fd is 2, so search starts at 2; fd 2 is the next free
        let fd = table.allocate_fd(make_test_file(), false).unwrap();
        assert_eq!(fd, 2);
        // After exhausting, wraps around to fd 0
        for _i in 3..8 {
            table.allocate_fd(make_test_file(), false).unwrap();
        }
        let fd = table.allocate_fd(make_test_file(), false).unwrap();
        assert_eq!(fd, 0);
    }
}
