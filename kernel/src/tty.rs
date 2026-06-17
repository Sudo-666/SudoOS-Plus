//! TTY 子系统（Linux 风格）。
//!
//! 参照 Linux `drivers/tty/n_tty.c`（N_TTY line discipline）。
//!
//! 提供：
//! - 行规则（canonical / raw 模式）
//! - echo、backspace、行编辑
//! - Ctrl-C → SIGINT、Ctrl-D → EOF、Ctrl-\ → SIGQUIT
//! - 前台进程组跟踪
//! - 读端 / 写端等待队列
//!
//! 不使用完整 termios，仅支持最小必需的标志位。

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicI32, Ordering};

use crate::file_table::{File, FileError, FileOperations, OpenFlags};
use crate::signal;
use crate::task::WaitQueue;

// ---------------------------------------------------------------------------
// 行规则缓冲区
// ---------------------------------------------------------------------------

/// 规范模式缓冲区大小（与 Linux N_TTY_BUF_SIZE 一致：4096）。
const N_TTY_BUF_SIZE: usize = 4096;

/// 规范模式行缓冲区。
struct CanonBuffer {
    data: [u8; N_TTY_BUF_SIZE],
    head: usize, // 读指针
    tail: usize, // 写指针
}

impl CanonBuffer {
    const fn new() -> Self {
        Self {
            data: [0u8; N_TTY_BUF_SIZE],
            head: 0,
            tail: 0,
        }
    }

    fn len(&self) -> usize {
        if self.tail >= self.head {
            self.tail - self.head
        } else {
            N_TTY_BUF_SIZE - self.head + self.tail
        }
    }

    fn space(&self) -> usize {
        N_TTY_BUF_SIZE - self.len() - 1
    }

    fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    fn put(&mut self, byte: u8) {
        let next = (self.tail + 1) % N_TTY_BUF_SIZE;
        if next != self.head {
            self.data[self.tail] = byte;
            self.tail = next;
        }
    }

    fn get(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let byte = self.data[self.head];
        self.head = (self.head + 1) % N_TTY_BUF_SIZE;
        Some(byte)
    }

    fn erase_last(&mut self) -> bool {
        if self.is_empty() {
            return false;
        }
        self.tail = if self.tail == 0 {
            N_TTY_BUF_SIZE - 1
        } else {
            self.tail - 1
        };
        true
    }

    fn flush(&mut self) {
        self.head = 0;
        self.tail = 0;
    }

    fn consume_to(&mut self, count: usize) -> usize {
        let avail = self.len();
        let n = count.min(avail);
        self.head = (self.head + n) % N_TTY_BUF_SIZE;
        n
    }

    fn copy_to_user_buf(&self, buf: &mut [u8]) -> usize {
        let avail = self.len();
        let n = buf.len().min(avail);
        for i in 0..n {
            buf[i] = self.data[(self.head + i) % N_TTY_BUF_SIZE];
        }
        n
    }
}

// ---------------------------------------------------------------------------
// Termios 标志（简化版）
// ---------------------------------------------------------------------------

/// c_lflag 位掩码。
#[derive(Clone, Copy, Debug)]
pub struct TermiosLflag(u32);

impl TermiosLflag {
    pub const ICANON: u32 = 0x0002;
    pub const ECHO: u32 = 0x0008;
    pub const ECHOE: u32 = 0x0010;
    pub const ECHOK: u32 = 0x0020;
    pub const ECHONL: u32 = 0x0040;
    pub const ISIG: u32 = 0x0001;

    pub const fn canonical() -> Self {
        Self(Self::ICANON | Self::ECHO | Self::ECHOE | Self::ECHOK | Self::ECHONL | Self::ISIG)
    }

    pub const fn raw() -> Self {
        Self(0)
    }

    pub fn contains(&self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub fn set(&mut self, flag: u32) {
        self.0 |= flag;
    }

    pub fn clear(&mut self, flag: u32) {
        self.0 &= !flag;
    }
}

impl Default for TermiosLflag {
    fn default() -> Self {
        Self::canonical()
    }
}

// ---------------------------------------------------------------------------
// TTY 对象（参照 Linux `struct tty_struct`）
// ---------------------------------------------------------------------------

/// TTY 设备（参照 Linux `struct tty_struct`）。
///
/// 每个 TTY 有一个关联的前台进程组。当收到特殊字符
/// （Ctrl-C、Ctrl-\ 等）时，向前台进程组发送信号。
///
/// 内部使用 `UnsafeCell` 实现内部可变性。
pub struct Tty {
    /// 规范模式读缓冲区（UnsafeCell）。
    read_buf: UnsafeCell<CanonBuffer>,
    /// 当前正在构建的行（仅在 ICANON 模式下使用）。
    line_state: UnsafeCell<LineState>,
    /// termios c_lflag 标志。
    lflag: UnsafeCell<TermiosLflag>,
    /// 前台进程组 ID（0 = 无前台进程组）。
    foreground_pgrp: AtomicI32,
    /// 所属会话 ID。
    session: AtomicI32,
    /// 读端等待队列（缓冲区为空时阻塞读者）。
    read_wait: WaitQueue,
    /// echo 缓冲。
    echo_buf: UnsafeCell<[u8; 256]>,
    echo_len: UnsafeCell<usize>,
}

// SAFETY: TTY is Sync — all mutable fields are protected by UnsafeCell and
// access is serialized through the IrqSpinLock in SYSTEM_TTY.
unsafe impl Sync for Tty {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineState {
    /// 无活动行。
    Idle,
    /// 一行已完成（等待读取）。
    Complete,
}

impl Tty {
    pub fn new() -> Self {
        Self {
            read_buf: UnsafeCell::new(CanonBuffer::new()),
            line_state: UnsafeCell::new(LineState::Idle),
            lflag: UnsafeCell::new(TermiosLflag::canonical()),
            foreground_pgrp: AtomicI32::new(0),
            session: AtomicI32::new(0),
            read_wait: WaitQueue::new(),
            echo_buf: UnsafeCell::new([0u8; 256]),
            echo_len: UnsafeCell::new(0),
        }
    }

    // SAFETY helpers: TTY access is serialized by SYSTEM_TTY lock.
    fn read_buf(&self) -> &mut CanonBuffer {
        unsafe { &mut *self.read_buf.get() }
    }
    fn line_state_mut(&self) -> &mut LineState {
        unsafe { &mut *self.line_state.get() }
    }
    fn lflag(&self) -> &mut TermiosLflag {
        unsafe { &mut *self.lflag.get() }
    }
    fn echo_buf(&self) -> &mut [u8; 256] {
        unsafe { &mut *self.echo_buf.get() }
    }
    fn echo_len_mut(&self) -> &mut usize {
        unsafe { &mut *self.echo_len.get() }
    }

    // -----------------------------------------------------------------------
    // 模式控制
    // -----------------------------------------------------------------------

    pub fn set_canonical(&self) {
        *self.lflag() = TermiosLflag::canonical();
        self.flush_line();
    }

    pub fn set_raw(&self) {
        *self.lflag() = TermiosLflag::raw();
        self.flush_line();
    }

    /// 检查是否为规范（行缓冲）模式。
    pub fn is_canonical(&self) -> bool {
        self.lflag().contains(TermiosLflag::ICANON)
    }

    // -----------------------------------------------------------------------
    // 进程组 / 会话
    // -----------------------------------------------------------------------

    /// 设置前台进程组。
    pub fn set_foreground_pgrp(&self, pgrp: i32) {
        self.foreground_pgrp.store(pgrp, Ordering::Release);
    }

    /// 获取前台进程组。
    pub fn foreground_pgrp(&self) -> i32 {
        self.foreground_pgrp.load(Ordering::Acquire)
    }

    /// 设置所属会话 ID。
    pub fn set_session(&self, sid: i32) {
        self.session.store(sid, Ordering::Release);
    }

    // -----------------------------------------------------------------------
    // 内核输入路径（从 console driver 接收字符）
    // -----------------------------------------------------------------------

    /// 向 TTY 输入一个字符（从硬件控制台驱动调用）。
    ///
    /// 处理 echo、行编辑、信号生成等。
    pub fn input_char(&self, byte: u8) {
        let c = byte;
        let isig = self.lflag().contains(TermiosLflag::ISIG);

        // 检查特殊字符（仅在 ISIG 开启时生效）
        if isig {
            match c {
                // Ctrl-C → SIGINT
                0x03 => {
                    self.signal_foreground(signal::SIGINT);
                    if self.lflag().contains(TermiosLflag::ECHO) {
                        self.echo_str("^C\r\n");
                    }
                    return;
                }
                // Ctrl-\ → SIGQUIT
                0x1c => {
                    self.signal_foreground(signal::SIGQUIT);
                    if self.lflag().contains(TermiosLflag::ECHO) {
                        self.echo_str("^\\\r\n");
                    }
                    return;
                }
                // Ctrl-Z → SIGTSTP
                0x1a => {
                    self.signal_foreground(signal::SIGTSTP);
                    if self.lflag().contains(TermiosLflag::ECHO) {
                        self.echo_str("^Z\r\n");
                    }
                    return;
                }
                _ => {}
            }
        }

        if self.lflag().contains(TermiosLflag::ICANON) {
            self.input_canonical(c);
        } else {
            self.input_raw(c);
        }
    }

    /// 规范模式字符处理。
    fn input_canonical(&self, c: u8) {
        let echo = self.lflag().contains(TermiosLflag::ECHO);
        let echoe = self.lflag().contains(TermiosLflag::ECHOE);

        match c {
            b'\r' | b'\n' => {
                self.read_buf().put(b'\n');
                *self.line_state_mut() = LineState::Complete;
                if echo {
                    self.echo_str("\r\n");
                }
                self.read_wait.wake_all();
            }
            0x08 | 0x7f => {
                if self.read_buf().erase_last() {
                    if echo && echoe {
                        self.echo_str("\x08 \x08");
                    }
                }
            }
            0x04 => {
                if self.read_buf().is_empty() {
                    *self.line_state_mut() = LineState::Complete;
                    if echo {
                        self.echo_str("^D\r\n");
                    }
                    self.read_wait.wake_all();
                }
            }
            0x15 => {
                self.read_buf().flush();
                if echo && self.lflag().contains(TermiosLflag::ECHOK) {
                    self.echo_str("^U\r\n");
                }
            }
            _ => {
                if self.read_buf().space() > 0 {
                    self.read_buf().put(c);
                    if echo {
                        self.echo_byte(c);
                    }
                }
            }
        }
    }

    /// 原始模式字符处理。
    fn input_raw(&self, c: u8) {
        if self.read_buf().space() > 0 {
            self.read_buf().put(c);
            if self.lflag().contains(TermiosLflag::ECHO) {
                self.echo_byte(c);
            }
            self.read_wait.wake_one();
        }
    }

    // -----------------------------------------------------------------------
    // 用户读取路径
    // -----------------------------------------------------------------------

    pub fn do_read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        if self.is_canonical() {
            match *self.line_state_mut() {
                LineState::Complete => {
                    let n = self.read_buf().copy_to_user_buf(buf);
                    self.read_buf().consume_to(n);
                    *self.line_state_mut() = LineState::Idle;
                    Ok(n)
                }
                _ => {
                    if self.read_buf().is_empty() {
                        Err(FileError::WouldBlock)
                    } else {
                        let n = self.read_buf().copy_to_user_buf(buf);
                        self.read_buf().consume_to(n);
                        if self.read_buf().is_empty() {
                            *self.line_state_mut() = LineState::Idle;
                        }
                        Ok(n)
                    }
                }
            }
        } else {
            if self.read_buf().is_empty() {
                Err(FileError::WouldBlock)
            } else {
                let n = self.read_buf().copy_to_user_buf(buf);
                self.read_buf().consume_to(n);
                Ok(n)
            }
        }
    }

    // -----------------------------------------------------------------------
    // 用户写入路径
    // -----------------------------------------------------------------------

    pub fn do_write(&self, buf: &[u8]) -> Result<usize, FileError> {
        let driver = console_driver();
        let mut written = 0;
        for &byte in buf {
            driver.write_byte(byte);
            written += 1;
            if byte == b'\n' {
                driver.write_byte(b'\r');
            }
        }
        Ok(written)
    }

    // -----------------------------------------------------------------------
    // 等待队列
    // -----------------------------------------------------------------------

    pub fn read_wait_queue(&self) -> &WaitQueue {
        &self.read_wait
    }

    // -----------------------------------------------------------------------
    // 内部辅助
    // -----------------------------------------------------------------------

    fn flush_line(&self) {
        self.read_buf().flush();
        *self.line_state_mut() = LineState::Idle;
    }

    fn signal_foreground(&self, sig: u32) {
        let pgrp = self.foreground_pgrp();
        if pgrp > 0 {
            signal::send_signal(crate::process::ProcessId(pgrp as usize), sig);
        }
    }

    fn echo_byte(&self, byte: u8) {
        let driver = console_driver();
        driver.write_byte(byte);
    }

    fn echo_str(&self, s: &str) {
        let driver = console_driver();
        for byte in s.bytes() {
            driver.write_byte(byte);
        }
    }
}

// ---------------------------------------------------------------------------
// 控制台驱动 trait
// ---------------------------------------------------------------------------

/// 控制台驱动抽象（参照 Linux `struct tty_driver` 的 output 部分）。
pub trait ConsoleDriver: Send + Sync {
    /// 向硬件控制台写入一个字节。
    fn write_byte(&self, byte: u8);
}

/// 全局控制台驱动（由架构模块在启动时注册）。
static CONSOLE_DRIVER: crate::irq_lock::IrqSpinLock<Option<&'static dyn ConsoleDriver>> =
    crate::irq_lock::IrqSpinLock::new_with_class(
        None,
        crate::lockdep::LockClass::new("console_driver", crate::lockdep::LockRank::Console, 2),
    );

/// 注册全局控制台驱动。
pub fn set_console_driver(driver: &'static dyn ConsoleDriver) {
    let mut slot = CONSOLE_DRIVER.lock();
    *slot = Some(driver);
}

fn console_driver() -> &'static dyn ConsoleDriver {
    CONSOLE_DRIVER
        .lock()
        .expect("console driver not initialized")
}

// ---------------------------------------------------------------------------
// 全局 TTY 实例（单一系统控制台）
// ---------------------------------------------------------------------------

static SYSTEM_TTY: crate::irq_lock::IrqSpinLock<Option<Tty>> =
    crate::irq_lock::IrqSpinLock::new_with_class(
        None,
        crate::lockdep::LockClass::new("system_tty", crate::lockdep::LockRank::Console, 3),
    );

/// 获取系统 TTY。
pub fn system_tty() -> &'static crate::irq_lock::IrqSpinLock<Option<Tty>> {
    &SYSTEM_TTY
}

/// 初始化系统 TTY。
pub fn initialize() {
    {
        let mut slot = SYSTEM_TTY.lock();
        *slot = Some(Tty::new());
    }

    // 注册默认控制台驱动
    struct ArchConsole;
    impl ConsoleDriver for ArchConsole {
        fn write_byte(&self, byte: u8) {
            crate::arch::early_console::write_byte(byte);
        }
    }
    set_console_driver(&ArchConsole);

    crate::println!("tty subsystem:");
    crate::println!("  line discipline: N_TTY (canonical + raw)");
    crate::println!("  buffer size    : {} bytes", N_TTY_BUF_SIZE);
    crate::println!("  signal keys    : ^C (SIGINT) ^\\ (SIGQUIT) ^Z (SIGTSTP)");
    crate::println!("  line editing   : ^H/DEL (erase) ^U (kill)");
}

// ---------------------------------------------------------------------------
// TTY FileOperations（/dev/console）
// ---------------------------------------------------------------------------

use alloc::sync::Arc;

struct TtyReader;

impl FileOperations for TtyReader {
    fn read(&self, buf: &mut [u8]) -> Result<usize, FileError> {
        let mut slot = SYSTEM_TTY.lock();
        let tty = slot.as_mut().ok_or(FileError::Invalid)?;
        tty.do_read(buf)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize, FileError> {
        Err(FileError::Invalid)
    }

    fn close(&self) {}
}

struct TtyWriter;

impl FileOperations for TtyWriter {
    fn read(&self, _buf: &mut [u8]) -> Result<usize, FileError> {
        Err(FileError::Invalid)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, FileError> {
        let slot = SYSTEM_TTY.lock();
        let tty = slot.as_ref().ok_or(FileError::Invalid)?;
        tty.do_write(buf)
    }

    fn close(&self) {}
}

/// 创建 /dev/console 文件描述符（读端）。
pub fn create_console_reader() -> File {
    File::new(
        Arc::new(TtyReader),
        OpenFlags(OpenFlags::RDONLY.0),
    )
}

/// 创建 /dev/console 文件描述符（写端）。
pub fn create_console_writer() -> File {
    File::new(
        Arc::new(TtyWriter),
        OpenFlags(OpenFlags::WRONLY.0),
    )
}

/// 从硬件控制台向 TTY 输入一个字符（由中断/轮询驱动调用）。
pub fn input_char(byte: u8) {
    let mut slot = SYSTEM_TTY.lock();
    if let Some(tty) = slot.as_mut() {
        tty.input_char(byte);
    }
}

/// 检查 TTY 是否有数据可读。
pub fn has_input() -> bool {
    let slot = SYSTEM_TTY.lock();
    slot.as_ref().is_some_and(|tty| !tty.read_buf().is_empty())
}
