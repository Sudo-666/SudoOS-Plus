/// `fcntl()` command codes matching Linux definitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FcntlCmd {
    /// Duplicate a file descriptor (`F_DUPFD` = 0).
    DupFd(usize),
    /// Get close-on-exec flag (`F_GETFD` = 1).
    GetFd,
    /// Set close-on-exec flag (`F_SETFD` = 2).
    SetFd(bool),
    /// Get file status flags (`F_GETFL` = 3).
    GetFl,
    /// Set file status flags (`F_SETFL` = 4).
    SetFl(u32),
}

impl FcntlCmd {
    pub const F_DUPFD: u32 = 0;
    pub const F_GETFD: u32 = 1;
    pub const F_SETFD: u32 = 2;
    pub const F_GETFL: u32 = 3;
    pub const F_SETFL: u32 = 4;

    /// Decode a raw command value from the syscall.
    ///
    /// `arg` provides the third argument for commands that need one.
    pub const fn from_raw(cmd: u32, arg: usize) -> Option<Self> {
        match cmd {
            Self::F_DUPFD => Some(Self::DupFd(arg)),
            Self::F_GETFD => Some(Self::GetFd),
            Self::F_SETFD => Some(Self::SetFd(arg != 0)),
            Self::F_GETFL => Some(Self::GetFl),
            Self::F_SETFL => Some(Self::SetFl(arg as u32)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_dupfd() {
        assert_eq!(FcntlCmd::from_raw(0, 5), Some(FcntlCmd::DupFd(5)));
    }

    #[test]
    fn decode_getfd() {
        assert_eq!(FcntlCmd::from_raw(1, 0), Some(FcntlCmd::GetFd));
    }

    #[test]
    fn decode_setfd_true() {
        assert_eq!(FcntlCmd::from_raw(2, 1), Some(FcntlCmd::SetFd(true)));
    }

    #[test]
    fn decode_setfd_false() {
        assert_eq!(FcntlCmd::from_raw(2, 0), Some(FcntlCmd::SetFd(false)));
    }

    #[test]
    fn decode_getfl() {
        assert_eq!(FcntlCmd::from_raw(3, 0), Some(FcntlCmd::GetFl));
    }

    #[test]
    fn decode_setfl() {
        assert_eq!(FcntlCmd::from_raw(4, 0x8000), Some(FcntlCmd::SetFl(0x8000)));
    }

    #[test]
    fn decode_unknown() {
        assert_eq!(FcntlCmd::from_raw(99, 0), None);
    }
}
