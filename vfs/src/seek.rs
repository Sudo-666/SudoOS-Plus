/// Seek offset reference, matching Linux `SEEK_SET` / `SEEK_CUR` / `SEEK_END`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeekWhence {
    /// Seek relative to the start of the file (`SEEK_SET` = 0).
    Set = 0,
    /// Seek relative to the current file position (`SEEK_CUR` = 1).
    Current = 1,
    /// Seek relative to the end of the file (`SEEK_END` = 2).
    End = 2,
}

impl SeekWhence {
    pub const fn from_usize(value: usize) -> Option<Self> {
        match value {
            0 => Some(Self::Set),
            1 => Some(Self::Current),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_valid_values() {
        assert_eq!(SeekWhence::from_usize(0), Some(SeekWhence::Set));
        assert_eq!(SeekWhence::from_usize(1), Some(SeekWhence::Current));
        assert_eq!(SeekWhence::from_usize(2), Some(SeekWhence::End));
    }

    #[test]
    fn from_invalid_rejected() {
        assert_eq!(SeekWhence::from_usize(3), None);
        assert_eq!(SeekWhence::from_usize(usize::MAX), None);
    }
}

