/// Device number (`dev_t`) — split into major and minor.
///
/// Major identifies the device driver; minor identifies the specific device
/// instance.  On Linux x86_64, 12 bits are used for major and 20 for minor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dev {
    major: u16,
    minor: u16,
}

impl Dev {
    /// Construct a new device number.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Encode as a single `u64` suitable for `Stat.st_rdev`.
    pub const fn to_u64(self) -> u64 {
        ((self.major as u64) << 20) | (self.minor as u64)
    }

    pub const fn major(self) -> u16 {
        self.major
    }

    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// Reserved device numbers matching Linux conventions.
///
/// These are the `major` numbers assigned to well-known device classes.
pub const DEV_MAJOR_MEM: u16 = 1;

/// `/dev/null` — data sink, reads return EOF.
pub const DEV_NULL: Dev = Dev::new(DEV_MAJOR_MEM, 3);
/// `/dev/zero` — infinite zero bytes.
pub const DEV_ZERO: Dev = Dev::new(DEV_MAJOR_MEM, 5);
/// `/dev/console` — system console.
pub const DEV_CONSOLE: Dev = Dev::new(5, 1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_encode() {
        let d = Dev::new(1, 3);
        assert_eq!(d.major(), 1);
        assert_eq!(d.minor(), 3);
        // major << 20 | minor
        assert_eq!(d.to_u64(), (1u64 << 20) | 3);
    }

    #[test]
    fn dev_null_values() {
        assert_eq!(DEV_NULL.major(), 1);
        assert_eq!(DEV_NULL.minor(), 3);
    }

    #[test]
    fn dev_zero_values() {
        assert_eq!(DEV_ZERO.major(), 1);
        assert_eq!(DEV_ZERO.minor(), 5);
    }

    #[test]
    fn dev_console_values() {
        assert_eq!(DEV_CONSOLE.major(), 5);
        assert_eq!(DEV_CONSOLE.minor(), 1);
    }
}

