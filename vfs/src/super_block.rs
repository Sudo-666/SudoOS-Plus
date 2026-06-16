use core::sync::atomic::{AtomicU32, Ordering};

use crate::SuperBlockOperations;

/// A mounted filesystem instance — analogous to Linux `struct super_block`.
///
/// Owns the root dentry, filesystem-private data (`s_fs_info`), and
/// the superblock operations vtable.
pub struct SuperBlock {
    /// Filesystem root inode.
    root_inode: crate::ArcInode,
    /// Superblock operations vtable.
    s_op: alloc::boxed::Box<dyn SuperBlockOperations>,
    /// Reference count (active mounts + vfs use).
    s_count: AtomicU32,
    /// Filesystem-private data pointer — analogous to Linux `s_fs_info`.
    s_fs_info: *mut (),
}

impl SuperBlock {
    pub fn new(
        root_inode: crate::ArcInode,
        s_op: alloc::boxed::Box<dyn SuperBlockOperations>,
    ) -> Self {
        Self {
            root_inode,
            s_op,
            s_count: AtomicU32::new(1),
            s_fs_info: core::ptr::null_mut(),
        }
    }

    pub fn root_inode(&self) -> &crate::ArcInode {
        &self.root_inode
    }

    pub fn root_inode_mut(&mut self) -> &mut crate::ArcInode {
        &mut self.root_inode
    }

    pub fn s_op(&self) -> &dyn SuperBlockOperations {
        self.s_op.as_ref()
    }

    pub fn s_count(&self) -> u32 {
        self.s_count.load(Ordering::Acquire)
    }

    /// Get the filesystem-private data pointer.
    ///
    /// # Safety
    ///
    /// The caller must know the correct type to cast to.
    pub fn s_fs_info(&self) -> *mut () {
        self.s_fs_info
    }

    /// Set the filesystem-private data pointer.
    pub fn set_s_fs_info(&mut self, ptr: *mut ()) {
        self.s_fs_info = ptr;
    }

    fn inc_ref(&self) -> u32 {
        self.s_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn dec_ref(&self) -> u32 {
        self.s_count.fetch_sub(1, Ordering::AcqRel) - 1
    }
}

/// Reference-counted handle to a `SuperBlock`.
///
/// Cloning increments the reference count; dropping decrements it.
/// When the count reaches zero, `put_super()` is called and the
/// superblock is freed.
pub struct ArcSuperBlock {
    ptr: *const SuperBlock,
}

#[allow(clippy::should_implement_trait)]
impl ArcSuperBlock {
    /// Create a new reference-counted handle from a `SuperBlock`.
    pub fn new(sb: SuperBlock) -> Self {
        Self {
            ptr: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(sb)),
        }
    }

    pub fn as_ref(&self) -> &SuperBlock {
        // SAFETY: pointer is valid as long as any ArcSuperBlock exists.
        // Drop only frees when refcount reaches zero.
        unsafe { &*self.ptr }
    }

    #[allow(dead_code)]
    pub fn as_mut(&mut self) -> &mut SuperBlock {
        // SAFETY: &mut self guarantees exclusive access.
        unsafe { &mut *(self.ptr as *mut SuperBlock) }
    }
}

impl Clone for ArcSuperBlock {
    fn clone(&self) -> Self {
        self.as_ref().inc_ref();
        Self { ptr: self.ptr }
    }
}

impl Drop for ArcSuperBlock {
    fn drop(&mut self) {
        let sb = self.as_ref();
        if sb.dec_ref() == 0 {
            sb.s_op().put_super(sb);
            // SAFETY: last reference, no concurrent access.
            unsafe {
                drop(alloc::boxed::Box::from_raw(self.ptr as *mut SuperBlock));
            }
        }
    }
}

// SAFETY: ArcSuperBlock's refcount is AtomicU32 — the underlying
// SuperBlock is safe to access from any thread as long as references
// are managed correctly.
unsafe impl Send for ArcSuperBlock {}
// SAFETY: &ArcSuperBlock only provides &SuperBlock (immutable access),
// all mutable state is behind AtomicU32.
unsafe impl Sync for ArcSuperBlock {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::boxed::Box;
    use crate::{ArcInode, Errno, FileMode, Inode, InodeId, SuperBlockOperations};

    struct TestSbOps;

    impl SuperBlockOperations for TestSbOps {
        fn alloc_inode(
            &self,
            _sb: &SuperBlock,
            _mode: FileMode,
        ) -> Result<ArcInode, Errno> {
            unimplemented!()
        }
        fn destroy_inode(&self, _inode: &Inode) {}
    }

    fn make_test_sb() -> ArcSuperBlock {
        let root = ArcInode::new(Inode::new(InodeId::new(1), FileMode::DIR_DEFAULT, true));
        let ops: Box<dyn SuperBlockOperations> = Box::new(TestSbOps);
        ArcSuperBlock::new(SuperBlock::new(root, ops))
    }

    #[test]
    fn arc_sb_refcount() {
        let sb1 = make_test_sb();
        assert_eq!(sb1.as_ref().s_count(), 1);

        let sb2 = sb1.clone();
        assert_eq!(sb1.as_ref().s_count(), 2);

        drop(sb2);
        assert_eq!(sb1.as_ref().s_count(), 1);
    }

    #[test]
    fn sb_root_inode() {
        let sb = make_test_sb();
        assert_eq!(sb.as_ref().root_inode().as_ref().i_ino().get(), 1);
    }

    #[test]
    fn sb_fs_info_default_null() {
        let sb = make_test_sb();
        assert!(sb.as_ref().s_fs_info().is_null());
    }

    #[test]
    fn sb_set_fs_info() {
        let mut sb = make_test_sb();
        let data: u32 = 42;
        let ptr = &data as *const u32 as *mut ();
        sb.as_mut().set_s_fs_info(ptr);
        assert!(!sb.as_ref().s_fs_info().is_null());
        assert_eq!(unsafe { *(sb.as_ref().s_fs_info() as *const u32) }, 42);
    }
}

