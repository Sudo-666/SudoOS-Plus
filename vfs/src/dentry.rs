use core::sync::atomic::{AtomicU32, Ordering};

use crate::DentryOperations;

/// Fixed-size name stored inside a `Dentry`.
const DENTRY_NAME_MAX: usize = 255;

/// Directory entry cache — analogous to Linux `struct dentry`.
///
/// A dentry maps a name to an inode, forming the basis of pathname
/// resolution.  Dentries are reference-counted and may be cached.
pub struct Dentry {
    /// Entry name (not null-terminated unless stored that way).
    d_name: DentryName,
    /// The inode this entry points to (may be `None` for negative dentries).
    d_inode: Option<crate::ArcInode>,
    /// Parent dentry (root's parent is `None`).
    d_parent: Option<DentryRef>,
    /// Mounted superblock.
    d_sb: Option<crate::ArcSuperBlock>,
    /// Reference count.
    d_count: AtomicU32,
    /// Optional dentry operations.
    d_op: Option<&'static dyn DentryOperations>,
}

impl Dentry {
    /// Create a new dentry with reference count 1.
    pub fn new(
        name: &str,
        inode: Option<crate::ArcInode>,
        parent: Option<DentryRef>,
        sb: Option<crate::ArcSuperBlock>,
    ) -> Self {
        Self {
            d_name: DentryName::from_str(name).expect("dentry name too long or empty"),
            d_inode: inode,
            d_parent: parent,
            d_sb: sb,
            d_count: AtomicU32::new(1),
            d_op: None,
        }
    }

    /// Create the root dentry of a filesystem (no parent, has inode).
    pub fn new_root(name: &str, root_inode: crate::ArcInode, sb: crate::ArcSuperBlock) -> Self {
        Self::new(name, Some(root_inode), None, Some(sb))
    }

    pub fn d_name(&self) -> &str {
        self.d_name.as_str()
    }

    pub fn d_inode(&self) -> Option<&crate::ArcInode> {
        self.d_inode.as_ref()
    }

    pub fn set_d_inode(&mut self, inode: Option<crate::ArcInode>) {
        self.d_inode = inode;
    }

    pub fn d_parent(&self) -> Option<&DentryRef> {
        self.d_parent.as_ref()
    }

    pub fn d_sb(&self) -> Option<&crate::ArcSuperBlock> {
        self.d_sb.as_ref()
    }

    pub fn set_d_op(&mut self, op: &'static dyn DentryOperations) {
        self.d_op = Some(op);
    }

    pub fn d_count(&self) -> u32 {
        self.d_count.load(Ordering::Acquire)
    }

    fn inc_ref(&self) -> u32 {
        self.d_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn dec_ref(&self) -> u32 {
        self.d_count.fetch_sub(1, Ordering::AcqRel) - 1
    }
}

/// Reference-counted handle to a heap-allocated `Dentry`.
///
/// Cloning increments the reference count; dropping decrements it.
pub struct DentryRef {
    ptr: *const Dentry,
}

#[allow(clippy::should_implement_trait)]
impl DentryRef {
    /// Allocate a new reference-counted dentry on the heap.
    pub fn new(dentry: Dentry) -> Self {
        Self {
            ptr: alloc::boxed::Box::into_raw(alloc::boxed::Box::new(dentry)),
        }
    }

    pub fn as_ref(&self) -> &Dentry {
        // SAFETY: pointer is valid as long as any DentryRef exists.
        unsafe { &*self.ptr }
    }

    pub fn as_mut(&mut self) -> &mut Dentry {
        // SAFETY: &mut self guarantees exclusive access.
        unsafe { &mut *(self.ptr as *mut Dentry) }
    }
}

impl Clone for DentryRef {
    fn clone(&self) -> Self {
        self.as_ref().inc_ref();
        Self { ptr: self.ptr }
    }
}

impl Drop for DentryRef {
    fn drop(&mut self) {
        let dentry = self.as_ref();
        let prev = dentry.dec_ref();
        if prev == 0 {
            if let Some(op) = dentry.d_op {
                op.d_release(dentry);
            }
            // SAFETY: last reference, no concurrent access.
            unsafe {
                drop(alloc::boxed::Box::from_raw(self.ptr as *mut Dentry));
            }
        }
    }
}

// SAFETY: DentryRef's refcount is AtomicU32 — the underlying Dentry is
// safe to access from any thread as long as references are managed correctly.
unsafe impl Send for DentryRef {}
// SAFETY: &DentryRef only provides &Dentry (immutable access), all mutable
// state is behind AtomicU32.
unsafe impl Sync for DentryRef {}

/// Small fixed-capacity string for dentry names.
///
/// Names longer than `DENTRY_NAME_MAX` (255) bytes are rejected.
struct DentryName {
    bytes: [u8; DENTRY_NAME_MAX],
    len: u8,
}

impl DentryName {
    fn from_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.is_empty() || b.len() > DENTRY_NAME_MAX {
            return None;
        }

        let mut bytes = [0u8; DENTRY_NAME_MAX];
        bytes[..b.len()].copy_from_slice(b);

        Some(Self {
            bytes,
            len: b.len() as u8,
        })
    }

    fn as_str(&self) -> &str {
        // SAFETY: the bytes were validated as UTF-8 at construction time.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArcInode, FileMode, Inode, InodeId};

    #[test]
    fn dentry_name_round_trip() {
        let inode = ArcInode::new(Inode::new(InodeId::new(1), FileMode::DIR_DEFAULT, true));
        let dentry = DentryRef::new(Dentry::new("hello", Some(inode), None, None));

        assert_eq!(dentry.as_ref().d_name(), "hello");
        assert!(dentry.as_ref().d_inode().is_some());
        assert_eq!(dentry.as_ref().d_count(), 1);
    }

    #[test]
    fn dentry_clone_refcount() {
        let inode = ArcInode::new(Inode::new(InodeId::new(1), FileMode::DIR_DEFAULT, true));
        let d1 = DentryRef::new(Dentry::new("test", Some(inode), None, None));
        assert_eq!(d1.as_ref().d_count(), 1);

        let d2 = d1.clone();
        assert_eq!(d1.as_ref().d_count(), 2);

        drop(d2);
        assert_eq!(d1.as_ref().d_count(), 1);
    }

    #[test]
    fn negative_dentry() {
        // A negative dentry has no inode (the file does not exist).
        let dentry = DentryRef::new(Dentry::new("missing", None, None, None));
        assert!(dentry.as_ref().d_inode().is_none());
    }
}
