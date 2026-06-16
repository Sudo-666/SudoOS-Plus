use crate::{Errno, FileMode, StatFs};

/// Superblock operations vtable — analogous to Linux `struct super_operations`.
///
/// Each mounted filesystem provides one superblock that manages inode
/// allocation, destruction, and periodic writeback.
pub trait SuperBlockOperations: Send + 'static {
    /// Allocate a new inode with the given `mode`.
    ///
    /// The returned inode is initially unlinked (link count = 0) and must
    /// be linked into a directory before it becomes reachable.
    fn alloc_inode(
        &self,
        sb: &crate::SuperBlock,
        mode: FileMode,
    ) -> Result<crate::ArcInode, Errno>;

    /// Destroy an inode and free its resources.
    ///
    /// Called when the last reference to the inode is dropped *and* its
    /// link count has reached zero.
    fn destroy_inode(&self, inode: &crate::Inode);

    /// Write back dirty inode metadata (if any).
    ///
    /// For in-memory filesystems this is a no-op.
    fn write_inode(&self, _inode: &crate::Inode) -> Result<(), Errno> {
        Ok(())
    }

    /// Fill filesystem statistics (for `statfs` / `fstatfs`).
    fn statfs(&self, _sb: &crate::SuperBlock) -> Result<StatFs, Errno> {
        Ok(StatFs {
            f_type: 0,
            f_bsize: 4096,
            f_blocks: 0,
            f_bfree: 0,
            f_bavail: 0,
            f_files: 0,
            f_ffree: 0,
            f_fsid: [0; 2],
            f_namelen: 255,
            f_frsize: 4096,
            f_flags: 0,
            __spare: [0; 4],
        })
    }

    /// Called when the filesystem is unmounted.
    ///
    /// The default implementation is a no-op; filesystems that hold external
    /// resources should release them here.
    fn put_super(&self, _sb: &crate::SuperBlock) {}
}
