use crate::FileType;

/// A directory entry returned by `getdents64()`.
///
/// Layout follows Linux `struct dirent64`:
/// - `d_ino`: 64-bit inode number
/// - `d_off`: 64-bit offset to next entry
/// - `d_reclen`: 16-bit length of this record
/// - `d_type`: 8-bit file type (`DT_REG`, `DT_DIR`, etc.)
/// - `d_name`: null-terminated name (length implied by `d_reclen`)
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub d_ino: u64,
    pub d_off: u64,
    pub d_reclen: u16,
    d_type: u8,
    name: DirEntryName,
}

/// DT_* constants matching Linux `dirent.d_type` values.
impl DirEntry {
    pub const DT_UNKNOWN: u8 = 0;
    pub const DT_REG: u8 = 8;
    pub const DT_DIR: u8 = 4;
    pub const DT_CHR: u8 = 2;
    pub const DT_BLK: u8 = 6;
    pub const DT_FIFO: u8 = 1;
    pub const DT_SOCK: u8 = 12;
    pub const DT_LNK: u8 = 10;

    /// Maximum length of a directory entry name (excluding null terminator).
    pub const MAX_NAME_LEN: usize = 255;

    /// Create a new directory entry.
    pub fn new(d_ino: u64, d_off: u64, d_type: u8, name: &str) -> Option<Self> {
        let name = DirEntryName::from_str(name)?;
        let d_reclen = Self::reclen_for(name.len());
        Some(Self {
            d_ino,
            d_off,
            d_reclen,
            d_type,
            name,
        })
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn d_type(&self) -> u8 {
        self.d_type
    }

    /// Map a `FileType` to its `DT_*` constant.
    pub const fn d_type_from_file_type(ft: FileType) -> u8 {
        match ft {
            FileType::RegularFile => Self::DT_REG,
            FileType::Directory => Self::DT_DIR,
            FileType::CharacterDevice => Self::DT_CHR,
            FileType::BlockDevice => Self::DT_BLK,
            FileType::Fifo => Self::DT_FIFO,
            FileType::Socket => Self::DT_SOCK,
            FileType::SymbolicLink => Self::DT_LNK,
            FileType::Unknown => Self::DT_UNKNOWN,
        }
    }

    /// Serialise this entry into a user-space buffer in Linux `dirent64` layout.
    ///
    /// Returns the number of bytes written, or `None` if the buffer is too small.
    pub fn write_to(&self, buffer: &mut [u8]) -> Option<usize> {
        let name_bytes = self.name.as_str().as_bytes();
        let total = self.d_reclen as usize;

        if buffer.len() < total {
            return None;
        }

        let buf = &mut buffer[..total];

        // d_ino (8 bytes, offset 0)
        buf[0..8].copy_from_slice(&self.d_ino.to_ne_bytes());

        // d_off (8 bytes, offset 8)
        buf[8..16].copy_from_slice(&self.d_off.to_ne_bytes());

        // d_reclen (2 bytes, offset 16)
        buf[16..18].copy_from_slice(&self.d_reclen.to_ne_bytes());

        // d_type (1 byte, offset 18)
        buf[18] = self.d_type;

        // d_name (null-terminated, starting at offset 19)
        let name_end = 19 + name_bytes.len();
        buf[19..name_end].copy_from_slice(name_bytes);
        buf[name_end] = 0; // null terminator

        Some(total)
    }

    /// Compute `d_reclen` for a given name length.
    ///
    /// The record must be at least `19 + name_len + 1` bytes and 8-byte aligned.
    fn reclen_for(name_len: usize) -> u16 {
        let raw = 19u16 + name_len as u16 + 1; // header + name + null
        let aligned = (raw + 7) & !7u16; // align to 8 bytes
        aligned.max(24) // minimum dirent64 size on glibc
    }
}

/// A fixed-capacity name stored inside a `DirEntry`.
///
/// The maximum length is `DirEntry::MAX_NAME_LEN` (255).
#[derive(Clone, Debug)]
struct DirEntryName {
    bytes: [u8; DirEntry::MAX_NAME_LEN],
    len: u8,
}

impl DirEntryName {
    fn from_str(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.is_empty() || bytes.len() > DirEntry::MAX_NAME_LEN {
            return None;
        }

        let mut buf = [0u8; DirEntry::MAX_NAME_LEN];
        buf[..bytes.len()].copy_from_slice(bytes);

        Some(Self {
            bytes: buf,
            len: bytes.len() as u8,
        })
    }

    fn len(&self) -> usize {
        self.len as usize
    }

    fn as_str(&self) -> &str {
        // SAFETY: the bytes were validated as UTF-8 at construction time.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn dirent_round_trip() {
        let entry = DirEntry::new(42, 100, DirEntry::DT_REG, "test.txt").unwrap();
        assert_eq!(entry.d_ino, 42);
        assert_eq!(entry.d_off, 100);
        assert_eq!(entry.d_type(), DirEntry::DT_REG);
        assert_eq!(entry.name(), "test.txt");
    }

    #[test]
    fn write_to_buffer() {
        let entry = DirEntry::new(1, 0, DirEntry::DT_DIR, ".").unwrap();
        let reclen = entry.d_reclen as usize;
        let mut buf = vec![0u8; reclen];

        let written = entry.write_to(&mut buf).unwrap();
        assert_eq!(written, reclen);

        // Check inode at offset 0
        assert_eq!(u64::from_ne_bytes(buf[0..8].try_into().unwrap()), 1);
        // Check d_type at offset 18
        assert_eq!(buf[18], DirEntry::DT_DIR);
    }
}
