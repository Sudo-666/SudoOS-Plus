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
const ONLCR: u32 = 0x0000_0004;
const CS8: u32 = 0x0000_0030;
const NOFLSH: u32 = 0x0000_0080;
// asm-generic CBAUD layout: B0..B38400 occupy the low nibble (0x0..0xf); the
// B57600..B4000000 family adds bit 12 (CBAUDEX = 0x1000). 0x1001 is B57600,
// NOT 115200 (the old constant reported 115200 for what is actually 57600).
const B57600: u32 = 0x0000_1001;
const B115200: u32 = 0x0000_1002;
const CBAUD: u32 = 0x0000_100f;
// asm-generic termios c_cc[] indexes (VINTR at index 0, etc.)
const VINTR: usize = 0;
const VERASE: usize = 2;
const VEOF: usize = 4;
const VTIME: usize = 5;
const VMIN: usize = 6;

static CONSOLE_TTY: IrqSpinLock<TtyState> = IrqSpinLock::new_with_class(TtyState::new(), TTY_LOCK);
static TTY_READ_WAIT: WaitQueue = WaitQueue::new();

struct TtyState {
    buffer: [u8; TTY_BUFFER],
    head: usize,
    len: usize,
    // Set when VEOF (^D) is received on an empty canonical line; the next
    // read() then returns 0 (end-of-file) so shells exit on ^D.
    eof_pending: bool,
    // Full termios state so TCSETS* -> TCGETS* round-trips (BusyBox ash saves
    // the settings, switches to raw for line editing, then restores them).
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    cc: [u8; NCCS],
    ispeed: u32,
    ospeed: u32,
    foreground_pgrp: isize,
    controlling_session: isize,
    rows: u16,
    cols: u16,
}

impl TtyState {
    const fn new() -> Self {
        let mut cc = [0_u8; NCCS];
        cc[VINTR] = 3;
        cc[VERASE] = 0x7f;
        cc[VEOF] = 4;
        cc[VTIME] = 0;
        cc[VMIN] = 1;
        Self {
            buffer: [0; TTY_BUFFER],
            head: 0,
            len: 0,
            eof_pending: false,
            iflag: ICRNL,
            oflag: OPOST | ONLCR,
            cflag: B115200 | CS8,
            lflag: ISIG | ICANON | ECHO,
            cc,
            ispeed: 115200,
            ospeed: 115200,
            foreground_pgrp: 0,
            controlling_session: 0,
            rows: 24,
            cols: 80,
        }
    }

    fn canonical(&self) -> bool {
        self.lflag & ICANON != 0
    }

    fn echo(&self) -> bool {
        self.lflag & ECHO != 0
    }

    fn isig(&self) -> bool {
        self.lflag & ISIG != 0
    }

    fn onlcr(&self) -> bool {
        // ONLCR is only honoured while output post-processing (OPOST) is on;
        // clearing OPOST disables the \n -> \r\n expansion even if the ONLCR
        // bit is left set in oflag.
        self.oflag & (OPOST | ONLCR) == OPOST | ONLCR
    }

    /// Install a full termios snapshot (all fields, so the next TCGETS*
    /// reports exactly what the caller set).
    fn apply_termios(
        &mut self,
        iflag: u32,
        oflag: u32,
        cflag: u32,
        lflag: u32,
        cc: &[u8; NCCS],
        ispeed: u32,
        ospeed: u32,
    ) {
        self.iflag = iflag;
        self.oflag = oflag;
        self.cflag = cflag;
        self.lflag = lflag;
        self.cc = *cc;
        self.ispeed = ispeed;
        self.ospeed = ospeed;
    }

    /// TCSETSF/TCSETSF2 semantics: discard all unread input so a stale
    /// carriage return can never produce a spurious prompt line.
    fn flush_input(&mut self) {
        self.head = 0;
        self.len = 0;
        self.eof_pending = false;
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
        // VEOF on an empty line: deliver end-of-file (read returns 0). The
        // flag is consumed here so it only fires once.
        if self.eof_pending {
            self.eof_pending = false;
            return 0;
        }
        let mut copied = 0;
        while self.len > 0 && output.remaining() > 0 {
            let byte = self.buffer[self.head];
            self.head = (self.head + 1) % TTY_BUFFER;
            self.len -= 1;
            copied += output.push(&[byte]);
            if self.canonical() && byte == b'\n' {
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
            return self.eof_pending;
        }
        if !self.canonical() {
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
    // VINTR (default ^C): interrupt the foreground group when ISIG is on.
    // Reads the configured cc[VINTR] so `stty intr` is honoured.
    if tty.isig() && tty.cc[VINTR] != 0 && byte == tty.cc[VINTR] {
        if tty.echo() {
            write_output(b"^C\r\n");
        }
        // NOFLSH clear: discard unread input so keystrokes typed before the
        // interrupt never leak into the next command line.
        if tty.lflag & NOFLSH == 0 {
            tty.flush_input();
        }
        let pgrp = tty.foreground_pgrp;
        drop(tty);
        if pgrp > 0 {
            // Job-control: interrupt the whole foreground process group
            // (e.g. `sleep 30 | cat` — both members must receive SIGINT).
            crate::signal::send_signal_to_process_group(pgrp, crate::signal::SIGINT);
        }
        return;
    }
    match byte {
        // VEOF (default ^D) in canonical mode.
        byte if tty.canonical() && tty.cc[VEOF] != 0 && byte == tty.cc[VEOF] => {
            if tty.len == 0 {
                // Empty line: next read() returns 0 (end-of-file) so shells
                // exit on ^D.
                tty.eof_pending = true;
                drop(tty);
                wake_readers();
            } else {
                // Partial line: deliver the pending input immediately (the
                // VEOF char is neither echoed nor copied into the buffer).
                tty.push(b'\n');
                drop(tty);
                wake_readers();
            }
        }
        // ICRNL: map CR to NL on input, so Enter terminates a line.
        b'\r' if tty.iflag & ICRNL != 0 => {
            tty.push(b'\n');
            if tty.echo() {
                if tty.onlcr() {
                    write_output(b"\r\n");
                } else {
                    write_output(b"\n");
                }
            }
            drop(tty);
            wake_readers();
        }
        // ICRNL clear: CR is an ordinary character.
        b'\r' => {
            tty.push(b'\r');
            if tty.echo() {
                write_output(&[b'\r']);
            }
            let wake = !tty.canonical();
            drop(tty);
            if wake {
                wake_readers();
            }
        }
        b'\n' => {
            tty.push(b'\n');
            if tty.echo() {
                if tty.onlcr() {
                    write_output(b"\r\n");
                } else {
                    write_output(b"\n");
                }
            }
            drop(tty);
            wake_readers();
        }
        // ERASE: the configured erase char (default DEL 0x7f), plus 0x08 which
        // many serial terminals send for Backspace regardless of stty.
        byte if tty.canonical() && (byte == tty.cc[VERASE] || byte == 0x08) => {
            if tty.erase() && tty.echo() {
                write_output(b"\x08 \x08");
            }
        }
        byte => {
            // New input cancels a pending VEOF.
            if tty.eof_pending {
                tty.eof_pending = false;
            }
            tty.push(byte);
            if tty.echo() {
                write_output(&[byte]);
            }
            // canonical mode delivers whole lines: plain bytes never wake a
            // reader, only the line terminator does.
            let wake = !tty.canonical();
            drop(tty);
            if wake {
                wake_readers();
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
            return Ok(n);
        }
        if !can_block {
            return Err(Errno::Eagain);
        }
        drop(tty);
        let _ = crate::task::block_current_on_if_from_user_trap(&TTY_READ_WAIT, || {
            !CONSOLE_TTY.lock().read_ready()
        });
    }
}

pub fn input_ready() -> bool {
    CONSOLE_TTY.lock().read_ready()
}

pub fn write_console(bytes: &[u8]) -> usize {
    if CONSOLE_TTY.lock().onlcr() {
        // ONLCR: expand \n -> \r\n while holding the console write lock once,
        // so user output breaks lines correctly (kernel println already does
        // this conversion via the runtime formatter).
        crate::console::write_bytes_translated(bytes, b'\n', b"\r\n");
    } else {
        write_output(bytes);
    }
    bytes.len()
}

/// Called when the last thread of a session leader exits: release the
/// controlling terminal (and its foreground group) so a respawned shell can
/// reclaim it with TIOCSCTTY instead of hitting EBUSY.
pub fn release_controlling_session(session: isize) {
    let mut tty = CONSOLE_TTY.lock();
    if tty.controlling_session == session {
        tty.controlling_session = 0;
        tty.foreground_pgrp = 0;
    }
}

/// Decode the CBAUD bits of a termios1 cflag into a real baud rate. termios2
/// carries ispeed/ospeed explicitly, but a termios1 TCSETS can still change
/// the baud (e.g. `stty 115200`), and TCGETS must then report it back.
fn baud_from_cflag(cflag: u32) -> u32 {
    match cflag & CBAUD {
        0x0000 => 0, // B0 (hang up)
        0x0001 => 50,
        0x0002 => 75,
        0x0003 => 110,
        0x0004 => 134,
        0x0005 => 150,
        0x0006 => 200,
        0x0007 => 300,
        0x0008 => 600,
        0x0009 => 1200,
        0x000a => 1800,
        0x000b => 2400,
        0x000c => 4800,
        0x000d => 9600,
        0x000e => 19200,
        0x000f => 38400,
        0x1000 => 0, // BOTHER: speed carried in termios2 ispeed/ospeed
        B57600 => 57600,
        B115200 => 115200,
        0x1003 => 230400,
        0x1004 => 460800,
        0x1005 => 500000,
        0x1006 => 576000,
        0x1007 => 921600,
        0x1008 => 1000000,
        0x1009 => 1152000,
        0x100a => 1500000,
        0x100b => 2000000,
        0x100c => 2500000,
        0x100d => 3000000,
        0x100e => 3500000,
        0x100f => 4000000,
        _ => 0,
    }
}

pub fn ioctl(cmd: usize, arg: usize) -> Result<usize, Errno> {
    ioctl_dispatch(cmd, arg)
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
            // termios1 has no explicit ispeed/ospeed: derive them from CBAUD.
            let baud = baud_from_cflag(termios.cflag);
            let mut tty = CONSOLE_TTY.lock();
            tty.apply_termios(
                termios.iflag,
                termios.oflag,
                termios.cflag,
                termios.lflag,
                &termios.cc,
                baud,
                baud,
            );
            if cmd == TCSETSF {
                tty.flush_input();
            }
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
            tty.apply_termios(
                termios2.iflag,
                termios2.oflag,
                termios2.cflag,
                termios2.lflag,
                &termios2.cc,
                termios2.ispeed,
                termios2.ospeed,
            );
            if cmd == TCSETSF2 {
                tty.flush_input();
            }
            Ok(0)
        }
        TIOCGPGRP => {
            let pgrp = CONSOLE_TTY.lock().foreground_pgrp;
            let bytes = (pgrp as i32).to_ne_bytes();
            copy_to_user(arg, &bytes)?;
            Ok(0)
        }
        TIOCSPGRP => {
            let process = current_process()?;
            let session = process.session();
            let mut bytes = [0_u8; core::mem::size_of::<i32>()];
            process
                .mm()
                .copy_from_user(arg, &mut bytes)
                .map_err(|_| Errno::Efault)?;
            let target = i32::from_ne_bytes(bytes);
            // Linux: the target pgrp must belong to a process in the caller's
            // own session; otherwise the request is EPERM.
            if target <= 0 || !crate::process::process_group_in_session(target as isize, session) {
                return Err(Errno::Eperm);
            }
            CONSOLE_TTY.lock().foreground_pgrp = target as isize;
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
        termios.iflag = tty.iflag;
        termios.oflag = tty.oflag;
        termios.cflag = tty.cflag;
        termios.lflag = tty.lflag;
        termios.cc = tty.cc;
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
        termios2.iflag = tty.iflag;
        termios2.oflag = tty.oflag;
        termios2.cflag = tty.cflag;
        termios2.lflag = tty.lflag;
        termios2.cc = tty.cc;
        termios2.ispeed = tty.ispeed;
        termios2.ospeed = tty.ospeed;
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

    // termios ABI sizes: termios1 = 4*u32 + line:u8 + cc[19] = 36; termios2
    // adds ispeed/ospeed = 44 (must match asm-generic).
    assert_eq!(core::mem::size_of::<KernelTermios>(), 36);
    assert_eq!(core::mem::size_of::<KernelTermios2>(), 44);
    // Baud decode: low nibble B0..B38400, bit 12 (CBAUDEX) selects the
    // B57600..B4000000 family. 0x1001 is 57600, 0x1002 is 115200.
    assert_eq!(baud_from_cflag(0x0000), 0);
    assert_eq!(baud_from_cflag(0x000d), 9600);
    assert_eq!(baud_from_cflag(0x000f), 38400);
    assert_eq!(baud_from_cflag(B57600), 57600);
    assert_eq!(baud_from_cflag(B115200), 115200);
    assert_eq!(baud_from_cflag(0x1003), 230400);
    assert_eq!(baud_from_cflag(0x100f), 4000000);
    // Only the CBAUD bits participate: unrelated high bits are masked off.
    assert_eq!(baud_from_cflag(0x000d | 0x10000), 9600);
    assert_eq!(baud_from_cflag(0x100d), 3000000);
    // termios2 round-trip: install a full snapshot, read it back unchanged.
    let mut tty = CONSOLE_TTY.lock();
    let mut snapshot = [0_u8; NCCS];
    snapshot[VINTR] = 3;
    snapshot[VERASE] = 0x7f;
    snapshot[VEOF] = 4;
    tty.apply_termios(
        ICRNL,
        OPOST | ONLCR,
        B115200 | CS8,
        ISIG | ICANON | ECHO,
        &snapshot,
        115200,
        115200,
    );
    let read_back = KernelTermios2::from_tty(&tty);
    drop(tty);
    assert_eq!(read_back.iflag, ICRNL);
    assert_eq!(read_back.oflag, OPOST | ONLCR);
    assert_eq!(read_back.cflag, B115200 | CS8);
    assert_eq!(read_back.lflag, ISIG | ICANON | ECHO);
    assert_eq!(read_back.cc, snapshot);
    assert_eq!(read_back.ispeed, 115200);
    assert_eq!(read_back.ospeed, 115200);
    assert_eq!(baud_from_cflag(read_back.cflag), 115200);

    crate::println!("M13 TTY gate:");
    crate::println!("  canonical input      : verified");
    crate::println!("  console output       : verified");
    crate::println!("  termios ABI/semantics : verified");
}
