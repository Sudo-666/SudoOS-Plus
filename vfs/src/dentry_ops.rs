/// Dentry operations vtable — analogous to Linux `struct dentry_operations`.
///
/// These are optional callbacks that customise dentry behaviour.  Most
/// filesystems can use the default (no-op) implementations.
pub trait DentryOperations: Send + Sync + 'static {
    /// Compare a dentry name against a lookup key.
    ///
    /// The default implementation does a byte-by-byte comparison.
    fn d_compare(&self, dentry: &crate::Dentry, name: &str) -> bool {
        dentry.d_name() == name
    }

    /// Called when a dentry's reference count drops to zero.
    ///
    /// The default is a no-op.
    fn d_release(&self, _dentry: &crate::Dentry) {}

    /// Called when a dentry loses its inode (e.g. during unlink).
    fn d_delete(&self, _dentry: &crate::Dentry) {}
}
