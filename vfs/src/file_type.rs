/// File type classification matching Linux `S_IFMT` bits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileType {
    Unknown = 0,
    RegularFile = 1,
    Directory = 2,
    CharacterDevice = 3,
    BlockDevice = 4,
    Fifo = 5,
    Socket = 6,
    SymbolicLink = 7,
}

impl FileType {
    /// Direct construction from Linux `stat.st_mode & S_IFMT`.
    pub const fn from_mode_bits(mode: u32) -> Self {
        match mode & 0o170000 {
            0o100000 => Self::RegularFile,
            0o040000 => Self::Directory,
            0o020000 => Self::CharacterDevice,
            0o060000 => Self::BlockDevice,
            0o010000 => Self::Fifo,
            0o140000 => Self::Socket,
            0o120000 => Self::SymbolicLink,
            _ => Self::Unknown,
        }
    }

    pub const fn is_regular(self) -> bool {
        matches!(self, Self::RegularFile)
    }

    pub const fn is_directory(self) -> bool {
        matches!(self, Self::Directory)
    }

    pub const fn is_device(self) -> bool {
        matches!(self, Self::CharacterDevice | Self::BlockDevice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_mode_bits_classifies() {
        assert_eq!(FileType::from_mode_bits(0o100000), FileType::RegularFile);
        assert_eq!(FileType::from_mode_bits(0o040000), FileType::Directory);
        assert_eq!(FileType::from_mode_bits(0o020000), FileType::CharacterDevice);
        assert_eq!(FileType::from_mode_bits(0o060000), FileType::BlockDevice);
        // Perm bits are masked out by caller; we just test the IFMT portion
    }
}
