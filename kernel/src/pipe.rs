use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use myos_vfs::{
    Errno, File, FileOperations, IoBuffer, MutableIoBuffer, OpenFlags, PollEvents, Stat,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::WaitQueue;

const PIPE_LOCK: LockClass = LockClass::new("pipe.state", LockRank::WorkQueue, 3);
const PIPE_CAPACITY: usize = 4096;

struct Pipe {
    state: IrqSpinLock<PipeState>,
    read_wait: WaitQueue,
    write_wait: WaitQueue,
    read_epoch: AtomicU64,
    write_epoch: AtomicU64,
}

struct PipeState {
    buffer: [u8; PIPE_CAPACITY],
    head: usize,
    len: usize,
    readers: usize,
    writers: usize,
}

impl Pipe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: IrqSpinLock::new_with_class(
                PipeState {
                    buffer: [0; PIPE_CAPACITY],
                    head: 0,
                    len: 0,
                    readers: 1,
                    writers: 1,
                },
                PIPE_LOCK,
            ),
            read_wait: WaitQueue::new(),
            write_wait: WaitQueue::new(),
            read_epoch: AtomicU64::new(0),
            write_epoch: AtomicU64::new(0),
        })
    }

    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        loop {
            let mut state = self.state.lock();
            if state.len == 0 {
                if state.writers == 0 {
                    return Ok(0);
                }
                if file.flags().contains(OpenFlags::O_NONBLOCK)
                    || !crate::task::scheduler_is_initialized()
                    || crate::task::current_user_thread().is_none()
                {
                    return Err(Errno::Eagain);
                }
                let observed_epoch = self.read_epoch.load(Ordering::Acquire);
                drop(state);
                let outcome = self.read_wait.wait_interruptible_from_user_trap(|| {
                    self.read_epoch.load(Ordering::Acquire) != observed_epoch
                });
                if matches!(outcome, crate::task::InterruptibleWaitOutcome::Interrupted) {
                    // No bytes transferred yet (partial reads return above):
                    // an unblocked signal interrupts the read.
                    return Err(Errno::Eintr);
                }
                continue;
            }

            let mut copied = 0;
            while state.len > 0 && buf.remaining() > 0 {
                let available = core::cmp::min(state.len, PIPE_CAPACITY - state.head);
                let chunk = core::cmp::min(available, buf.remaining());
                let start = state.head;
                copied += buf.push(&state.buffer[start..start + chunk]);
                state.head = (state.head + chunk) % PIPE_CAPACITY;
                state.len -= chunk;
            }
            drop(state);
            self.wake_writers();
            return Ok(copied);
        }
    }

    fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        let input = buf.as_bytes();
        let mut copied = 0;
        while copied < input.len() {
            let mut state = self.state.lock();
            if state.readers == 0 {
                return if copied == 0 {
                    Err(Errno::Epipe)
                } else {
                    Ok(copied)
                };
            }
            if state.len == PIPE_CAPACITY {
                if copied != 0 {
                    return Ok(copied);
                }
                if file.flags().contains(OpenFlags::O_NONBLOCK)
                    || !crate::task::scheduler_is_initialized()
                    || crate::task::current_user_thread().is_none()
                {
                    return Err(Errno::Eagain);
                }
                let observed_epoch = self.write_epoch.load(Ordering::Acquire);
                drop(state);
                let outcome = self.write_wait.wait_interruptible_from_user_trap(|| {
                    self.write_epoch.load(Ordering::Acquire) != observed_epoch
                });
                if matches!(outcome, crate::task::InterruptibleWaitOutcome::Interrupted) {
                    // 0 bytes written (partial writes return above): an
                    // unblocked signal interrupts the write.
                    return Err(Errno::Eintr);
                }
                continue;
            }

            while copied < input.len() && state.len < PIPE_CAPACITY {
                let tail = (state.head + state.len) % PIPE_CAPACITY;
                let writable = if tail >= state.head {
                    PIPE_CAPACITY - tail
                } else {
                    state.head - tail
                };
                let writable = core::cmp::min(writable, PIPE_CAPACITY - state.len);
                let chunk = core::cmp::min(writable, input.len() - copied);
                state.buffer[tail..tail + chunk].copy_from_slice(&input[copied..copied + chunk]);
                state.len += chunk;
                copied += chunk;
            }
            drop(state);
            self.wake_readers();
        }
        Ok(copied)
    }

    fn close_reader(&self) {
        let mut state = self.state.lock();
        state.readers = state.readers.saturating_sub(1);
        drop(state);
        self.wake_writers();
    }

    fn exit_reader(&self) {
        let mut state = self.state.lock();
        state.readers = 0;
        drop(state);
        self.wake_writers();
    }

    fn close_writer(&self) {
        let mut state = self.state.lock();
        state.writers = state.writers.saturating_sub(1);
        drop(state);
        self.wake_readers();
    }

    fn exit_writer(&self) {
        let mut state = self.state.lock();
        state.writers = 0;
        drop(state);
        self.wake_readers();
    }

    fn reader_poll(&self, requested: PollEvents) -> PollEvents {
        let state = self.state.lock();
        let mut ready = PollEvents::empty();
        if state.len != 0 || state.writers == 0 {
            ready = ready.union(PollEvents::IN);
        }
        if state.writers == 0 {
            ready = ready.union(PollEvents::HUP);
        }
        ready.intersect(requested.union(PollEvents::HUP))
    }

    fn writer_poll(&self, requested: PollEvents) -> PollEvents {
        let state = self.state.lock();
        let mut ready = PollEvents::empty();
        if state.readers == 0 {
            ready = ready.union(PollEvents::ERR);
        } else if state.len < PIPE_CAPACITY {
            ready = ready.union(PollEvents::OUT);
        }
        ready.intersect(requested.union(PollEvents::ERR))
    }

    fn wake_readers(&self) {
        self.read_epoch.fetch_add(1, Ordering::Release);
        if crate::task::scheduler_is_initialized() {
            self.read_wait.wake_all();
        }
    }

    fn wake_writers(&self) {
        self.write_epoch.fetch_add(1, Ordering::Release);
        if crate::task::scheduler_is_initialized() {
            self.write_wait.wake_all();
        }
    }
}

struct PipeReader {
    pipe: Arc<Pipe>,
}

impl FileOperations for PipeReader {
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        self.pipe.read(_file, buf)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.mode = myos_vfs::FileMode::S_IFIFO | 0o600;
        stat.nlink = 1;
        Ok(stat)
    }

    fn release(&self, _file: &File) {
        // crate::println!("pipe-release: kind=reader pipe={:#x}", ...);
        self.pipe.close_reader();
    }

    fn process_exit(&self, _file: &File) {
        // A process exit removes only that process's descriptor reference.
        // fork/dup may leave the same Arc<File> alive elsewhere, so forcing
        // the whole pipe endpoint closed here would publish a false EOF.
        // The final Arc<File> drop calls release() and closes the endpoint.
    }

    fn poll(&self, _file: &File, requested: PollEvents) -> PollEvents {
        self.pipe.reader_poll(requested)
    }
}

struct PipeWriter {
    pipe: Arc<Pipe>,
}

impl FileOperations for PipeWriter {
    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        self.pipe.write(_file, buf)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.mode = myos_vfs::FileMode::S_IFIFO | 0o600;
        stat.nlink = 1;
        Ok(stat)
    }

    fn release(&self, _file: &File) {
        // crate::println!("pipe-release: kind=writer pipe={:#x}", ...);
        self.pipe.close_writer();
    }

    fn process_exit(&self, _file: &File) {
        // See PipeReader::process_exit. EOF belongs to open-file-description
        // lifetime, not to one process removing its inherited descriptor.
    }

    fn poll(&self, _file: &File, requested: PollEvents) -> PollEvents {
        self.pipe.writer_poll(requested)
    }
}

pub fn create_pipe(flags: OpenFlags) -> Result<(myos_vfs::ArcFile, myos_vfs::ArcFile), Errno> {
    let allowed = OpenFlags::O_CLOEXEC.bits() | OpenFlags::O_NONBLOCK.bits();
    if flags.bits() & !allowed != 0 {
        return Err(Errno::Einval);
    }
    let status_flags = if flags.contains(OpenFlags::O_NONBLOCK) {
        OpenFlags::O_NONBLOCK
    } else {
        OpenFlags::empty()
    };
    let pipe = Pipe::new();
    let reader = File::new(
        OpenFlags::O_RDONLY.union(status_flags),
        Arc::new(PipeReader {
            pipe: Arc::clone(&pipe),
        }),
    );
    let writer = File::new(
        OpenFlags::O_WRONLY.union(status_flags),
        Arc::new(PipeWriter { pipe }),
    );
    Ok((reader, writer))
}

#[cfg(debug_assertions)]
pub fn verify() {
    let (reader, writer) = create_pipe(OpenFlags::empty()).expect("pipe creation failed");
    assert_eq!(
        writer
            .write(&IoBuffer::new(b"pipe-ok"))
            .expect("pipe write failed"),
        7,
    );
    let mut bytes = [0_u8; 8];
    let mut output = MutableIoBuffer::new(&mut bytes);
    assert_eq!(reader.read(&mut output).expect("pipe read failed"), 7);
    assert_eq!(output.filled_bytes(), b"pipe-ok");
    drop(writer);
    let mut eof = MutableIoBuffer::new(&mut bytes[..1]);
    assert_eq!(reader.read(&mut eof).expect("pipe EOF failed"), 0);

    // fork/dup share an Arc<File>. One process exiting must not set the
    // endpoint count to zero while another process still owns that Arc.
    let (jobserver_reader, jobserver_writer) =
        create_pipe(OpenFlags::O_NONBLOCK).expect("jobserver pipe creation failed");
    jobserver_writer.process_exit();
    let mut token = [0_u8; 1];
    let mut token_output = MutableIoBuffer::new(&mut token);
    assert_eq!(
        jobserver_reader.read(&mut token_output),
        Err(Errno::Eagain),
        "jobserver inherited writer survived process-exit hook",
    );
    drop(jobserver_writer);
    let mut jobserver_eof = MutableIoBuffer::new(&mut token);
    assert_eq!(
        jobserver_reader
            .read(&mut jobserver_eof)
            .expect("jobserver final EOF failed"),
        0,
    );
    assert!(
        reader
            .poll(PollEvents::IN.union(PollEvents::HUP))
            .contains_any(PollEvents::IN.union(PollEvents::HUP)),
        "pipe reader did not report EOF readiness",
    );
    let (nonblock_reader, nonblock_writer) =
        create_pipe(OpenFlags::O_NONBLOCK).expect("nonblocking pipe creation failed");
    assert!(nonblock_reader.flags().contains(OpenFlags::O_NONBLOCK));
    assert!(nonblock_writer.flags().contains(OpenFlags::O_NONBLOCK));
    assert!(
        nonblock_writer
            .poll(PollEvents::OUT)
            .contains_any(PollEvents::OUT),
        "pipe writer did not report writable readiness",
    );

    crate::println!("M12 pipe gate:");
    crate::println!("  ring buffer          : verified");
    crate::println!("  EOF after writer drop: verified");
    crate::println!("  pipe2 status flags   : verified");
    crate::println!("  poll readiness       : verified");
}
