pub mod devfs;
pub mod tmpfs;

/// Create a minimal dentry for testing / boot verification.
///
/// This is a convenience for filesystem verification before the full
/// VFS mount table and dentry cache are operational.
#[cfg(debug_assertions)]
pub fn make_test_dentry(
    name: &str,
    inode: myos_vfs::ArcInode,
    sb: myos_vfs::ArcSuperBlock,
) -> myos_vfs::DentryRef {
    myos_vfs::DentryRef::new(myos_vfs::Dentry::new_root(name, inode, sb))
}
