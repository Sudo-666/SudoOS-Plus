/// Linux 64-bit `struct stat` matching the x86_64 ABI layout.
///
/// This struct is returned by `fstat()` / `newfstatat()` and filled from the
/// inode's metadata.  Field offsets and sizes match the Linux kernel's
/// `struct stat` for 64-bit architectures.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Stat {
    /// Device ID (filesystem identifier).
    pub st_dev: u64,
    /// Inode number.
    pub st_ino: u64,
    /// Hard-link count.
    pub st_nlink: u64,
    /// File mode (type + permissions).
    pub st_mode: u32,
    /// Owner user ID.
    pub st_uid: u32,
    /// Owner group ID.
    pub st_gid: u32,
    /// Padding.
    pub __pad0: u32,
    /// Device ID (for device special files).
    pub st_rdev: u64,
    /// File size in bytes.
    pub st_size: i64,
    /// Preferred I/O block size.
    pub st_blksize: i32,
    /// Padding.
    pub __pad1: i32,
    /// Number of 512-byte blocks allocated.
    pub st_blocks: i64,
    /// Access time: seconds.
    pub st_atime_sec: i64,
    /// Access time: nanoseconds.
    pub st_atime_nsec: i64,
    /// Modification time: seconds.
    pub st_mtime_sec: i64,
    /// Modification time: nanoseconds.
    pub st_mtime_nsec: i64,
    /// Change time: seconds.
    pub st_ctime_sec: i64,
    /// Change time: nanoseconds.
    pub st_ctime_nsec: i64,
    /// Reserved for future use.
    __unused: [i64; 3],
}

impl Stat {
    /// Construct a `Stat` with default (zeroed) timestamps and metadata.
    ///
    /// The caller must set at least `st_ino`, `st_mode`, `st_size`, and
    /// `st_blksize` to meaningful values.
    pub const fn zeroed() -> Self {
        Self {
            st_dev: 0,
            st_ino: 0,
            st_nlink: 1,
            st_mode: 0,
            st_uid: 0,
            st_gid: 0,
            __pad0: 0,
            st_rdev: 0,
            st_size: 0,
            st_blksize: 4096,
            __pad1: 0,
            st_blocks: 0,
            st_atime_sec: 0,
            st_atime_nsec: 0,
            st_mtime_sec: 0,
            st_mtime_nsec: 0,
            st_ctime_sec: 0,
            st_ctime_nsec: 0,
            __unused: [0; 3],
        }
    }

    /// Compute `st_blocks` from `st_size` (512-byte blocks, rounded up).
    pub fn blocks_from_size(size: u64) -> i64 {
        size.div_ceil(512) as i64
    }

    /// Return the parsed `FileType` from `st_mode`.
    pub fn file_type(self) -> crate::FileType {
        crate::FileType::from_mode_bits(self.st_mode)
    }
}

/// Filesystem statistics returned by `statfs()` / `fstatfs()`.
#[derive(Clone, Copy, Debug)]
pub struct StatFs {
    /// Filesystem type magic.
    pub f_type: u64,
    /// Optimal transfer block size.
    pub f_bsize: i64,
    /// Total data blocks.
    pub f_blocks: u64,
    /// Free blocks.
    pub f_bfree: u64,
    /// Free blocks for unprivileged users.
    pub f_bavail: u64,
    /// Total inodes.
    pub f_files: u64,
    /// Free inodes.
    pub f_ffree: u64,
    /// Filesystem ID.
    pub f_fsid: [i32; 2],
    /// Maximum filename length.
    pub f_namelen: i64,
    /// Fragment size.
    pub f_frsize: i64,
    /// Mount flags.
    pub f_flags: u64,
    /// Padding.
    pub __spare: [i64; 4],
}

/// Magic numbers for well-known filesystem types (`StatFs.f_type`).
pub const TMPFS_MAGIC: u64 = 0x01021994;
pub const DEVFS_MAGIC: u64 = 0x1373;
pub const EXT4_SUPER_MAGIC: u64 = 0xEF53;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_zeroed_defaults() {
        let s = Stat::zeroed();
        assert_eq!(s.st_ino, 0);
        assert_eq!(s.st_mode, 0);
        assert_eq!(s.st_size, 0);
        assert_eq!(s.st_blksize, 4096);
        assert_eq!(s.st_nlink, 1);
    }

    #[test]
    fn blocks_from_size_exact() {
        // 512 bytes = 1 block
        assert_eq!(Stat::blocks_from_size(512), 1);
    }

    #[test]
    fn blocks_from_size_round_up() {
        // 513 bytes = 2 blocks
        assert_eq!(Stat::blocks_from_size(513), 2);
    }

    #[test]
    fn blocks_from_size_zero() {
        assert_eq!(Stat::blocks_from_size(0), 0);
    }

    #[test]
    fn statfs_layout_sizes() {
        // Verify StatFs matches expected Linux layout size.
        // On x86_64, struct statfs64 is typically ~120 bytes.
        let s = StatFs {
            f_type: TMPFS_MAGIC,
            f_bsize: 4096,
            f_blocks: 1000,
            f_bfree: 500,
            f_bavail: 400,
            f_files: 100,
            f_ffree: 50,
            f_fsid: [0x1234, 0x5678],
            f_namelen: 255,
            f_frsize: 4096,
            f_flags: 0,
            __spare: [0; 4],
        };
        assert_eq!(s.f_type, TMPFS_MAGIC);
        assert_eq!(s.f_blocks, 1000);
        assert_eq!(s.f_namelen, 255);
    }
}

