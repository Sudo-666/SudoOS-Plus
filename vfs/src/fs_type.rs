use crate::Errno;

/// Registered filesystem type — analogous to Linux `struct file_system_type`.
///
/// Implementations are stateless singletons registered at boot time.
/// The `mount()` method creates a new superblock instance.
pub trait FileSystemType: Send + Sync + 'static {
    /// Human-readable filesystem name (e.g. `"tmpfs"`, `"devfs"`, `"ext4"`).
    fn name(&self) -> &'static str;

    /// Mount a new instance of this filesystem.
    ///
    /// `source` is the device path (optional, unused by tmpfs).
    /// `flags` are the mount flags (`MS_RDONLY`, `MS_NOSUID`, etc.).
    /// `data` is an optional filesystem-specific option string.
    ///
    /// Returns the root superblock on success.
    fn mount(
        &self,
        source: Option<&str>,
        flags: u64,
        data: Option<&str>,
    ) -> Result<crate::ArcSuperBlock, Errno>;
}

/// Mount flags matching Linux `MS_*` constants.
pub const MS_RDONLY: u64 = 1;
pub const MS_NOSUID: u64 = 2;
pub const MS_NODEV: u64 = 4;
pub const MS_NOEXEC: u64 = 8;
pub const MS_SYNCHRONOUS: u64 = 16;
pub const MS_REMOUNT: u64 = 32;
pub const MS_NOATIME: u64 = 1024;
pub const MS_BIND: u64 = 4096;
