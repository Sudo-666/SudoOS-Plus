//! Linux 风格的 /dev/pts 伪终端支持 (PTY master/slave pairs)。
//!
//! 提供 PTY 对：master 端用作进程间通信控制端，slave 端模拟终端设备。
//! 对 `/dev/ptmx` 执行 open 会创建新的 PTY 对，返回 master fd；
//! slave 端出现在 `/dev/pts/<N>`。

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use myos_vfs::{
    Errno, File, FileMode, FileOperations, IoBuffer, MutableIoBuffer, OpenFlags, PollEvents, Stat,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::WaitQueue;

const PTY_LOCK: LockClass = LockClass::new("devpts.pty", LockRank::Vfs, 7);
const PTY_BUFFER: usize = 4096;

static NEXT_PTS_INDEX: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// PTY Slave
// ---------------------------------------------------------------------------

/// PTY 对共享的内部缓冲区。
struct PtyShared {
    /// 从 master 到 slave 的输入 (master write → slave read)
    input: IrqSpinLock<PtyRingBuffer>,
    /// 从 slave 到 master 的输出 (slave write → master read)
    output: IrqSpinLock<PtyRingBuffer>,
    /// master 端是否已关闭
    master_closed: AtomicBool,
    /// slave 端是否已关闭
    slave_closed: AtomicBool,
    /// 等待输入可读
    input_wait: WaitQueue,
    /// 等待输出可写
    output_wait: WaitQueue,
}

struct PtyRingBuffer {
    buffer: [u8; PTY_BUFFER],
    head: usize,
    len: usize,
}

impl PtyRingBuffer {
    const fn new() -> Self {
        Self {
            buffer: [0; PTY_BUFFER],
            head: 0,
            len: 0,
        }
    }

    fn remaining(&self) -> usize {
        PTY_BUFFER - self.len
    }

    fn write(&mut self, data: &[u8]) -> usize {
        let mut copied = 0;
        while copied < data.len() && self.len < PTY_BUFFER {
            let tail = (self.head + self.len) % PTY_BUFFER;
            let available = if tail >= self.head {
                PTY_BUFFER - tail
            } else {
                self.head - tail
            };
            let available = available.min(PTY_BUFFER - self.len);
            let chunk = available.min(data.len() - copied);
            self.buffer[tail..tail + chunk].copy_from_slice(&data[copied..copied + chunk]);
            self.len += chunk;
            copied += chunk;
        }
        copied
    }

    fn read(&mut self, output: &mut MutableIoBuffer<'_>) -> usize {
        let mut copied = 0;
        while self.len > 0 && output.remaining() > 0 {
            let available = core::cmp::min(self.len, PTY_BUFFER - self.head);
            let chunk = core::cmp::min(available, output.remaining());
            let start = self.head;
            copied += output.push(&self.buffer[start..start + chunk]);
            self.head = (self.head + chunk) % PTY_BUFFER;
            self.len -= chunk;
        }
        copied
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn is_full(&self) -> bool {
        self.len == PTY_BUFFER
    }
}

// ---------------------------------------------------------------------------
// PTY Master
// ---------------------------------------------------------------------------

struct PtyMaster {
    shared: Arc<PtyShared>,
}

impl FileOperations for PtyMaster {
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        loop {
            let mut output = self.shared.output.lock();
            if !output.is_empty() {
                return Ok(output.read(buf));
            }
            if self.shared.slave_closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            if !crate::task::scheduler_is_initialized()
                || crate::task::current_user_thread().is_none()
            {
                return Err(Errno::Eagain);
            }
            drop(output);
            let _ =
                crate::task::block_current_on_if_from_user_trap(&self.shared.output_wait, || {
                    self.shared.output.lock().is_empty()
                        && !self.shared.slave_closed.load(Ordering::Acquire)
                });
            if crate::task::current_user_thread()
                .and_then(|t| t.forced_exit_status())
                .is_some()
            {
                return Err(Errno::Eintr);
            }
            // SUDOOS_SIGNAL_WAKE_BLOCKED_V1: surface interrupting signals as
            // EINTR instead of re-blocking, so the trap-return path can
            // deliver them.
            if crate::task::current_user_thread()
                .as_deref()
                .is_some_and(|t| {
                    crate::signal::has_interrupting_signal(&t.process(), t.blocked_signals())
                })
            {
                return Err(Errno::Eintr);
            }
        }
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        if self.shared.slave_closed.load(Ordering::Acquire) {
            return Err(Errno::Epipe);
        }
        let mut input = self.shared.input.lock();
        let written = input.write(buf.as_bytes());
        drop(input);
        safe_wake(&self.shared.input_wait);
        Ok(written)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.mode = FileMode::S_IFCHR | 0o620;
        stat.nlink = 1;
        Ok(stat)
    }

    fn poll(&self, _file: &File, requested: PollEvents) -> PollEvents {
        let output = self.shared.output.lock();
        let mut ready = PollEvents::empty();
        if !output.is_empty() || self.shared.slave_closed.load(Ordering::Acquire) {
            ready = ready.union(PollEvents::IN);
        }
        if self.shared.slave_closed.load(Ordering::Acquire) {
            ready = ready.union(PollEvents::HUP);
        }
        {
            let input = self.shared.input.lock();
            if !input.is_full() {
                ready = ready.union(PollEvents::OUT);
            }
        }
        ready.intersect(requested)
    }

    fn release(&self, _file: &File) {
        self.shared.master_closed.store(true, Ordering::Release);
        safe_wake(&self.shared.input_wait);
        safe_wake(&self.shared.output_wait);
    }

    fn ioctl(&self, _file: &File, _cmd: usize, _arg: usize) -> Result<usize, Errno> {
        // TIOCGPTN, TIOCSPTLCK 等可在此实现
        Err(Errno::Enotty)
    }
}

// ---------------------------------------------------------------------------
// PTY Slave
// ---------------------------------------------------------------------------

struct PtySlave {
    shared: Arc<PtyShared>,
    index: usize,
}

impl FileOperations for PtySlave {
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        loop {
            let mut input = self.shared.input.lock();
            if !input.is_empty() {
                return Ok(input.read(buf));
            }
            if self.shared.master_closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            if !crate::task::scheduler_is_initialized()
                || crate::task::current_user_thread().is_none()
            {
                return Err(Errno::Eagain);
            }
            drop(input);
            let _ =
                crate::task::block_current_on_if_from_user_trap(&self.shared.input_wait, || {
                    self.shared.input.lock().is_empty()
                        && !self.shared.master_closed.load(Ordering::Acquire)
                });
            if crate::task::current_user_thread()
                .and_then(|t| t.forced_exit_status())
                .is_some()
            {
                return Err(Errno::Eintr);
            }
            // SUDOOS_SIGNAL_WAKE_BLOCKED_V1: surface interrupting signals as
            // EINTR instead of re-blocking, so the trap-return path can
            // deliver them.
            if crate::task::current_user_thread()
                .as_deref()
                .is_some_and(|t| {
                    crate::signal::has_interrupting_signal(&t.process(), t.blocked_signals())
                })
            {
                return Err(Errno::Eintr);
            }
        }
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        if self.shared.master_closed.load(Ordering::Acquire) {
            return Err(Errno::Epipe);
        }
        let mut output = self.shared.output.lock();
        let written = output.write(buf.as_bytes());
        drop(output);
        safe_wake(&self.shared.output_wait);
        Ok(written)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.mode = FileMode::S_IFCHR | 0o620;
        stat.nlink = 1;
        Ok(stat)
    }

    fn poll(&self, _file: &File, requested: PollEvents) -> PollEvents {
        let input = self.shared.input.lock();
        let mut ready = PollEvents::empty();
        if !input.is_empty() || self.shared.master_closed.load(Ordering::Acquire) {
            ready = ready.union(PollEvents::IN);
        }
        if self.shared.master_closed.load(Ordering::Acquire) {
            ready = ready.union(PollEvents::HUP);
        }
        {
            let output = self.shared.output.lock();
            if !output.is_full() {
                ready = ready.union(PollEvents::OUT);
            }
        }
        ready.intersect(requested)
    }

    fn release(&self, _file: &File) {
        self.shared.slave_closed.store(true, Ordering::Release);
        safe_wake(&self.shared.input_wait);
        safe_wake(&self.shared.output_wait);
    }

    fn ioctl(&self, _file: &File, _cmd: usize, _arg: usize) -> Result<usize, Errno> {
        // TIOCGWINSZ, TCGETS, TCSETS 等可在 slave 端实现
        Err(Errno::Enotty)
    }
}

// ---------------------------------------------------------------------------
// Public API — 由 VFS /dev/ptmx 调用
// ---------------------------------------------------------------------------

/// 安全唤醒：仅在 scheduler 初始化后调用 wake_all。
fn safe_wake(wq: &WaitQueue) {
    if crate::task::scheduler_is_initialized() {
        wq.wake_all();
    }
}

/// 创建一个新的 PTY 对。
///
/// 返回 (master_file, slave_file, pts_index)。
pub fn create_pty_pair(
    flags: OpenFlags,
) -> Result<(myos_vfs::ArcFile, myos_vfs::ArcFile, usize), Errno> {
    let index = NEXT_PTS_INDEX.fetch_add(1, Ordering::Relaxed);
    let shared = Arc::new(PtyShared {
        input: IrqSpinLock::new_with_class(PtyRingBuffer::new(), PTY_LOCK),
        output: IrqSpinLock::new_with_class(PtyRingBuffer::new(), PTY_LOCK),
        master_closed: AtomicBool::new(false),
        slave_closed: AtomicBool::new(false),
        input_wait: WaitQueue::named("pts_in"),
        output_wait: WaitQueue::named("pts_out"),
    });

    let nonblock_flag = if flags.contains(OpenFlags::O_NONBLOCK) {
        OpenFlags::O_NONBLOCK
    } else {
        OpenFlags::empty()
    };
    let master_flags = OpenFlags::O_RDWR.union(nonblock_flag);
    let slave_flags = OpenFlags::O_RDWR.union(nonblock_flag);

    let master = File::new(
        master_flags,
        Arc::new(PtyMaster {
            shared: Arc::clone(&shared),
        }),
    );
    let slave = File::new(slave_flags, Arc::new(PtySlave { shared, index }));

    Ok((master, slave, index))
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
pub fn verify() {
    let (master, slave, index) =
        create_pty_pair(OpenFlags::empty()).expect("devpts create_pty_pair failed");

    // 写入 master → 从 slave 读取
    assert_eq!(
        master
            .write(&IoBuffer::new(b"pty-ok"))
            .expect("pty master write failed"),
        6,
    );
    let mut bytes = [0_u8; 8];
    let mut output = MutableIoBuffer::new(&mut bytes);
    assert_eq!(slave.read(&mut output).expect("pty slave read failed"), 6,);
    assert_eq!(output.filled_bytes(), b"pty-ok");

    // 写入 slave → 从 master 读取
    assert_eq!(
        slave
            .write(&IoBuffer::new(b"echo"))
            .expect("pty slave write failed"),
        4,
    );
    let mut bytes2 = [0_u8; 8];
    let mut output2 = MutableIoBuffer::new(&mut bytes2);
    assert_eq!(
        master.read(&mut output2).expect("pty master read failed"),
        4,
    );
    assert_eq!(output2.filled_bytes(), b"echo");

    // 关闭 master → slave 读到 EOF
    drop(master);
    let mut bytes3 = [0_u8; 4];
    let mut output3 = MutableIoBuffer::new(&mut bytes3);
    assert_eq!(
        slave.read(&mut output3).expect("pty slave EOF read failed"),
        0,
    );

    crate::println!("M16 devpts gate:");
    crate::println!("  PTY pair create     : verified");
    crate::println!("  master→slave I/O    : verified");
    crate::println!("  slave→master I/O    : verified");
    crate::println!("  master close → EOF  : verified");
    crate::println!("  pts index           : {index}");
}
