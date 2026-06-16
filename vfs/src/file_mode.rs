/// Unix file mode / permission bits matching Linux definitions.
///
/// Combines file type (`S_IFMT`) and permission bits in a single `u32`,
/// exactly as stored in `struct stat.st_mode`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct FileMode(u32);

impl FileMode {
    // --- Permission bits ---
    pub const S_IXOTH: Self = Self(0o001);
    pub const S_IWOTH: Self = Self(0o002);
    pub const S_IROTH: Self = Self(0o004);
    pub const S_IXGRP: Self = Self(0o010);
    pub const S_IWGRP: Self = Self(0o020);
    pub const S_IRGRP: Self = Self(0o040);
    pub const S_IXUSR: Self = Self(0o100);
    pub const S_IWUSR: Self = Self(0o200);
    pub const S_IRUSR: Self = Self(0o400);

    // --- Sticky / setuid / setgid ---
    pub const S_ISVTX: Self = Self(0o1000);
    pub const S_ISGID: Self = Self(0o2000);
    pub const S_ISUID: Self = Self(0o4000);

    // --- File type mask and values ---
    pub const S_IFMT: Self = Self(0o170000);
    pub const S_IFSOCK: Self = Self(0o140000);
    pub const S_IFLNK: Self = Self(0o120000);
    pub const S_IFREG: Self = Self(0o100000);
    pub const S_IFBLK: Self = Self(0o060000);
    pub const S_IFDIR: Self = Self(0o040000);
    pub const S_IFCHR: Self = Self(0o020000);
    pub const S_IFIFO: Self = Self(0o010000);

    /// Full permission set for user/group/other (0777).
    pub const PERM_MASK: Self = Self(0o777);

    /// Default directory creation mode: `rwxr-xr-x` (0755).
    pub const DIR_DEFAULT: Self =
        Self(Self::S_IFDIR.0 | 0o755);

    /// Default regular file creation mode: `rw-r--r--` (0644).
    pub const FILE_DEFAULT: Self =
        Self(Self::S_IFREG.0 | 0o644);

    // --- Constructors ---

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn regular(perm: u32) -> Self {
        Self(Self::S_IFREG.0 | (perm & Self::PERM_MASK.0))
    }

    pub const fn directory(perm: u32) -> Self {
        Self(Self::S_IFDIR.0 | (perm & Self::PERM_MASK.0))
    }

    pub const fn symlink(perm: u32) -> Self {
        Self(Self::S_IFLNK.0 | (perm & Self::PERM_MASK.0))
    }

    pub const fn char_device(perm: u32) -> Self {
        Self(Self::S_IFCHR.0 | (perm & Self::PERM_MASK.0))
    }

    pub const fn block_device(perm: u32) -> Self {
        Self(Self::S_IFBLK.0 | (perm & Self::PERM_MASK.0))
    }

    /// Return the raw `u32` value (e.g. for `Stat.st_mode`).
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Return the file type portion (S_IFMT bits).
    pub const fn file_type_bits(self) -> u32 {
        self.0 & Self::S_IFMT.0
    }

    /// Return the permission bits (0777).
    pub const fn permissions(self) -> u32 {
        self.0 & Self::PERM_MASK.0
    }

    // --- Predicates ---

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_regular(self) -> bool {
        self.file_type_bits() == Self::S_IFREG.0
    }

    pub const fn is_directory(self) -> bool {
        self.file_type_bits() == Self::S_IFDIR.0
    }

    pub const fn is_symlink(self) -> bool {
        self.file_type_bits() == Self::S_IFLNK.0
    }

    pub const fn is_char_device(self) -> bool {
        self.file_type_bits() == Self::S_IFCHR.0
    }

    pub const fn is_block_device(self) -> bool {
        self.file_type_bits() == Self::S_IFBLK.0
    }

    pub const fn is_fifo(self) -> bool {
        self.file_type_bits() == Self::S_IFIFO.0
    }

    pub const fn is_socket(self) -> bool {
        self.file_type_bits() == Self::S_IFSOCK.0
    }

    pub const fn is_readable_by_owner(self) -> bool {
        self.contains(Self::S_IRUSR)
    }

    pub const fn is_writable_by_owner(self) -> bool {
        self.contains(Self::S_IWUSR)
    }

    pub const fn is_executable_by_owner(self) -> bool {
        self.contains(Self::S_IXUSR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_file_mode() {
        let mode = FileMode::FILE_DEFAULT;
        assert!(mode.is_regular());
        assert!(!mode.is_directory());
        assert_eq!(mode.permissions(), 0o644);
    }

    #[test]
    fn directory_mode() {
        let mode = FileMode::DIR_DEFAULT;
        assert!(mode.is_directory());
        assert!(!mode.is_regular());
        assert_eq!(mode.permissions(), 0o755);
    }

    #[test]
    fn permission_bits() {
        let mode = FileMode::regular(0o600);
        assert!(mode.is_readable_by_owner());
        assert!(mode.is_writable_by_owner());
        assert!(!mode.is_executable_by_owner());
    }
}
