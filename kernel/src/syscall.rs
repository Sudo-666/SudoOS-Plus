//! Linux asm-generic 64-bit syscall ABI shared by both SudoOS ports.
//!
//! This module owns ABI facts only. Syscall policy remains in the dispatcher so
//! architecture-specific register handling cannot drift as the table grows.

pub mod number {
    pub const GETCWD: usize = 17;
    pub const DUP: usize = 23;
    pub const DUP3: usize = 24;
    pub const FCNTL: usize = 25;
    pub const IOCTL: usize = 29;
    pub const MKDIRAT: usize = 34;
    pub const UNLINKAT: usize = 35;
    pub const SYMLINKAT: usize = 36;
    pub const LINKAT: usize = 37;
    pub const RENAMEAT: usize = 38;
    pub const UMOUNT2: usize = 39;
    pub const MOUNT: usize = 40;
    pub const FACCESSAT: usize = 48;
    pub const CHDIR: usize = 49;
    pub const OPENAT: usize = 56;
    pub const CLOSE: usize = 57;
    pub const PIPE2: usize = 59;
    pub const GETDENTS64: usize = 61;
    pub const LSEEK: usize = 62;
    pub const READ: usize = 63;
    pub const WRITE: usize = 64;
    pub const READV: usize = 65;
    pub const WRITEV: usize = 66;
    pub const PREAD64: usize = 67;
    pub const PSELECT6: usize = 72;
    pub const PPOLL: usize = 73;
    pub const READLINKAT: usize = 78;
    pub const NEWFSTATAT: usize = 79;
    pub const FSTAT: usize = 80;
    pub const FSYNC: usize = 82;
    pub const EXIT: usize = 93;
    pub const EXIT_GROUP: usize = 94;
    pub const SET_TID_ADDRESS: usize = 96;
    pub const SET_ROBUST_LIST: usize = 99;
    pub const NANOSLEEP: usize = 101;
    pub const CLOCK_GETTIME: usize = 113;
    pub const SCHED_YIELD: usize = 124;
    pub const KILL: usize = 129;
    pub const TKILL: usize = 130;
    pub const TGKILL: usize = 131;
    pub const SETSID: usize = 132;
    pub const SETPGID: usize = 133;
    pub const RT_SIGACTION: usize = 134;
    pub const RT_SIGPROCMASK: usize = 135;
    pub const RT_SIGTIMEDWAIT: usize = 137;
    pub const RT_SIGRETURN: usize = 139;
    pub const TIMES: usize = 153;
    pub const GETPGID: usize = 155;
    pub const GETSID: usize = 156;
    pub const UNAME: usize = 160;
    pub const GETTIMEOFDAY: usize = 169;
    pub const GETPID: usize = 172;
    pub const GETPPID: usize = 173;
    pub const GETUID: usize = 174;
    pub const GETEUID: usize = 175;
    pub const GETGID: usize = 176;
    pub const GETEGID: usize = 177;
    pub const GETTID: usize = 178;
    pub const SYSINFO: usize = 179;
    pub const BRK: usize = 214;
    pub const MUNMAP: usize = 215;
    pub const CLONE: usize = 220;
    pub const EXECVE: usize = 221;
    pub const MMAP: usize = 222;
    pub const MPROTECT: usize = 226;
    pub const WAIT4: usize = 260;
    pub const PRLIMIT64: usize = 261;
    pub const GETRANDOM: usize = 278;
    pub const STATX: usize = 291;
    pub const FTRUNCATE: usize = 46;
    // Socket syscalls (Linux asm-generic)
    pub const SOCKET: usize = 198;
    pub const BIND: usize = 200;
    pub const LISTEN: usize = 201;
    pub const ACCEPT: usize = 202;
    pub const CONNECT: usize = 203;
    pub const GETSOCKNAME: usize = 204;
    pub const GETPEERNAME: usize = 205;
    pub const SENDTO: usize = 206;
    pub const RECVFROM: usize = 207;
    pub const SHUTDOWN: usize = 210;
    pub const SETSOCKOPT: usize = 208;
    pub const GETSOCKOPT: usize = 209;
}

pub mod errno {
    pub const EPERM: isize = 1;
    pub const EBADF: isize = 9;
    pub const ESRCH: isize = 3;
    pub const ECHILD: isize = 10;
    pub const EAGAIN: isize = 11;
    pub const ENOMEM: isize = 12;
    pub const EFAULT: isize = 14;
    pub const EINVAL: isize = 22;
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
    assert_eq!(number::GETCWD, 17);
    assert_eq!(number::DUP, 23);
    assert_eq!(number::DUP3, 24);
    assert_eq!(number::FCNTL, 25);
    assert_eq!(number::IOCTL, 29);
    assert_eq!(number::MKDIRAT, 34);
    assert_eq!(number::UNLINKAT, 35);
    assert_eq!(number::SYMLINKAT, 36);
    assert_eq!(number::LINKAT, 37);
    assert_eq!(number::RENAMEAT, 38);
    assert_eq!(number::UMOUNT2, 39);
    assert_eq!(number::MOUNT, 40);
    assert_eq!(number::FTRUNCATE, 46);
    assert_eq!(number::FACCESSAT, 48);
    assert_eq!(number::CHDIR, 49);
    assert_eq!(number::OPENAT, 56);
    assert_eq!(number::CLOSE, 57);
    assert_eq!(number::PIPE2, 59);
    assert_eq!(number::GETDENTS64, 61);
    assert_eq!(number::LSEEK, 62);
    assert_eq!(number::READ, 63);
    assert_eq!(number::WRITE, 64);
    assert_eq!(number::PSELECT6, 72);
    assert_eq!(number::PPOLL, 73);
    assert_eq!(number::READLINKAT, 78);
    assert_eq!(number::NEWFSTATAT, 79);
    assert_eq!(number::FSTAT, 80);
    assert_eq!(number::FSYNC, 82);
    assert_eq!(number::EXIT, 93);
    assert_eq!(number::EXIT_GROUP, 94);
    assert_eq!(number::SET_TID_ADDRESS, 96);
    assert_eq!(number::SET_ROBUST_LIST, 99);
    assert_eq!(number::NANOSLEEP, 101);
    assert_eq!(number::CLOCK_GETTIME, 113);
    assert_eq!(number::SCHED_YIELD, 124);
    assert_eq!(number::KILL, 129);
    assert_eq!(number::RT_SIGACTION, 134);
    assert_eq!(number::RT_SIGPROCMASK, 135);
    assert_eq!(number::RT_SIGRETURN, 139);
    assert_eq!(number::UNAME, 160);
    assert_eq!(number::GETPID, 172);
    assert_eq!(number::GETPPID, 173);
    assert_eq!(number::GETUID, 174);
    assert_eq!(number::GETEUID, 175);
    assert_eq!(number::GETGID, 176);
    assert_eq!(number::GETEGID, 177);
    assert_eq!(number::GETTID, 178);
    assert_eq!(number::SYSINFO, 179);
    assert_eq!(number::BRK, 214);
    assert_eq!(number::MUNMAP, 215);
    assert_eq!(number::CLONE, 220);
    assert_eq!(number::EXECVE, 221);
    assert_eq!(number::MMAP, 222);
    assert_eq!(number::MPROTECT, 226);
    assert_eq!(number::WAIT4, 260);
    assert_eq!(number::PRLIMIT64, 261);
    assert_eq!(number::GETRANDOM, 278);
    assert_eq!(errno::EAGAIN, 11);
    assert_eq!(errno::EPIPE, 32);
    assert!(errno::is_error(errno::encode(errno::ENOSYS)));
    assert!(!errno::is_error(0));
}
