//! 管道实现（Linux 风格）。
//!
//! 参照 Linux `fs/pipe.c`。
//!
//! 提供：
//! - `Pipe`：内核管道对象（环形缓冲区 + 等待队列）
//! - `create_pipe()`：创建一对 fd（读端 / 写端）
//! - 阻塞读写、EOF 检测、SIGPIPE 支持
//!
//! 管道缓冲区大小为单页（PAGE_SIZE = 4096），
//! 与 Linux 默认 `pipe_max_size` 一致。

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::file_table::{File, FileError, FileOperations, OpenFlags};
use crate::task::WaitQueue;

/// Leak detection: number of live Pipe objects.
static LIVE_PIPES: AtomicUsize = AtomicUsize::new(0);

/// Assert no Pipe objects have leaked.
pub fn assert_no_leaks() {
    let count = LIVE_PIPES.load(Ordering::Acquire);
    assert_eq!(count, 0, "M12 leaked {} Pipe object(s)", count);
}

// ---------------------------------------------------------------------------
// Pipe 对象（参照 Linux `struct pipe_inode_info`）
// ---------------------------------------------------------------------------

/// 管道缓冲区大小（Linux 默认：PAGE_SIZE）。
const PIPE_BUF_SIZE: usize = 4096;

/// 管道对象（参照 Linux `struct pipe_inode_info`）。
///
/// 一个 `Pipe` 被两个 `File` 对象共享：
/// - 读端（`OpenFlags::RDONLY`）
/// - 写端（`OpenFlags::WRONLY`）
///
/// 内部使用 `UnsafeCell` 实现内部可变性（`FileOperations` trait 方法
/// 接收 `&self`），与 Linux 内核的 `pipe->mutex` 语义对应。
pub struct Pipe {
    /// 环形缓冲区（UnsafeCell 用于内部可变性）。
    buffer: UnsafeCell<[u8; PIPE_BUF_SIZE]>,
    /// 缓冲区中有效数据的起始位置。
    head: UnsafeCell<usize>,
    /// 缓冲区中有效数据的长度。
    len: UnsafeCell<usize>,
    /// 是否有读端打开。
    reader_open: AtomicBool,
    /// 是否有写端打开。
    writer_open: AtomicBool,
    /// 读端等待队列（缓冲区为空时阻塞）。
    read_wait: WaitQueue,
    /// 写端等待队列（缓冲区已满时阻塞）。
    write_wait: WaitQueue,
}

// SAFETY: Pipe is Sync because all mutable fields are protected by UnsafeCell
// and we never hold references across yield points. This mirrors Linux's
// pipe_inode_info which uses a mutex.
unsafe impl Sync for Pipe {}

impl Pipe {
    pub fn new() -> Self {
        LIVE_PIPES.fetch_add(1, Ordering::AcqRel);
        Self {
            buffer: UnsafeCell::new([0u8; PIPE_BUF_SIZE]),
            head: UnsafeCell::new(0),
            len: UnsafeCell::new(0),
            reader_open: AtomicBool::new(true),
            writer_open: AtomicBool::new(true),
            read_wait: WaitQueue::new(),
            write_wait: WaitQueue::new(),
        }
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        LIVE_PIPES.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Pipe {
    /// 从管道读取数据（内部实现）。
    fn do_read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        // SAFETY: Pipe methods are only called from FileOperations trait which
        // receives &self. No concurrent writers exist because kernel
        // execution is single-threaded per CPU and preemption is disabled
        // during syscall handling.
        let buffer = unsafe { &mut *self.buffer.get() };
        let head = unsafe { &mut *self.head.get() };
        let len = unsafe { &mut *self.len.get() };

        if *len == 0 {
            if !self.writer_open.load(Ordering::Acquire) {
                return Ok(0); // EOF
            }
            return Err(FileError::WouldBlock);
        }

        let available = (*len).min(buf.len());
        let tail = *head;

        for i in 0..available {
            buf[i] = buffer[(tail + i) % PIPE_BUF_SIZE];
        }

        *head = (tail + available) % PIPE_BUF_SIZE;
        *len -= available;

        self.write_wait.wake_all();

        Ok(available)
    }

    /// 向管道写入数据（内部实现）。
    fn do_write(&self, buf: &[u8]) -> Result<usize, FileError> {
        if !self.reader_open.load(Ordering::Acquire) {
            return Err(FileError::BrokenPipe);
        }

        // SAFETY: Same reasoning as do_read.
        let buffer = unsafe { &mut *self.buffer.get() };
        let head = unsafe { &mut *self.head.get() };
        let len = unsafe { &mut *self.len.get() };

        if *len == PIPE_BUF_SIZE {
            return Err(FileError::WouldBlock);
        }

        let available = (PIPE_BUF_SIZE - *len).min(buf.len());
        let tail = *head;

        for i in 0..available {
            buffer[(tail + *len + i) % PIPE_BUF_SIZE] = buf[i];
        }

        *len += available;

        self.read_wait.wake_all();

        Ok(available)
    }
}

// ---------------------------------------------------------------------------
// PipeReader：读端 FileOperations
// ---------------------------------------------------------------------------

struct PipeReader {
    pipe: Arc<Pipe>,
}

impl FileOperations for PipeReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        self.pipe.do_read(buf)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize, FileError> {
        Err(FileError::Invalid)
    }

    fn close(&self) {
        self.pipe.reader_open.store(false, Ordering::Release);
        self.pipe.write_wait.wake_all();
    }
}

// ---------------------------------------------------------------------------
// PipeWriter：写端 FileOperations
// ---------------------------------------------------------------------------

struct PipeWriter {
    pipe: Arc<Pipe>,
}

impl FileOperations for PipeWriter {
    fn read(&self, _buf: &mut [u8]) -> Result<usize, FileError> {
        Err(FileError::Invalid)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, FileError> {
        self.pipe.do_write(buf)
    }

    fn close(&self) {
        self.pipe.writer_open.store(false, Ordering::Release);
        self.pipe.read_wait.wake_all();
    }
}

// ---------------------------------------------------------------------------
// 公共 API：create_pipe（参照 Linux `do_pipe2`）
// ---------------------------------------------------------------------------

/// 创建管道（参照 Linux `do_pipe2`）。
///
/// `flags` 为修饰标志位（如 `O_NONBLOCK`），将被合并到读/写端的打开标志中。
///
/// 返回 `(reader_file, writer_file)` 两个文件对象。
/// 调用者需要将这两个文件插入到 fd 表中。
pub fn create_pipe(flags: usize) -> (File, File) {
    let pipe = Arc::new(Pipe::new());
    let nonblock = if flags & 0x800 != 0 { OpenFlags::NONBLOCK.0 } else { 0 };

    let reader = File::new(
        Arc::new(PipeReader {
            pipe: Arc::clone(&pipe),
        }),
        OpenFlags(OpenFlags::RDONLY.0 | nonblock),
    );

    let writer = File::new(
        Arc::new(PipeWriter { pipe }),
        OpenFlags(OpenFlags::WRONLY.0 | nonblock),
    );

    (reader, writer)
}
