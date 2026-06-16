/// Immutable byte buffer passed to `FileOperations::write()`.
///
/// Provides a zero-copy view into the caller's data.  The kernel's syscall
/// layer constructs this from a checked `copy_from_user` buffer before
/// dispatching to the VFS.
#[derive(Clone, Copy)]
pub struct IoBuffer<'a> {
    data: &'a [u8],
}

impl<'a> IoBuffer<'a> {
    /// Create a new buffer from an existing byte slice.
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Return the number of bytes in the buffer.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Access the raw bytes.
    pub fn as_bytes(&self) -> &'a [u8] {
        self.data
    }
}

/// Mutable byte buffer passed to `FileOperations::read()`.
///
/// The VFS layer fills this buffer; the syscall layer then copies the result
/// back to user space via `copy_to_user`.
pub struct MutableIoBuffer<'a> {
    data: &'a mut [u8],
    filled: usize,
}

impl<'a> MutableIoBuffer<'a> {
    /// Create a new buffer from an existing mutable byte slice.
    pub const fn new(data: &'a mut [u8]) -> Self {
        Self { data, filled: 0 }
    }

    /// Total capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Number of bytes that have been written so far.
    pub fn len(&self) -> usize {
        self.filled
    }

    /// Check whether nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.filled == 0
    }

    /// Remaining writable space.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.filled
    }

    /// Append bytes to the buffer.
    ///
    /// Returns the number of bytes actually copied (capped at remaining space).
    pub fn push(&mut self, bytes: &[u8]) -> usize {
        let n = bytes.len().min(self.remaining());
        self.data[self.filled..self.filled + n].copy_from_slice(&bytes[..n]);
        self.filled += n;
        n
    }

    /// Append as many bytes as fit; return how many were written.
    ///
    /// Identical to `push()`, used for clarity in `read()` implementations.
    pub fn fill(&mut self, bytes: &[u8]) -> usize {
        self.push(bytes)
    }

    /// Access the filled portion as an immutable slice.
    pub fn filled_bytes(&self) -> &[u8] {
        &self.data[..self.filled]
    }

    /// Access the entire mutable slice (including unfilled space).
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_buffer_len() {
        let buf = IoBuffer::new(b"hello");
        assert_eq!(buf.len(), 5);
        assert!(!buf.is_empty());
        assert_eq!(buf.as_bytes(), b"hello");
    }

    #[test]
    fn io_buffer_empty() {
        let buf = IoBuffer::new(b"");
        assert!(buf.is_empty());
    }

    #[test]
    fn mutable_io_buffer_push() {
        let mut backing = [0u8; 8];
        let mut buf = MutableIoBuffer::new(&mut backing);

        let n = buf.push(b"test");
        assert_eq!(n, 4);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.remaining(), 4);

        // Fill remaining space
        let n = buf.push(b"more!!");
        assert_eq!(n, 4);
        assert_eq!(buf.len(), 8);
        assert_eq!(buf.remaining(), 0);

        // Overflow is capped
        let n = buf.push(b"x");
        assert_eq!(n, 0);
    }

    #[test]
    fn mutable_io_buffer_filled_bytes() {
        let mut backing = [0u8; 16];
        let mut buf = MutableIoBuffer::new(&mut backing);
        buf.push(b"world");
        assert_eq!(buf.filled_bytes(), b"world");
    }

    #[test]
    fn mutable_io_buffer_empty_initial() {
        let mut backing = [0u8; 4];
        let buf = MutableIoBuffer::new(&mut backing);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.remaining(), 4);
    }
}
