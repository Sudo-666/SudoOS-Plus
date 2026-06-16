/// File open flags matching Linux `O_*` constants.
///
/// These are the flags passed to `openat()` and stored in `File.f_flags`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct OpenFlags(u32);

impl OpenFlags {
    // --- Access mode (bottom 2 bits) ---
    pub const O_RDONLY: Self = Self(0o0);
    pub const O_WRONLY: Self = Self(0o1);
    pub const O_RDWR: Self = Self(0o2);
    pub const O_ACCMODE: Self = Self(0o3);

    // --- Creation flags ---
    pub const O_CREAT: Self = Self(0o100);
    pub const O_EXCL: Self = Self(0o200);
    pub const O_NOCTTY: Self = Self(0o400);
    pub const O_TRUNC: Self = Self(0o1000);

    // --- Status flags ---
    pub const O_APPEND: Self = Self(0o2000);
    pub const O_NONBLOCK: Self = Self(0o4000);
    pub const O_DSYNC: Self = Self(0o10000);
    pub const O_SYNC: Self = Self(0o4010000);

    // --- Additional flags ---
    pub const O_DIRECTORY: Self = Self(0o200000);
    pub const O_NOFOLLOW: Self = Self(0o400000);
    pub const O_CLOEXEC: Self = Self(0o2000000);

    // --- Constructors ---

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw `u32` value.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Extract the access mode bits (O_RDONLY / O_WRONLY / O_RDWR).
    pub const fn access_mode(self) -> AccessMode {
        match self.0 & Self::O_ACCMODE.0 {
            0 => AccessMode::ReadOnly,
            1 => AccessMode::WriteOnly,
            _ => AccessMode::ReadWrite,
        }
    }

    // --- Predicates ---

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_create(self) -> bool {
        self.contains(Self::O_CREAT)
    }

    pub const fn is_exclusive(self) -> bool {
        self.contains(Self::O_EXCL)
    }

    pub const fn is_truncate(self) -> bool {
        self.contains(Self::O_TRUNC)
    }

    pub const fn is_append(self) -> bool {
        self.contains(Self::O_APPEND)
    }

    pub const fn is_directory(self) -> bool {
        self.contains(Self::O_DIRECTORY)
    }

    pub const fn is_cloexec(self) -> bool {
        self.contains(Self::O_CLOEXEC)
    }
}

/// Access mode extracted from `OpenFlags`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl AccessMode {
    /// Whether this mode permits reading.
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    /// Whether this mode permits writing.
    pub const fn is_writable(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_mode_inference() {
        assert_eq!(OpenFlags::O_RDONLY.access_mode(), AccessMode::ReadOnly);
        assert_eq!(OpenFlags::O_WRONLY.access_mode(), AccessMode::WriteOnly);
        assert_eq!(OpenFlags::O_RDWR.access_mode(), AccessMode::ReadWrite);
    }

    #[test]
    fn flag_combination() {
        let flags = OpenFlags::O_RDWR.union(OpenFlags::O_CREAT).union(OpenFlags::O_TRUNC);
        assert!(flags.is_create());
        assert!(flags.is_truncate());
        assert!(!flags.is_append());
        assert_eq!(flags.access_mode(), AccessMode::ReadWrite);
    }
}
