//! 信号子系统（Linux 风格）。
//!
//! 参照 Linux `kernel/signal.c`。
//!
//! M12/M13: Integrated with SudoOS-Plus's Arc<Process> + IrqSpinLock model.
//! All process state is accessed via closure-based helpers that manage locking.

// ---------------------------------------------------------------------------
// 信号编号
// ---------------------------------------------------------------------------

pub const SIGHUP: u32 = 1;
pub const SIGINT: u32 = 2;
pub const SIGQUIT: u32 = 3;
pub const SIGILL: u32 = 4;
pub const SIGTRAP: u32 = 5;
pub const SIGABRT: u32 = 6;
pub const SIGBUS: u32 = 7;
pub const SIGFPE: u32 = 8;
pub const SIGKILL: u32 = 9;
pub const SIGUSR1: u32 = 10;
pub const SIGSEGV: u32 = 11;
pub const SIGUSR2: u32 = 12;
pub const SIGPIPE: u32 = 13;
pub const SIGALRM: u32 = 14;
pub const SIGTERM: u32 = 15;
pub const SIGCHLD: u32 = 17;
pub const SIGCONT: u32 = 18;
pub const SIGSTOP: u32 = 19;
pub const SIGTSTP: u32 = 20;
pub const SIGURG: u32 = 23;
pub const SIGWINCH: u32 = 28;

pub const NSIG: usize = 33;
const SIGSET_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// 信号集
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SigSet(pub u64);

impl SigSet {
    pub const fn empty() -> Self { Self(0) }

    pub fn contains(&self, signum: u32) -> bool {
        if signum == 0 || signum > SIGSET_SIZE as u32 { return false; }
        self.0 & (1u64 << (signum - 1)) != 0
    }

    pub fn add(&mut self, signum: u32) {
        if signum > 0 && signum <= SIGSET_SIZE as u32 {
            self.0 |= 1u64 << (signum - 1);
        }
    }

    pub fn remove(&mut self, signum: u32) {
        if signum > 0 && signum <= SIGSET_SIZE as u32 {
            self.0 &= !(1u64 << (signum - 1));
        }
    }

    pub fn union_with(&self, other: SigSet) -> Self { Self(self.0 | other.0) }
    pub fn intersect_with(&self, other: SigSet) -> Self { Self(self.0 & other.0) }
}

impl From<u64> for SigSet {
    fn from(bits: u64) -> Self { Self(bits) }
}

impl From<SigSet> for u64 {
    fn from(set: SigSet) -> u64 { set.0 }
}

// ---------------------------------------------------------------------------
// 信号动作
// ---------------------------------------------------------------------------

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

pub const SA_NODEFER: u64 = 0x40000000;
pub const SA_RESETHAND: u64 = 0x80000000;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SigAction {
    pub handler: usize,
    pub flags: u64,
    pub restorer: usize,
    pub mask: SigSet,
}

impl SigAction {
    pub const fn default() -> Self {
        Self { handler: SIG_DFL, flags: 0, restorer: 0, mask: SigSet::empty() }
    }
}

// ---------------------------------------------------------------------------
// 信号状态
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SignalState {
    pub pending: SigSet,
    pub blocked: SigSet,
    pub actions: [SigAction; NSIG],
}

impl SignalState {
    pub fn new() -> Self {
        Self {
            pending: SigSet::empty(),
            blocked: SigSet::empty(),
            actions: [SigAction::default(); NSIG],
        }
    }

    pub fn clone_for_fork(&self) -> Self {
        Self {
            pending: SigSet::empty(),
            blocked: self.blocked,
            actions: self.actions,
        }
    }

    pub fn action_for(&self, signum: u32) -> Option<&SigAction> {
        if signum == 0 || signum >= NSIG as u32 { return None; }
        Some(&self.actions[signum as usize])
    }

    pub fn action_mut(&mut self, signum: u32) -> Option<&mut SigAction> {
        if signum == 0 || signum >= NSIG as u32 { return None; }
        Some(&mut self.actions[signum as usize])
    }

    pub fn next_signal(&self) -> Option<u32> {
        let bits = self.pending.0 & !self.blocked.0;
        if bits == 0 { return None; }
        Some((bits.trailing_zeros() + 1) as u32)
    }

    pub fn add_pending(&mut self, signum: u32) {
        if signum > 0 && signum < SIGSET_SIZE as u32 {
            self.pending.add(signum);
        }
    }

    pub fn clear_pending(&mut self, signum: u32) { self.pending.remove(signum); }
    pub fn has_pending(&self) -> bool { self.pending.0 & !self.blocked.0 != 0 }
}

// ---------------------------------------------------------------------------
// 信号发送
// ---------------------------------------------------------------------------

pub fn send_signal(target_pid: crate::process::ProcessId, signum: u32) -> bool {
    if signum == 0 || signum >= SIGSET_SIZE as u32 { return false; }

    let result = crate::process::lookup_process(target_pid).map(|process| {
        process.with_signal_mut(|sig| sig.add_pending(signum));
    });

    if result.is_some() {
        crate::task::request_reschedule_local();
        true
    } else {
        false
    }
}

pub fn kill_pgrp(pgrp: i32, signum: u32) -> bool {
    if pgrp <= 0 { return false; }
    let current = crate::process::current_pid();
    if crate::process::getpgrp(current) == pgrp {
        send_signal(current, signum)
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// 信号递送
// ---------------------------------------------------------------------------

/// Check and deliver pending signals before returning to user mode.
/// Returns true if a signal was delivered (trap frame modified).
pub fn do_signal(frame: &mut crate::arch::trap::TrapFrame) -> bool {
    let pid = crate::process::current_pid();

    let action_info = crate::process::lookup_process(pid).and_then(|process| {
        process.with_signal_mut(|sig| {
            let signum = sig.next_signal()?;
            let action = sig.action_for(signum)?;
            match action.handler {
                SIG_DFL => {
                    match signum {
                        SIGCHLD | SIGURG | SIGWINCH | SIGCONT => {
                            sig.clear_pending(signum);
                            None
                        }
                        _ => {
                            sig.clear_pending(signum);
                            Some((signum, SigAction::default(), true)) // true = terminate
                        }
                    }
                }
                SIG_IGN => {
                    sig.clear_pending(signum);
                    None
                }
                handler if handler > 1 => {
                    let act_copy = *action;
                    sig.clear_pending(signum);
                    Some((signum, act_copy, false))
                }
                _ => {
                    sig.clear_pending(signum);
                    None
                }
            }
        })
    });

    match action_info {
        None => false,
        Some((signum, action, should_terminate)) => {
            if should_terminate {
                crate::process::exit_process(pid, signum);
                false // Process is being terminated
            } else {
                setup_rt_frame(frame, signum, &action);
                true
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 用户态信号帧
// ---------------------------------------------------------------------------

fn setup_rt_frame(frame: &mut crate::arch::trap::TrapFrame, signum: u32, action: &SigAction) {
    let old_sp = frame.user_stack_pointer();
    const SIGFRAME_SIZE: usize = 1024;
    let aligned_size = (SIGFRAME_SIZE + 15) & !15;
    let new_sp = old_sp.checked_sub(aligned_size)
        .expect("signal frame overflowed user stack");

    save_trap_frame_to_stack(new_sp, frame);

    frame.set_user_stack_pointer(new_sp);
    frame.set_return_address(action.restorer);
    frame.set_argument_register(0, signum as usize);
    frame.set_program_counter(action.handler);

    let pid = crate::process::current_pid();
    if let Some(process) = crate::process::lookup_process(pid) {
        process.with_signal_mut(|sig| {
            sig.blocked.add(signum);
            sig.blocked = sig.blocked.union_with(action.mask);
            if action.flags & SA_RESETHAND != 0 {
                if let Some(slot) = sig.action_mut(signum) {
                    *slot = SigAction::default();
                }
            }
        });
    }
}

fn save_trap_frame_to_stack(sp: usize, frame: &crate::arch::trap::TrapFrame) {
    let size = core::mem::size_of::<crate::arch::trap::TrapFrame>();
    let dst = sp as *mut u8;
    unsafe {
        let src = (frame as *const crate::arch::trap::TrapFrame) as *const u8;
        core::ptr::copy_nonoverlapping(src, dst, size);
    }
}

pub fn restore_sigframe(frame: &mut crate::arch::trap::TrapFrame) -> bool {
    let sp = frame.user_stack_pointer();
    let size = core::mem::size_of::<crate::arch::trap::TrapFrame>();
    let src = sp as *const u8;
    unsafe {
        let dst = (frame as *mut crate::arch::trap::TrapFrame) as *mut u8;
        core::ptr::copy_nonoverlapping(src, dst, size);
    }

    let pid = crate::process::current_pid();
    if let Some(process) = crate::process::lookup_process(pid) {
        process.with_signal_mut(|sig| {
            sig.blocked = SigSet::empty();
        });
    }

    true
}

// ---------------------------------------------------------------------------
// ABI 序列化
// ---------------------------------------------------------------------------

/// Read sigaction from user space. `mm` must be the current thread's UserMm.
pub fn copy_sigaction_from_user(mm: &crate::user_mm::UserMm, user_ptr: usize) -> Option<SigAction> {
    if user_ptr == 0 { return None; }
    let mut raw = [0u8; 32];
    mm.copy_from_user(user_ptr, &mut raw).ok()?;
    let handler = usize::from_ne_bytes(raw[0..8].try_into().ok()?);
    let flags = u64::from_ne_bytes(raw[8..16].try_into().ok()?);
    let restorer = usize::from_ne_bytes(raw[16..24].try_into().ok()?);
    let mask_bits = u64::from_ne_bytes(raw[24..32].try_into().ok()?);
    Some(SigAction { handler, flags, restorer, mask: SigSet::from(mask_bits) })
}

/// Write sigaction to user space. `mm` must be the current thread's UserMm.
pub fn copy_sigaction_to_user(mm: &crate::user_mm::UserMm, user_ptr: usize, action: &SigAction) -> Result<(), ()> {
    if user_ptr == 0 { return Ok(()); }
    let mut raw = [0u8; 32];
    raw[0..8].copy_from_slice(&action.handler.to_ne_bytes());
    raw[8..16].copy_from_slice(&action.flags.to_ne_bytes());
    raw[16..24].copy_from_slice(&action.restorer.to_ne_bytes());
    let mask_bits: u64 = action.mask.into();
    raw[24..32].copy_from_slice(&mask_bits.to_ne_bytes());
    mm.copy_to_user(user_ptr, &raw).map_err(|_| ())
}

/// Read sigset from user space. `mm` must be the current thread's UserMm.
pub fn copy_sigset_from_user(mm: &crate::user_mm::UserMm, user_ptr: usize) -> Option<SigSet> {
    if user_ptr == 0 { return Some(SigSet::empty()); }
    let mut raw = [0u8; 8];
    mm.copy_from_user(user_ptr, &mut raw).ok()?;
    Some(SigSet::from(u64::from_ne_bytes(raw)))
}

/// Write sigset to user space. `mm` must be the current thread's UserMm.
pub fn copy_sigset_to_user(mm: &crate::user_mm::UserMm, user_ptr: usize, set: &SigSet) -> Result<(), ()> {
    if user_ptr == 0 { return Ok(()); }
    let bits: u64 = (*set).into();
    mm.copy_to_user(user_ptr, &bits.to_ne_bytes()).map_err(|_| ())
}

// ---------------------------------------------------------------------------
// sigprocmask
// ---------------------------------------------------------------------------

pub const SIG_BLOCK: usize = 0;
pub const SIG_UNBLOCK: usize = 1;
pub const SIG_SETMASK: usize = 2;

/// Modify signal mask. `mm` must be the current thread's UserMm.
pub fn do_sigprocmask(mm: &crate::user_mm::UserMm, how: usize, set: SigSet, oldset_ptr: usize) -> Result<(), ()> {
    let pid = crate::process::current_pid();
    let process = crate::process::lookup_process(pid).ok_or(())?;

    // Save old value — must use the passed mm to avoid Scheduler lock inside Process lock
    if oldset_ptr != 0 {
        process.with_signal(|sig| {
            let _ = copy_sigset_to_user(mm, oldset_ptr, &sig.blocked);
        });
    }

    // Apply new mask
    let mut effective = set;
    effective.remove(SIGKILL);
    effective.remove(SIGSTOP);

    process.with_signal_mut(|sig| {
        match how {
            SIG_BLOCK => {
                sig.blocked = sig.blocked.union_with(effective);
                Ok(())
            }
            SIG_UNBLOCK => {
                let unblocked = sig.blocked.intersect_with(effective);
                sig.blocked.0 &= !unblocked.0;
                Ok(())
            }
            SIG_SETMASK => {
                sig.blocked = effective;
                Ok(())
            }
            _ => Err(()),
        }
    })
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

pub fn initialize() {
    crate::println!("signal subsystem:");
    crate::println!("  nsig           : {}", NSIG);
    crate::println!("  sigsetsize     : {} bytes", 8);
    crate::println!("  sigaction size : {} bytes", 32);
    crate::println!("  sigframe size  : 1024 bytes");
}
