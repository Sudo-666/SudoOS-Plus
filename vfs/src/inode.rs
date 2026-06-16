use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{FileMode, InodeId, InodeOperations};

/// In-memory inode — analogous to Linux `struct inode`.
///
/// Represents a filesystem object (file, directory, device, symlink, etc.).
/// Reference-counted via `i_count`; the filesystem is responsible for
/// managing the link count (`i_nlink`) separately.
pub struct Inode {
    /// Inode number — unique within a single filesystem.
    i_ino: InodeId,
    /// File mode (type + permissions).
    i_mode: FileMode,
    /// File size in bytes (regular files) or entry count (directories).
    i_size: AtomicU64,
    /// Number of hard links to this inode.
    i_nlink: AtomicU32,
    /// Reference count (file handles + dentry references).
    i_count: AtomicU32,
    /// Inode operations vtable.
    i_op: Option<alloc::boxed::Box<dyn InodeOperations>>,
    /// Filesystem-private data pointer — analogous to Linux `i_private`.
    i_private: *mut (),
    /// Back-reference to the owning superblock.
    i_sb: *const (),
}

impl Inode {
    /// Create a new inode with reference count 1 and link count 1 (for
    /// newly created files) or 2 (for directories, which have `..`).
    pub fn new(ino: InodeId, mode: FileMode, is_directory: bool) -> Self {
        let nlink = if is_directory { 2 } else { 1 };
        Self {
            i_ino: ino,
            i_mode: mode,
            i_size: AtomicU64::new(0),
            i_nlink: AtomicU32::new(nlink),
            i_count: AtomicU32::new(1),
            i_op: None,
            i_private: core::ptr::null_mut(),
            i_sb: core::ptr::null(),
        }
    }

    /// Create an inode with link count 0 (unlinked but still referenced).
    pub fn new_unlinked(ino: InodeId, mode: FileMode) -> Self {
        Self {
            i_ino: ino,
            i_mode: mode,
            i_size: AtomicU64::new(0),
            i_nlink: AtomicU32::new(0),
            i_count: AtomicU32::new(1),
            i_op: None,
            i_private: core::ptr::null_mut(),
            i_sb: core::ptr::null(),
        }
    }

    // --- i_sb accessors ---

    /// Get the superblock back-reference.
    pub fn i_sb(&self) -> *const () {
        self.i_sb
    }

    /// Set the superblock back-reference.
    pub fn set_i_sb(&mut self, sb: *const ()) {
        self.i_sb = sb;
    }

    // --- i_private accessors ---

    /// Get the filesystem-private data pointer.
    ///
    /// # Safety
    ///
    /// The caller must know the correct type to cast to.  The pointer is
    /// valid as long as the inode exists.
    pub fn i_private(&self) -> *mut () {
        self.i_private
    }

    /// Set the filesystem-private data pointer.
    pub fn set_i_private(&mut self, ptr: *mut ()) {
        self.i_private = ptr;
    }

    // --- Field accessors ---

    pub const fn i_ino(&self) -> InodeId {
        self.i_ino
    }

    pub const fn i_mode(&self) -> FileMode {
        self.i_mode
    }

    pub fn set_i_mode(&mut self, mode: FileMode) {
        self.i_mode = mode;
    }

    pub fn i_size(&self) -> u64 {
        self.i_size.load(Ordering::Acquire)
    }

    pub fn set_i_size(&self, size: u64) {
        self.i_size.store(size, Ordering::Release);
    }

    pub fn i_nlink(&self) -> u32 {
        self.i_nlink.load(Ordering::Acquire)
    }

    pub fn inc_nlink(&self) -> u32 {
        self.i_nlink.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn dec_nlink(&self) -> u32 {
        self.i_nlink.fetch_sub(1, Ordering::AcqRel) - 1
    }

    pub fn i_count(&self) -> u32 {
        self.i_count.load(Ordering::Acquire)
    }

    pub fn i_op(&self) -> Option<&dyn InodeOperations> {
        self.i_op.as_ref().map(|b| b.as_ref())
    }

    pub fn set_i_op(&mut self, op: alloc::boxed::Box<dyn InodeOperations>) {
        self.i_op = Some(op);
    }

    // --- Reference counting ---

    fn inc_ref(&self) -> u32 {
        self.i_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn dec_ref(&self) -> u32 {
        self.i_count.fetch_sub(1, Ordering::AcqRel) - 1
    }

    /// Whether the inode should be freed: no links and no references.
    pub fn should_free(&self) -> bool {
        self.i_nlink() == 0 && self.i_count() == 1
    }
}

/// Reference-counted handle to a heap-allocated `Inode`.
///
/// Cloning increments the reference count; dropping decrements it.
/// Unlike `ArcInode`, this type does NOT automatically free the inode
/// when the count reaches zero — use `destroy()` for explicit teardown
/// (which calls `destroy_inode` on the superblock and then frees the
/// allocation).
pub struct ArcInode {
    ptr: *const Inode,
}

#[allow(clippy::should_implement_trait)]
impl ArcInode {
    /// Allocate a new reference-counted inode on the heap.
    pub fn new(inode: Inode) -> Self {
        Self {
            ptr: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(inode)),
        }
    }

    pub fn as_ref(&self) -> &Inode {
        // SAFETY: pointer is valid as long as any ArcInode exists.
        unsafe { &*self.ptr }
    }

    pub fn as_mut(&mut self) -> &mut Inode {
        // SAFETY: &mut self guarantees exclusive access.
        unsafe { &mut *(self.ptr as *mut Inode) }
    }

    /// Destroy the underlying `Inode` and free its memory.
    ///
    /// The caller must ensure that `i_count` has reached 1 (this handle
    /// is the sole remaining reference) and that `i_nlink` is 0.
    ///
    /// # Safety
    ///
    /// After this call, all existing references to the inode are dangling.
    pub unsafe fn destroy(self, sb: &crate::SuperBlock) {
        sb.s_op().destroy_inode(self.as_ref());
        // SAFETY: caller guarantees exclusive ownership.
        unsafe { drop(alloc::boxed::Box::from_raw(self.ptr as *mut Inode)); }
        // Prevent Drop from running.
        core::mem::forget(self);
    }

    /// Inode pointer equality check (for `i_sb` lookups).
    pub fn ptr_eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.ptr, other.ptr)
    }
}

impl Clone for ArcInode {
    fn clone(&self) -> Self {
        self.as_ref().inc_ref();
        Self { ptr: self.ptr }
    }
}

impl Drop for ArcInode {
    fn drop(&mut self) {
        let inode = self.as_ref();
        let prev = inode.dec_ref();
        // When i_count reaches 0 and i_nlink is 0, the inode should be
        // freed — but free_inode() requires access to the superblock,
        // which we don't have here.  The filesystem's destroy_inode() is
        // called explicitly via ArcInode::destroy() instead.
        //
        // If i_count drops to 0 without destroy() being called, the
        // allocation leaks.  The safety comment on destroy() documents
        // this requirement.
        let _ = prev;
    }
}

// SAFETY: ArcInode's refcount is AtomicU32 — the underlying Inode is
// safe to access from any thread as long as references are managed correctly.
unsafe impl Send for ArcInode {}
// SAFETY: &ArcInode only provides &Inode (immutable access), all mutable
// state is behind AtomicU32/AtomicU64.
unsafe impl Sync for ArcInode {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inode_basic_fields() {
        let inode = Inode::new(InodeId::new(1), FileMode::FILE_DEFAULT, false);

        assert_eq!(inode.i_ino(), InodeId::new(1));
        assert!(inode.i_mode().is_regular());
        assert_eq!(inode.i_size(), 0);
        assert_eq!(inode.i_nlink(), 1);
        assert_eq!(inode.i_count(), 1);
    }

    #[test]
    fn directory_has_initial_nlink_2() {
        let inode = Inode::new(InodeId::new(2), FileMode::DIR_DEFAULT, true);
        assert_eq!(inode.i_nlink(), 2); // . and ..
    }

    #[test]
    fn arc_inode_refcount() {
        let arc = ArcInode::new(Inode::new(InodeId::new(10), FileMode::FILE_DEFAULT, false));
        assert_eq!(arc.as_ref().i_count(), 1);

        let clone = arc.clone();
        assert_eq!(arc.as_ref().i_count(), 2);

        drop(clone);
        assert_eq!(arc.as_ref().i_count(), 1);
    }
}
