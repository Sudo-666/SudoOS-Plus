use crate::{Dev, Errno, FileMode, InodeId, RenameFlags, Stat};

/// Inode operations vtable — analogous to Linux `struct inode_operations`.
///
/// Each filesystem provides a concrete implementation that handles directory
/// traversal, file creation, linking, and attribute access.
pub trait InodeOperations: Send + 'static {
    /// Create a new regular file in `dir` with the given `name` and `mode`.
    ///
    /// Returns the inode ID of the newly created file.
    fn create(&self, dir: &crate::Inode, name: &str, mode: FileMode) -> Result<InodeId, Errno>;

    /// Look up `name` in the directory `dir`.
    ///
    /// Returns the inode ID of the found entry, or `-ENOENT`.
    fn lookup(&self, dir: &crate::Inode, name: &str) -> Result<InodeId, Errno>;

    /// Create a hard link: `old` → `dir/name`.
    ///
    /// Returns `-EMLINK` if the link count would overflow.
    fn link(
        &self,
        old: &crate::Inode,
        dir: &crate::Inode,
        name: &str,
    ) -> Result<(), Errno>;

    /// Remove `name` from the directory `dir`.
    ///
    /// Decrements the inode link count; the inode is freed when it reaches
    /// zero and all open file handles are closed.
    fn unlink(&self, dir: &crate::Inode, name: &str) -> Result<(), Errno>;

    /// Create a new subdirectory in `dir`.
    fn mkdir(
        &self,
        dir: &crate::Inode,
        name: &str,
        mode: FileMode,
    ) -> Result<InodeId, Errno>;

    /// Remove an empty subdirectory from `dir`.
    ///
    /// Returns `-ENOTEMPTY` if the directory still contains entries.
    fn rmdir(&self, dir: &crate::Inode, name: &str) -> Result<(), Errno>;

    /// Rename (or exchange) an entry.
    ///
    /// `flags` controls whether to fail on existing targets (`RENAME_NOREPLACE`)
    /// or exchange atomically (`RENAME_EXCHANGE`).  Only files within the same
    /// filesystem are supported; `-EXDEV` is returned for cross-fs renames.
    fn rename(
        &self,
        old_dir: &crate::Inode,
        old_name: &str,
        new_dir: &crate::Inode,
        new_name: &str,
        flags: RenameFlags,
    ) -> Result<(), Errno>;

    /// Create a symbolic link `dir/name` → `target`.
    fn symlink(
        &self,
        dir: &crate::Inode,
        name: &str,
        target: &str,
    ) -> Result<InodeId, Errno>;

    /// Read the target of a symbolic link.
    ///
    /// Writes the null-terminated target path into `buffer`.  Returns the
    /// number of bytes written (including the null terminator), or `-ERANGE`
    /// if `buffer` is too small.
    fn readlink(&self, inode: &crate::Inode, buffer: &mut [u8]) -> Result<usize, Errno>;

    /// Create a device special file (character or block device).
    fn mknod(
        &self,
        dir: &crate::Inode,
        name: &str,
        mode: FileMode,
        dev: Dev,
    ) -> Result<InodeId, Errno>;

    /// Fill a `Stat` struct for this inode.
    fn getattr(&self, inode: &crate::Inode) -> Result<Stat, Errno>;

    /// Update inode attributes (only size for now).
    fn setattr(&self, _inode: &crate::Inode, _stat: &Stat) -> Result<(), Errno> {
        Ok(())
    }

    /// Open this inode, returning a file-operations object.
    ///
    /// This is the bridge from directory traversal to I/O: `lookup()` finds
    /// the inode, then `open()` returns the `FileOperations` that handle
    /// `read`/`write`/`seek` for the resulting file descriptor.
    fn open(&self, inode: &crate::Inode) -> Result<alloc::boxed::Box<dyn crate::FileOperations>, Errno>;
}
