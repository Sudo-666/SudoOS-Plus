use core::sync::atomic::{AtomicUsize, Ordering};

use myos_vfs::{Errno, MutableIoBuffer};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::WaitQueue;

const TTY_LOCK: LockClass = LockClass::new("tty.console", LockRank::Console, 1);
const TTY_BUFFER: usize = 256;

pub const TIOCGPGRP: usize = 0x540f;
pub const TIOCSPGRP: usize = 0x5410;
pub const TCGETS: usize = 0x5401;
pub const TCSETS: usize = 0x5402;
pub const TCSETSW: usize = 0x5403;
pub const TCSETSF: usize = 0x5404;
pub const TIOCGWINSZ: usize = 0x5413;
pub const TIOCSWINSZ: usize = 0x5414;
// termios2 ABI (2026 static glibc uses TCGETS2 for isatty()/tcgetattr())
pub const TCGETS2: usize = 0x802c_542a;
pub const TCSETS2: usize = 0x402c_542b;
pub const TCSETSW2: usize = 0x402c_542c;
pub const TCSETSF2: usize = 0x402c_542d;
// controlling terminal
pub const TIOCSCTTY: usize = 0x540e;
pub const TIOCNOTTY: usize = 0x5422;
pub const TIOCGSID: usize = 0x5429;

const NCCS: usize = 19;
const ECHO: u32 = 0x0000_0008;
const ICANON: u32 = 0x0000_0002;
const ISIG: u32 = 0x0000_0001;
const ICRNL: u32 = 0x0000_0100;
const OPOST: u32 = 0x0000_0001;
const CS8: u32 = 0x0000_0030;
const B38400: u32 = 0x0000_000f;

static CONSOLE_TTY: IrqSpinLock<TtyState> = IrqSpinLock::new_with_class(TtyState::new(), TTY_LOCK);
static TTY_READ_WAIT: WaitQueue = WaitQueue::new();

// Stage-4 Gate-C diagnostics (bounded so they never flood the console). These
// are shared TTY instrumentation, not board-specific: they only fire when a
// process actually blocks on the console or input is fed to it, so qemu_virt's
// SelfTest boot (no input path) never prints them.
const TTY_DIAG_LIMIT: usize = 8;
static TTY_BLOCK_LOG: AtomicUsize = AtomicUsize::new(0);
static TTY_RX_LOG: AtomicUsize = AtomicUsize::new(0);
static TTY_READ_LOG: AtomicUsize = AtomicUsize::new(0);
static TTY_IOCTL_LOG: AtomicUsize = AtomicUsize::new(0);

struct TtyState {
    buffer: [u8; TTY_BUFFER],
    head: usize,
    len: usize,
    canonical: bool,
    echo: bool,
    isig: bool,
    foreground_pgrp: isize,
    controlling_session: isize,
    rows: u16,
    cols: u16,
}

impl TtyState {
    const fn new() -> Self {
        Self {
            buffer: [0; TTY_BUFFER],
            head: 0,
            len: 0,
            canonical: true,
            echo: true,
            isig: true,
            foreground_pgrp: 0,
            controlling_session: 0,
            rows: 24,
            cols: 80,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.len == TTY_BUFFER {
            return;
        }
        let tail = (self.head + self.len) % TTY_BUFFER;
        self.buffer[tail] = byte;
        self.len += 1;
    }

    fn pop_into(&mut self, output: &mut MutableIoBuffer<'_>) -> usize {
        let mut copied = 0;
        while self.len > 0 && output.remaining() > 0 {
            let byte = self.buffer[self.head];
            self.head = (self.head + 1) % TTY_BUFFER;
            self.len -= 1;
            copied += output.push(&[byte]);
            if self.canonical && byte == b'\n' {
                break;
            }
        }
        copied
    }

    fn erase(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        true
    }

    /// Whether a read() would return right now. Canonical mode waits for a
    /// complete line (terminated by '\n'); non-canonical mode returns as soon
    /// as any byte is buffered.
    fn read_ready(&self) -> bool {
        if self.len == 0 {
            return false;
        }
        if !self.canonical {
            return true;
        }
        let mut index = self.head;
        for _ in 0..self.len {
            if self.buffer[index] == b'\n' {
                return true;
            }
            index = (index + 1) % TTY_BUFFER;
        }
        false
    }
}

/// Whether the current process is a member of the console's controlling
/// session (Linux semantics for opening /dev/tty).
pub fn has_controlling_tty() -> bool {
    let Some(thread) = crate::task::current_user_thread() else {
        return false;
    };
    let session = thread.process_arc().session();
    if session == 0 {
        return false;
    }
    CONSOLE_TTY.lock().controlling_session == session
}

pub fn initialize() {
    crate::println!("tty:");
    crate::println!("  console line discipline: canonical");
    crate::println!("  ioctl hooks            : termios/pgrp/winsize");
}

pub fn input_byte(byte: u8) {
    let mut tty = CONSOLE_TTY.lock();
    match byte {
        b'\r' | b'\n' => {
            tty.push(b'\n');
            if tty.echo {
                write_output(b"\r\n");
            }
            drop(tty);
            let woken = wake_readers();
            let index = TTY_RX_LOG.fetch_add(1, Ordering::Relaxed);
            if index < TTY_DIAG_LIMIT {
                crate::println!("TTY-RX: byte={byte:#04x} wake_count={woken}");
            }
        }
        0x08 | 0x7f if tty.canonical => {
            if tty.erase() && tty.echo {
                write_output(b"\x08 \x08");
            }
        }
        0x03 if tty.isig => {
            if tty.echo {
                write_output(b"^C\r\n");
            }
            let pgrp = tty.foreground_pgrp;
            drop(tty);
            if pgrp > 0 {
                let _ = crate::signal::send_signal(
                    crate::process::ProcessId::from_raw_for_kernel(pgrp as usize),
                    crate::signal::SIGINT,
                );
            }
        }
        byte => {
            tty.push(byte);
            if tty.echo {
                write_output(&[byte]);
            }
            // canonical mode delivers whole lines: plain bytes never wake a
            // reader, only the line terminator does.
            let wake = !tty.canonical;
            drop(tty);
            let woken = if wake { wake_readers() } else { 0 };
            let index = TTY_RX_LOG.fetch_add(1, Ordering::Relaxed);
            if index < TTY_DIAG_LIMIT {
                crate::println!("TTY-RX: byte={byte:#04x} wake_count={woken}");
            }
        }
    }
}

pub fn read_console(buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
    // Pre-check scheduler state WITHOUT holding the Console lock to avoid
    // lock order violation: Console (rank 80) -> Scheduler (rank 20) is invalid.
    let can_block =
        crate::task::scheduler_is_initialized() && crate::task::current_user_thread().is_some();

    loop {
        let mut tty = CONSOLE_TTY.lock();
        if tty.read_ready() {
            let n = tty.pop_into(buf);
            drop(tty);
            let index = TTY_READ_LOG.fetch_add(1, Ordering::Relaxed);
            if index < TTY_DIAG_LIMIT {
                let pid = current_process().map(|p| p.id().get()).unwrap_or(0);
                crate::println!("TTY-READ: pid={pid} got {n} byte(s)");
            }
            return Ok(n);
        }
        if !can_block {
            return Err(Errno::Eagain);
        }
        drop(tty);
        let index = TTY_BLOCK_LOG.fetch_add(1, Ordering::Relaxed);
        if index < TTY_DIAG_LIMIT {
            let pid = current_process().map(|p| p.id().get()).unwrap_or(0);
            crate::println!("TTY-READ: pid={pid} blocked");
        }
        let _ = crate::task::block_current_on_if_from_user_trap(&TTY_READ_WAIT, || {
            !CONSOLE_TTY.lock().read_ready()
        });
    }
}

pub fn input_ready() -> bool {
    CONSOLE_TTY.lock().read_ready()
}

pub fn write_console(bytes: &[u8]) -> usize {
    write_output(bytes);
    bytes.len()
}

pub fn ioctl(cmd: usize, arg: usize) -> Result<usize, Errno> {
    let result = ioctl_dispatch(cmd, arg);
    // Stage-4 Gate-C diagnostic: record the first few termios ioctls (with
    // result) so a non-interactive shell can be traced to a failing TCGETS /
    // TCSETS / TIOCGPGRP. Gated by the verbose flag (only armed on rdinit),
    // so qemu_virt SelfTest never prints these.
    if crate::user::oscomp_verbose_user_trace_active() {
        let trace_index = TTY_IOCTL_LOG.fetch_add(1, Ordering::Relaxed);
        if trace_index < TTY_DIAG_LIMIT {
            let pid = current_process().map(|p| p.id().get()).unwrap_or(0);
            match &result {
                Ok(value) => crate::println!("TTY-IOCTL: pid={pid} cmd={cmd:#04x} -> ok({value})"),
                Err(errno) => crate::println!(
                    "TTY-IOCTL: pid={pid} cmd={cmd:#04x} -> errno={}",
                    errno.to_isize(),
                ),
            }
        }
    }
    result
}

fn ioctl_dispatch(cmd: usize, arg: usize) -> Result<usize, Errno> {
    match cmd {
        TCGETS => {
            let tty = CONSOLE_TTY.lock();
            let termios = KernelTermios::from_tty(&tty);
            drop(tty);
            copy_to_user(arg, termios.as_bytes())?;
            Ok(0)
        }
        TCGETS2 => {
            let tty = CONSOLE_TTY.lock();
            let termios2 = KernelTermios2::from_tty(&tty);
            drop(tty);
            copy_to_user(arg, termios2.as_bytes())?;
            Ok(0)
        }
        TCSETS | TCSETSW | TCSETSF => {
            let process = current_process()?;
            let mut termios = KernelTermios::zeroed();
            process
                .mm()
                .copy_from_user(arg, termios.as_mut_bytes())
                .map_err(|_| Errno::Efault)?;
            let mut tty = CONSOLE_TTY.lock();
            tty.canonical = termios.lflag & ICANON != 0;
            tty.echo = termios.lflag & ECHO != 0;
            tty.isig = termios.lflag & ISIG != 0;
            Ok(0)
        }
        TCSETS2 | TCSETSW2 | TCSETSF2 => {
            let process = current_process()?;
            let mut termios2 = KernelTermios2::zeroed();
            process
                .mm()
                .copy_from_user(arg, termios2.as_mut_bytes())
                .map_err(|_| Errno::Efault)?;
            let mut tty = CONSOLE_TTY.lock();
            tty.canonical = termios2.lflag & ICANON != 0;
            tty.echo = termios2.lflag & ECHO != 0;
            tty.isig = termios2.lflag & ISIG != 0;
            Ok(0)
        }
        TIOCGPGRP => {
            let pgrp = CONSOLE_TTY.lock().foreground_pgrp;
            let bytes = (pgrp as i32).to_ne_bytes();
            copy_to_user(arg, &bytes)?;
            Ok(0)
        }
        TIOCSPGRP => {
            let mut bytes = [0_u8; core::mem::size_of::<i32>()];
            current_process()?
                .mm()
                .copy_from_user(arg, &mut bytes)
                .map_err(|_| Errno::Efault)?;
            CONSOLE_TTY.lock().foreground_pgrp = i32::from_ne_bytes(bytes) as isize;
            Ok(0)
        }
        TIOCGWINSZ => {
            let tty = CONSOLE_TTY.lock();
            let winsize = KernelWinsize {
                rows: tty.rows,
                cols: tty.cols,
                xpixels: 0,
                ypixels: 0,
            };
            drop(tty);
            copy_to_user(arg, winsize.as_bytes())?;
            Ok(0)
        }
        TIOCSWINSZ => {
            let mut winsize = KernelWinsize::zeroed();
            current_process()?
                .mm()
                .copy_from_user(arg, winsize.as_mut_bytes())
                .map_err(|_| Errno::Efault)?;
            let mut tty = CONSOLE_TTY.lock();
            if winsize.rows != 0 {
                tty.rows = winsize.rows;
            }
            if winsize.cols != 0 {
                tty.cols = winsize.cols;
            }
            Ok(0)
        }
        TIOCSCTTY => {
            let process = current_process()?;
            let session = process.session();
            // Only a session leader without a controlling terminal may claim it.
            if session == 0 || session != process.id().get() as isize {
                return Err(Errno::Eperm);
            }
            let pgrp = process.process_group();
            let mut tty = CONSOLE_TTY.lock();
            if tty.controlling_session != 0 {
                return Err(Errno::Ebusy);
            }
            tty.controlling_session = session;
            tty.foreground_pgrp = pgrp;
            drop(tty);
            if crate::user::oscomp_verbose_user_trace_active() {
                crate::println!(
                    "TTY-CTTY: pid={} sid={} pgrp={} acquired",
                    process.id().get(),
                    session,
                    pgrp,
                );
            }
            Ok(0)
        }
        TIOCNOTTY => {
            let process = current_process()?;
            let mut tty = CONSOLE_TTY.lock();
            if tty.controlling_session == process.id().get() as isize {
                // The session leader is releasing the terminal.
                tty.controlling_session = 0;
                tty.foreground_pgrp = 0;
            }
            Ok(0)
        }
        TIOCGSID => {
            let sid = CONSOLE_TTY.lock().controlling_session;
            if sid == 0 {
                return Err(Errno::Enotty);
            }
            copy_to_user(arg, &(sid as i32).to_ne_bytes())?;
            Ok(0)
        }
        _ => Err(Errno::Enotty),
    }
}

#[repr(C)]
struct KernelTermios {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; NCCS],
}

impl KernelTermios {
    const fn zeroed() -> Self {
        Self {
            iflag: 0,
            oflag: 0,
            cflag: 0,
            lflag: 0,
            line: 0,
            cc: [0; NCCS],
        }
    }

    fn from_tty(tty: &TtyState) -> Self {
        let mut termios = Self::zeroed();
        termios.iflag = ICRNL;
        termios.oflag = OPOST;
        termios.cflag = B38400 | CS8;
        if tty.isig {
            termios.lflag |= ISIG;
        }
        if tty.canonical {
            termios.lflag |= ICANON;
        }
        if tty.echo {
            termios.lflag |= ECHO;
        }
        termios.cc[0] = 3;
        termios.cc[4] = 4;
        termios.cc[5] = 0;
        termios.cc[6] = 1;
        termios
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: termios is a repr(C) POD ABI buffer copied immediately.
        unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(*self) as *const u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: termios is a repr(C) POD ABI buffer filled immediately.
        unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(*self) as *mut u8,
                core::mem::size_of::<Self>(),
            )
        }
    }
}

#[repr(C)]
struct KernelTermios2 {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; NCCS],
    ispeed: u32,
    ospeed: u32,
}

const _: () = assert!(core::mem::size_of::<KernelTermios2>() == 44);

impl KernelTermios2 {
    const fn zeroed() -> Self {
        Self {
            iflag: 0,
            oflag: 0,
            cflag: 0,
            lflag: 0,
            line: 0,
            cc: [0; NCCS],
            ispeed: 0,
            ospeed: 0,
        }
    }

    fn from_tty(tty: &TtyState) -> Self {
        let mut termios2 = Self::zeroed();
        termios2.iflag = ICRNL;
        termios2.oflag = OPOST;
        termios2.cflag = B38400 | CS8;
        if tty.isig {
            termios2.lflag |= ISIG;
        }
        if tty.canonical {
            termios2.lflag |= ICANON;
        }
        if tty.echo {
            termios2.lflag |= ECHO;
        }
        termios2.cc[0] = 3;
        termios2.cc[4] = 4;
        termios2.cc[5] = 0;
        termios2.cc[6] = 1;
        termios2.ispeed = 115200;
        termios2.ospeed = 115200;
        termios2
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: termios2 is a repr(C) POD ABI buffer copied immediately.
        unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(*self) as *const u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: termios2 is a repr(C) POD ABI buffer filled immediately.
        unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(*self) as *mut u8,
                core::mem::size_of::<Self>(),
            )
        }
    }
}

#[repr(C)]
struct KernelWinsize {
    rows: u16,
    cols: u16,
    xpixels: u16,
    ypixels: u16,
}

impl KernelWinsize {
    const fn zeroed() -> Self {
        Self {
            rows: 0,
            cols: 0,
            xpixels: 0,
            ypixels: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: winsize is a repr(C) POD ABI buffer copied immediately.
        unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!(*self) as *const u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: winsize is a repr(C) POD ABI buffer filled immediately.
        unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(*self) as *mut u8,
                core::mem::size_of::<Self>(),
            )
        }
    }
}

fn current_process() -> Result<alloc::sync::Arc<crate::process::Process>, Errno> {
    crate::task::current_user_thread()
        .ok_or(Errno::Einval)
        .map(|thread| thread.process_arc())
}

fn copy_to_user(address: usize, bytes: &[u8]) -> Result<(), Errno> {
    current_process()?
        .mm()
        .copy_to_user(address, bytes)
        .map_err(|_| Errno::Efault)
}

fn write_output(bytes: &[u8]) {
    // SUDOOS_FINAL_DIRECT_FIX_V1: the bytes still come from the real guest process.
    crate::console::write_bytes(bytes);
}

fn wake_readers() -> usize {
    if crate::task::scheduler_is_initialized() {
        TTY_READ_WAIT.wake_all()
    } else {
        0
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    input_byte(b'o');
    input_byte(b'k');
    input_byte(b'\n');
    let mut bytes = [0_u8; 8];
    let mut output = MutableIoBuffer::new(&mut bytes);
    assert_eq!(read_console(&mut output).expect("tty read failed"), 3);
    assert_eq!(output.filled_bytes(), b"ok\n");

    crate::println!("M13 TTY gate:");
    crate::println!("  canonical input      : verified");
    crate::println!("  console output       : verified");
}
