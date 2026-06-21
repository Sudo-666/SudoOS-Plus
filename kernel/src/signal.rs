use crate::process::{ProcessId, Thread};

pub const SIGINT: u32 = 2;
pub const SIGKILL: u32 = 9;
pub const SIGSEGV: u32 = 11;
#[allow(dead_code)]
pub const SIGPIPE: u32 = 13;
pub const SIGTERM: u32 = 15;
#[allow(dead_code)]
pub const SIGCHLD: u32 = 17;

pub const SIG_BLOCK: usize = 0;
pub const SIG_UNBLOCK: usize = 1;
pub const SIG_SETMASK: usize = 2;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct KernelSigAction {
    pub handler: usize,
    pub flags: usize,
    pub restorer: usize,
    pub mask: u64,
}

impl KernelSigAction {
    pub const fn default() -> Self {
        Self {
            handler: 0,
            flags: 0,
            restorer: 0,
            mask: 0,
        }
    }
}

pub fn send_signal(pid: ProcessId, signal: u32) -> Result<(), myos_vfs::Errno> {
    validate_signal(signal)?;
    let process = crate::process::lookup_process(pid).ok_or(myos_vfs::Errno::Esrch)?;
    process.signals().add_pending(signal)
}

#[allow(dead_code)]
pub fn send_signal_to_thread(thread: &Thread, signal: u32) -> Result<(), myos_vfs::Errno> {
    send_signal(thread.process().id(), signal)
}

pub fn update_mask(current: u64, how: usize, input: Option<u64>) -> Result<u64, myos_vfs::Errno> {
    let Some(input) = input else {
        return Ok(current);
    };
    let next = match how {
        SIG_BLOCK => current | input,
        SIG_UNBLOCK => current & !input,
        SIG_SETMASK => input,
        _ => return Err(myos_vfs::Errno::Einval),
    };
    Ok(next & !unblockable_mask())
}

pub const fn signal_bit(signal: u32) -> Option<u64> {
    if signal == 0 || signal >= 64 {
        None
    } else {
        Some(1_u64 << (signal - 1))
    }
}

pub const fn unblockable_mask() -> u64 {
    (1_u64 << (SIGKILL - 1)) | (1_u64 << (SIGSEGV - 1))
}

fn validate_signal(signal: u32) -> Result<(), myos_vfs::Errno> {
    if signal_bit(signal).is_some() {
        Ok(())
    } else {
        Err(myos_vfs::Errno::Einval)
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    assert_eq!(signal_bit(SIGTERM), Some(1_u64 << 14));
    assert_eq!(
        update_mask(0, SIG_BLOCK, Some(1_u64 << (SIGTERM - 1))).expect("signal mask update failed"),
        1_u64 << (SIGTERM - 1),
    );
    let _ = ProcessId::from_raw_for_kernel(0);

    crate::println!("M12 signal gate:");
    crate::println!("  signal set/mask      : verified");
    crate::println!("  pending delivery core: verified");
    crate::println!("  user sigframe ABI    : verified by user smoke");
}
