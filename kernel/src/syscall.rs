//! Linux asm-generic 64-bit syscall ABI shared by both SudoOS ports.
//!
//! This module owns ABI facts only. Syscall policy remains in the dispatcher so
//! architecture-specific register handling cannot drift as the table grows.

pub mod number {
    // Basic I/O
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const OPENAT: usize = 56;
    pub const CLOSE: usize = 57;
    // File descriptor
    pub const PIPE2: usize = 59;
    pub const DUP: usize = 23;
    pub const DUP3: usize = 24;
    // Process control
    pub const CLONE: usize = 220;
    pub const EXECVE: usize = 221;
    pub const EXIT: usize = 93;
    pub const EXIT_GROUP: usize = 94;
    pub const WAIT4: usize = 260;
    // Memory
    pub const BRK: usize = 214;
    pub const MUNMAP: usize = 215;
    pub const MMAP: usize = 222;
    pub const MPROTECT: usize = 226;
    // Scheduling
    pub const SCHED_YIELD: usize = 124;
    pub const NANOSLEEP: usize = 101;
    // Signals
    pub const KILL: usize = 129;
    pub const TKILL: usize = 130;
    pub const TGKILL: usize = 131;
    pub const RT_SIGACTION: usize = 134;
    pub const RT_SIGPROCMASK: usize = 135;
    pub const RT_SIGRETURN: usize = 139;
    // Process info
    pub const GETPID: usize = 172;
    pub const GETPPID: usize = 173;
    // Session / process group
    pub const SETSID: usize = 132;
    pub const SETPGID: usize = 133;
    pub const GETPGID: usize = 155;
    pub const GETPGRP: usize = 155; // getpgrp() is getpgid(0) in Linux
    pub const GETSID: usize = 156;
    // Device I/O
    pub const IOCTL: usize = 29;
    // Time
    pub const GETTIMEOFDAY: usize = 169;
    pub const CLOCK_GETTIME: usize = 113;
    pub const TIMES: usize = 153;
    // System info
    pub const UNAME: usize = 160;
    pub const GETRANDOM: usize = 278;
}

pub mod errno {
    pub const EPERM: isize = 1;
    pub const ENOENT: isize = 2;
    pub const ESRCH: isize = 3;
    pub const EINTR: isize = 4;
    pub const EIO: isize = 5;
    pub const EBADF: isize = 9;
    pub const ECHILD: isize = 10;
    pub const EAGAIN: isize = 11;
    pub const ENOMEM: isize = 12;
    pub const EACCES: isize = 13;
    pub const EFAULT: isize = 14;
    pub const EBUSY: isize = 16;
    pub const EEXIST: isize = 17;
    pub const EINVAL: isize = 22;
    pub const ENFILE: isize = 23;
    pub const EMFILE: isize = 24;
    pub const EPIPE: isize = 32;
    pub const ENOSYS: isize = 38;
    pub const MAX_ERRNO: isize = 4095;

    pub const fn encode(errno: isize) -> isize {
        -errno
    }

    pub const fn is_error(value: isize) -> bool {
        value < 0 && value >= -MAX_ERRNO
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub number: usize,
    pub arguments: [usize; 6],
}

pub mod abi {
    use super::Request;

    pub const SYSCALL_INSTRUCTION_BYTES: usize = 4;

    #[cfg(target_arch = "riscv64")]
    pub fn decode(frame: &crate::arch::trap::TrapFrame) -> Request {
        Request {
            number: frame.gpr[17],
            arguments: [
                frame.gpr[10],
                frame.gpr[11],
                frame.gpr[12],
                frame.gpr[13],
                frame.gpr[14],
                frame.gpr[15],
            ],
        }
    }

    #[cfg(target_arch = "loongarch64")]
    pub fn decode(frame: &crate::arch::trap::TrapFrame) -> Request {
        Request {
            number: frame.gpr[11],
            arguments: [
                frame.gpr[4],
                frame.gpr[5],
                frame.gpr[6],
                frame.gpr[7],
                frame.gpr[8],
                frame.gpr[9],
            ],
        }
    }

    pub fn advance(frame: &mut crate::arch::trap::TrapFrame) {
        frame.advance_pc(SYSCALL_INSTRUCTION_BYTES);
    }

    #[cfg(target_arch = "riscv64")]
    pub fn set_result(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
        frame.gpr[10] = result as usize;
    }

    #[cfg(target_arch = "loongarch64")]
    pub fn set_result(frame: &mut crate::arch::trap::TrapFrame, result: isize) {
        frame.gpr[4] = result as usize;
    }
}

pub fn verify_contract() {
    assert_eq!(number::WRITE, 64);
    assert_eq!(number::EXIT, 93);
    assert_eq!(number::EXIT_GROUP, 94);
    assert_eq!(number::SCHED_YIELD, 124);
    assert_eq!(number::BRK, 214);
    assert_eq!(number::MUNMAP, 215);
    assert_eq!(number::MMAP, 222);
    assert_eq!(number::MPROTECT, 226);
    assert!(errno::is_error(errno::encode(errno::ENOSYS)));
    assert!(!errno::is_error(0));
}
