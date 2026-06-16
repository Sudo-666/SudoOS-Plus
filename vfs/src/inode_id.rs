/// Opaque inode identifier, unique within a single filesystem.
///
/// Corresponds to the `st_ino` field in `struct stat`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(transparent)]
pub struct InodeId(u64);

impl InodeId {
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}
