/// Linux-compatible error numbers used by all VFS operations.
///
/// Each variant corresponds to a standard Linux `errno` value. Syscall
/// handlers return the negated value (e.g. `-ENOENT` = -2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(isize)]
pub enum Errno {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENXIO = 6,
    E2BIG = 7,
    EBADF = 9,
    EAGAIN = 11,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    EBUSY = 16,
    EEXIST = 17,
    EXDEV = 18,
    ENODEV = 19,
    ENOTDIR = 20,
    EISDIR = 21,
    EINVAL = 22,
    ENFILE = 23,
    EMFILE = 24,
    ENOTTY = 25,
    ENOSPC = 28,
    ESPIPE = 29,
    EROFS = 30,
    EMLINK = 31,
    EPIPE = 32,
    ERANGE = 34,
    ENOSYS = 38,
    ENOTEMPTY = 39,
    ELOOP = 40,
    EOVERFLOW = 75,
}

impl Errno {
    /// Convert to the negated `isize` form used by Linux syscall ABI.
    ///
    /// `ENOENT` → -2, `EINVAL` → -22, etc.
    pub const fn to_isize(self) -> isize {
        -(self as isize)
    }

    /// Recover an `Errno` from a negated syscall return value.
    ///
    /// Returns `None` if the value does not match any known errno.
    pub const fn from_isize(value: isize) -> Option<Self> {
        if value >= 0 {
            return None;
        }
        match -value {
            1 => Some(Self::EPERM),
            2 => Some(Self::ENOENT),
            3 => Some(Self::ESRCH),
            4 => Some(Self::EINTR),
            5 => Some(Self::EIO),
            6 => Some(Self::ENXIO),
            7 => Some(Self::E2BIG),
            9 => Some(Self::EBADF),
            11 => Some(Self::EAGAIN),
            12 => Some(Self::ENOMEM),
            13 => Some(Self::EACCES),
            14 => Some(Self::EFAULT),
            16 => Some(Self::EBUSY),
            17 => Some(Self::EEXIST),
            18 => Some(Self::EXDEV),
            19 => Some(Self::ENODEV),
            20 => Some(Self::ENOTDIR),
            21 => Some(Self::EISDIR),
            22 => Some(Self::EINVAL),
            23 => Some(Self::ENFILE),
            24 => Some(Self::EMFILE),
            25 => Some(Self::ENOTTY),
            28 => Some(Self::ENOSPC),
            29 => Some(Self::ESPIPE),
            30 => Some(Self::EROFS),
            31 => Some(Self::EMLINK),
            32 => Some(Self::EPIPE),
            34 => Some(Self::ERANGE),
            38 => Some(Self::ENOSYS),
            39 => Some(Self::ENOTEMPTY),
            40 => Some(Self::ELOOP),
            75 => Some(Self::EOVERFLOW),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errno_round_trip() {
        let cases = [
            Errno::ENOENT,
            Errno::EINVAL,
            Errno::EBADF,
            Errno::ENOMEM,
            Errno::ENOSYS,
        ];

        for errno in cases {
            let isize_val = errno.to_isize();
            assert!(isize_val < 0, "{errno:?} should be negative");
            assert_eq!(Errno::from_isize(isize_val), Some(errno));
        }
    }

    #[test]
    fn non_errno_values_rejected() {
        assert_eq!(Errno::from_isize(0), None);
        assert_eq!(Errno::from_isize(1), None);
        assert_eq!(Errno::from_isize(-999), None);
    }
}
