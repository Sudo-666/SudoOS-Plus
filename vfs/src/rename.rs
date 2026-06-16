/// Flags for the `renameat2()` / `renameat()` syscall.
///
/// `renameat()` uses `flags=0` (fail if target exists).
/// Linux 3.15 added `RENAME_NOREPLACE` and `RENAME_EXCHANGE`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct RenameFlags(u32);

impl RenameFlags {
    /// Default rename: fail with `EEXIST` if the target exists.
    pub const NONE: Self = Self(0);

    /// `RENAME_NOREPLACE`: don't overwrite the target; fail with `EEXIST`.
    pub const NOREPLACE: Self = Self(1);

    /// `RENAME_EXCHANGE`: atomically exchange source and target.
    pub const EXCHANGE: Self = Self(2);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub const fn is_noreplace(self) -> bool {
        self.0 & Self::NOREPLACE.0 != 0
    }

    pub const fn is_exchange(self) -> bool {
        self.0 & Self::EXCHANGE.0 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_has_no_flags() {
        assert!(!RenameFlags::NONE.is_noreplace());
        assert!(!RenameFlags::NONE.is_exchange());
    }

    #[test]
    fn noreplace_flag() {
        assert!(RenameFlags::NOREPLACE.is_noreplace());
        assert!(!RenameFlags::NOREPLACE.is_exchange());
    }

    #[test]
    fn exchange_flag() {
        assert!(RenameFlags::EXCHANGE.is_exchange());
        assert!(!RenameFlags::EXCHANGE.is_noreplace());
    }
}
