//! Linux 风格的 socket 层。
//!
//! 支持 AF_INET (IPv4/IPv6) 地址族。
//! TCP (SOCK_STREAM) 和 UDP (SOCK_DGRAM) socket 类型。
//!
//! Socket 通过全局 socket 表管理，key 为分配的 fd。
//! 每个 socket fd 由 `sys_socket()` 分配并写入当前进程的 fd 表。

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

use myos_vfs::{
    Errno, File, FileOperations, IoBuffer, MutableIoBuffer, OpenFlags, PollEvents, Stat,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::task::WaitQueue;

const SOCKET_LOCK: LockClass = LockClass::new("net.socket", LockRank::Vfs, 9);
const SOCKET_TABLE_LOCK: LockClass = LockClass::new("net.socket_table", LockRank::Vfs, 10);

// ---------------------------------------------------------------------------
// Socket 域 / 类型 / 协议常量
// ---------------------------------------------------------------------------

pub const AF_INET: usize = 2;
pub const AF_INET6: usize = 10;
pub const AF_UNIX: usize = 1;
pub const SOCK_STREAM: usize = 1;
pub const SOCK_DGRAM: usize = 2;
pub const SOCK_SEQPACKET: usize = 5;
pub const IPPROTO_TCP: usize = 6;
pub const IPPROTO_UDP: usize = 17;

// ---------------------------------------------------------------------------
// Socket 地址结构
// ---------------------------------------------------------------------------

/// `sockaddr_in` — IPv4 socket 地址。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SockAddrIn {
    pub sin_family: u16,
    pub sin_port: u16,
    pub sin_addr: [u8; 4],
    pub sin_zero: [u8; 8],
}

impl SockAddrIn {
    pub const fn new(addr: [u8; 4], port: u16) -> Self {
        Self {
            sin_family: AF_INET as u16,
            sin_port: port.to_be(),
            sin_addr: addr,
            sin_zero: [0; 8],
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(
                self as *const Self as *const u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    fn as_mut_bytes(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(
                self as *mut Self as *mut u8,
                core::mem::size_of::<Self>(),
            )
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < core::mem::size_of::<Self>() {
            return None;
        }
        let mut addr = Self {
            sin_family: 0,
            sin_port: 0,
            sin_addr: [0; 4],
            sin_zero: [0; 8],
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                addr.as_mut_bytes().as_mut_ptr(),
                core::mem::size_of::<Self>(),
            );
        }
        if addr.sin_family != AF_INET as u16 {
            return None;
        }
        Some(addr)
    }

    pub fn port(&self) -> u16 {
        u16::from_be(self.sin_port)
    }

    pub fn addr(&self) -> [u8; 4] {
        self.sin_addr
    }
}

// ---------------------------------------------------------------------------
// Socket 状态
// ---------------------------------------------------------------------------

enum SocketType {
    Tcp(TcpState),
    Udp(UdpState),
}

enum TcpState {
    Created,
    Bound {
        local: SockAddrIn,
    },
    Listening {
        local: SockAddrIn,
        backlog: usize,
        pending: Vec<usize>,
    },
    Connected {
        local: SockAddrIn,
        peer: SockAddrIn,
        peer_id: usize,
    },
}

enum UdpState {
    Created,
    Bound { local: SockAddrIn },
}

struct SocketInner {
    domain: usize,
    sock_type: usize,
    protocol: usize,
    state: SocketType,
    recv_buf: Vec<u8>,
    peer_closed: bool,
    nonblock: bool,
}

impl SocketInner {
    fn new(domain: usize, sock_type: usize, protocol: usize) -> Self {
        let state = match sock_type {
            SOCK_STREAM => SocketType::Tcp(TcpState::Created),
            SOCK_DGRAM => SocketType::Udp(UdpState::Created),
            _ => SocketType::Tcp(TcpState::Created),
        };
        Self {
            domain,
            sock_type,
            protocol,
            state,
            recv_buf: Vec::new(),
            peer_closed: false,
            nonblock: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Socket FileOperations
// ---------------------------------------------------------------------------

struct SocketFile {
    id: usize,
}

impl FileOperations for SocketFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        let current_thread = crate::task::current_user_thread();
        loop {
            if current_thread
                .as_deref()
                .is_some_and(thread_has_unblocked_signal)
            {
                return Err(Errno::Eintr);
            }
            let mut table = SOCKET_TABLE.lock();
            let inner = table.get_mut(&self.id).ok_or(Errno::Ebadf)?;

            if inner.recv_buf.is_empty() {
                if inner.peer_closed {
                    return Ok(0);
                }
                let can_wait = !file.flags().contains(OpenFlags::O_NONBLOCK) && !inner.nonblock;
                drop(table);
                if !can_wait || !scheduler_can_block_current() {
                    return Err(Errno::Eagain);
                }
                let _ = crate::task::block_current_on_if_from_user_trap(&SOCKET_IO_WAIT, || {
                    !current_thread
                        .as_deref()
                        .is_some_and(thread_has_unblocked_signal)
                });
                continue;
            }
            let copied = buf.push(&inner.recv_buf);
            let remaining = inner.recv_buf[copied..].to_vec();
            inner.recv_buf = remaining;
            trace_socket("read", self.id, copied);
            return Ok(copied);
        }
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        let mut table = SOCKET_TABLE.lock();
        let written = queue_to_peer(&mut table, self.id, buf.as_bytes())?;
        drop(table);
        SOCKET_IO_WAIT.wake_all();
        Ok(written)
    }

    fn poll(&self, _file: &File, requested: PollEvents) -> PollEvents {
        let table = SOCKET_TABLE.lock();
        let mut ready = PollEvents::empty();
        ready = ready.union(PollEvents::OUT);

        if let Some(inner) = table.get(&self.id) {
            if !inner.recv_buf.is_empty() || inner.peer_closed {
                ready = ready.union(PollEvents::IN);
            }
            if inner.peer_closed {
                ready = ready.union(PollEvents::HUP);
            }
        }
        ready.intersect(requested)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        let mut stat = Stat::zeroed();
        stat.mode = myos_vfs::FileMode::S_IFREG | 0o600;
        stat.nlink = 1;
        Ok(stat)
    }

    fn ioctl(&self, _file: &File, cmd: usize, arg: usize) -> Result<usize, Errno> {
        // FIONBIO: set/clear non-blocking mode on the socket.
        // FIONBIO = 0x5421 (asm-generic ioctl)
        const FIONBIO: usize = 0x5421;
        match cmd {
            FIONBIO => {
                if arg != 0 {
                    let process = crate::task::current_user_thread()
                        .map(|t| t.process_arc())
                        .ok_or(Errno::Einval)?;
                    let mut bytes = [0_u8; 4];
                    process
                        .mm()
                        .copy_from_user(arg, &mut bytes)
                        .map_err(|_| Errno::Efault)?;
                    let value = i32::from_ne_bytes(bytes);
                    if let Some(inner) = SOCKET_TABLE.lock().get_mut(&self.id) {
                        inner.nonblock = value != 0;
                    }
                }
                Ok(0)
            }
            _ => Err(Errno::Enotty),
        }
    }

    fn release(&self, _file: &File) {
        close_socket_id(self.id);
    }
}

// ---------------------------------------------------------------------------
// 全局 Socket 表
// ---------------------------------------------------------------------------

static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(1);
static NEXT_EPHEMERAL_PORT: AtomicUsize = AtomicUsize::new(49152);
static SOCKET_TABLE: IrqSpinLock<BTreeMap<usize, SocketInner>> =
    IrqSpinLock::new_with_class(BTreeMap::new(), SOCKET_TABLE_LOCK);
static SOCKET_ACCEPT_WAIT: WaitQueue = WaitQueue::new();
static SOCKET_IO_WAIT: WaitQueue = WaitQueue::new();

fn allocate_socket_id() -> usize {
    NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed)
}

fn allocate_ephemeral_port() -> u16 {
    let raw = NEXT_EPHEMERAL_PORT.fetch_add(1, Ordering::Relaxed);
    49152 + ((raw - 49152) % 16384) as u16
}

fn trace_socket(_event: &str, _sid: usize, _value: usize) {}

fn is_unspecified(addr: [u8; 4]) -> bool {
    addr == [0, 0, 0, 0]
}

fn is_loopback(addr: [u8; 4]) -> bool {
    addr[0] == 127 || is_unspecified(addr)
}

fn socket_matches_listener(local: SockAddrIn, dest: SockAddrIn) -> bool {
    local.port() == dest.port() && (is_unspecified(local.addr()) || local.addr() == dest.addr())
}

fn scheduler_can_block_current() -> bool {
    crate::task::scheduler_is_initialized() && crate::task::current_user_thread().is_some()
}

fn thread_has_unblocked_signal(thread: &crate::process::Thread) -> bool {
    thread.process().signals().pending() & !thread.blocked_signals() != 0
}

fn queue_to_peer(
    table: &mut BTreeMap<usize, SocketInner>,
    sid: usize,
    input: &[u8],
) -> Result<usize, Errno> {
    let peer_id = match table.get(&sid) {
        Some(inner) if inner.peer_closed => return Err(Errno::Epipe),
        Some(inner) => match &inner.state {
            SocketType::Tcp(TcpState::Connected { peer_id, .. }) => *peer_id,
            SocketType::Udp(_) => sid,
            _ => return Err(Errno::Einval),
        },
        None => return Err(Errno::Ebadf),
    };

    let peer = table.get_mut(&peer_id).ok_or(Errno::Epipe)?;
    if peer.peer_closed {
        return Err(Errno::Epipe);
    }
    peer.recv_buf.extend_from_slice(input);
    trace_socket("send", sid, input.len());
    Ok(input.len())
}

fn close_socket_id(id: usize) {
    let mut table = SOCKET_TABLE.lock();
    let peer_id = table.get(&id).and_then(|inner| match &inner.state {
        SocketType::Tcp(TcpState::Connected { peer_id, .. }) => Some(*peer_id),
        _ => None,
    });
    if let Some(peer_id) = peer_id {
        if let Some(peer) = table.get_mut(&peer_id) {
            peer.peer_closed = true;
        }
    }
    table.remove(&id);
    drop(table);
    SOCKET_ACCEPT_WAIT.wake_all();
    SOCKET_IO_WAIT.wake_all();
}

pub fn wake_all_waiters() {
    SOCKET_ACCEPT_WAIT.wake_all();
    SOCKET_IO_WAIT.wake_all();
}

fn copy_addr_from_user(ptr: usize) -> Result<SockAddrIn, isize> {
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Einval as isize)),
    };

    let mut bytes = [0_u8; 16];
    let addr_len = core::mem::size_of::<SockAddrIn>();
    if process
        .mm()
        .copy_from_user(ptr, &mut bytes[..addr_len])
        .is_err()
    {
        return Err(-(Errno::Efault as isize));
    }
    SockAddrIn::from_bytes(&bytes).ok_or(-(Errno::Einval as isize))
}

fn copy_to_user(ptr: usize, bytes: &[u8]) -> Result<(), isize> {
    if ptr == 0 {
        return Ok(());
    }
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Einval as isize)),
    };
    process
        .mm()
        .copy_to_user(ptr, bytes)
        .map_err(|_| -(Errno::Efault as isize))
}

fn copy_data_from_user(ptr: usize, len: usize, buf: &mut [u8]) -> Result<usize, isize> {
    let copy_len = len.min(buf.len());
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Einval as isize)),
    };
    process
        .mm()
        .copy_from_user(ptr, &mut buf[..copy_len])
        .map_err(|_| -(Errno::Efault as isize))?;
    Ok(copy_len)
}

fn copy_usize_from_user(ptr: usize) -> Result<usize, isize> {
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Einval as isize)),
    };
    let mut bytes = [0_u8; core::mem::size_of::<usize>()];
    process
        .mm()
        .copy_from_user(ptr, &mut bytes)
        .map_err(|_| -(Errno::Efault as isize))?;
    Ok(usize::from_ne_bytes(bytes))
}

fn copy_u32_from_user(ptr: usize) -> Result<u32, isize> {
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Einval as isize)),
    };
    let mut bytes = [0_u8; 4];
    process
        .mm()
        .copy_from_user(ptr, &mut bytes)
        .map_err(|_| -(Errno::Efault as isize))?;
    Ok(u32::from_ne_bytes(bytes))
}

fn copy_iov_from_user(iov_ptr: usize, index: usize) -> Result<(usize, usize), isize> {
    let base = copy_usize_from_user(iov_ptr + index * 2 * core::mem::size_of::<usize>())?;
    let len = copy_usize_from_user(
        iov_ptr + index * 2 * core::mem::size_of::<usize>() + core::mem::size_of::<usize>(),
    )?;
    Ok((base, len))
}

fn copy_socket_addr_to_user(addr_ptr: usize, addr_len_ptr: usize, addr: SockAddrIn) -> isize {
    if addr_ptr != 0 {
        let peer_bytes = addr.as_bytes();
        if copy_to_user(addr_ptr, peer_bytes).is_err() {
            return -(Errno::Efault as isize);
        }
    }
    if addr_len_ptr != 0 {
        let len = (core::mem::size_of::<SockAddrIn>() as u32).to_ne_bytes();
        if copy_to_user(addr_len_ptr, &len).is_err() {
            return -(Errno::Efault as isize);
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Socket 系统调用
// ---------------------------------------------------------------------------

/// 从 fd 获取 socket ID。通过解析 File 的 path 字段实现。
fn get_socket_id_from_fd(fd: usize) -> Result<usize, isize> {
    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return Err(-(Errno::Ebadf as isize)),
    };

    let file = match process.files().get(fd) {
        Ok(f) => f,
        Err(e) => return Err(e.to_isize()),
    };

    // Socket 文件通过 path 字段编码 "socket:<id>"
    match file.path() {
        Some(path) if path.starts_with("socket:") => match path[7..].parse::<usize>() {
            Ok(id) => Ok(id),
            Err(_) => Err(-(Errno::Ebadf as isize)),
        },
        _ => Err(-(Errno::Enotsock as isize)),
    }
}

/// socket(domain, type, protocol) → fd
pub fn sys_socket(domain: usize, sock_type: usize, protocol: usize) -> isize {
    const SOCK_CLOEXEC: usize = 0o2000000;
    const SOCK_NONBLOCK: usize = 0o0004000;

    if domain != AF_INET && domain != AF_INET6 {
        return -(Errno::Eafnosupport as isize);
    }
    let wants_cloexec = sock_type & SOCK_CLOEXEC != 0;
    let wants_nonblock = sock_type & SOCK_NONBLOCK != 0;
    let base_type = sock_type & !(SOCK_CLOEXEC | SOCK_NONBLOCK);
    if base_type != SOCK_STREAM && base_type != SOCK_DGRAM {
        return -(Errno::Einval as isize);
    }
    let protocol = if protocol == 0 {
        match base_type {
            SOCK_STREAM => IPPROTO_TCP,
            SOCK_DGRAM => IPPROTO_UDP,
            _ => return -(Errno::Einval as isize),
        }
    } else {
        protocol
    };

    let id = allocate_socket_id();
    let mut inner = SocketInner::new(domain, base_type, protocol);
    inner.nonblock = wants_nonblock;

    {
        let mut table = SOCKET_TABLE.lock();
        if table.contains_key(&id) {
            return -(Errno::Eexist as isize);
        }
        table.insert(id, inner);
    }

    let socket_ops = Arc::new(SocketFile { id });
    // 通过 path 字段编码 socket id 以便后续查找
    let path = alloc::format!("socket:{}", id);
    let file = File::new_with_path(OpenFlags::O_RDWR, path, socket_ops);

    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(e) => return e.to_isize(),
    };

    match process.files().allocate(file, wants_cloexec) {
        Ok(fd) => {
            trace_socket("socket", id, fd);
            fd as isize
        }
        Err(e) => e.to_isize(),
    }
}

/// socketpair(AF_UNIX, type, 0, sv) -> 0
///
/// Rust's process launcher uses a close-on-exec Unix seqpacket pair to report
/// an exec failure to its parent. The byte-stream backing is sufficient for
/// that protocol and also provides ordinary bidirectional socketpair I/O.
pub fn sys_socketpair(
    domain: usize,
    sock_type: usize,
    protocol: usize,
    pair_address: usize,
) -> isize {
    const SOCK_CLOEXEC: usize = 0o2000000;
    const SOCK_NONBLOCK: usize = 0o0004000;

    if domain != AF_UNIX || protocol != 0 || pair_address == 0 {
        return -(Errno::Einval as isize);
    }
    let wants_cloexec = sock_type & SOCK_CLOEXEC != 0;
    let wants_nonblock = sock_type & SOCK_NONBLOCK != 0;
    let base_type = sock_type & !(SOCK_CLOEXEC | SOCK_NONBLOCK);
    if base_type != SOCK_STREAM && base_type != SOCK_DGRAM && base_type != SOCK_SEQPACKET {
        return -(Errno::Einval as isize);
    }

    let first_id = allocate_socket_id();
    let second_id = allocate_socket_id();
    let local = SockAddrIn::new([127, 0, 0, 1], 0);
    let mut first = SocketInner::new(AF_UNIX, base_type, 0);
    let mut second = SocketInner::new(AF_UNIX, base_type, 0);
    first.nonblock = wants_nonblock;
    second.nonblock = wants_nonblock;
    first.state = SocketType::Tcp(TcpState::Connected {
        local,
        peer: local,
        peer_id: second_id,
    });
    second.state = SocketType::Tcp(TcpState::Connected {
        local,
        peer: local,
        peer_id: first_id,
    });
    {
        let mut table = SOCKET_TABLE.lock();
        table.insert(first_id, first);
        table.insert(second_id, second);
    }

    let process = match crate::task::current_user_thread().map(|thread| thread.process_arc()) {
        Some(process) => process,
        None => {
            close_socket_id(first_id);
            close_socket_id(second_id);
            return -(Errno::Einval as isize);
        }
    };
    let flags = if wants_nonblock {
        OpenFlags::O_RDWR.union(OpenFlags::O_NONBLOCK)
    } else {
        OpenFlags::O_RDWR
    };
    let first_file = File::new_with_path(
        flags,
        alloc::format!("socket:{}", first_id),
        Arc::new(SocketFile { id: first_id }),
    );
    let second_file = File::new_with_path(
        flags,
        alloc::format!("socket:{}", second_id),
        Arc::new(SocketFile { id: second_id }),
    );
    let first_fd = match process.files().allocate(first_file, wants_cloexec) {
        Ok(fd) => fd,
        Err(error) => {
            close_socket_id(first_id);
            close_socket_id(second_id);
            return error.to_isize();
        }
    };
    let second_fd = match process.files().allocate(second_file, wants_cloexec) {
        Ok(fd) => fd,
        Err(error) => {
            let _ = process.files().close(first_fd);
            close_socket_id(second_id);
            return error.to_isize();
        }
    };
    let pair = [first_fd as i32, second_fd as i32];
    let pair_bytes = unsafe {
        core::slice::from_raw_parts(pair.as_ptr().cast::<u8>(), core::mem::size_of_val(&pair))
    };
    if copy_to_user(pair_address, pair_bytes).is_err() {
        let _ = process.files().close(first_fd);
        let _ = process.files().close(second_fd);
        return -(Errno::Efault as isize);
    }
    crate::println!(
        "socketpair: pid={} type={:#x} fds={},{} ids={},{}",
        process.id().get(),
        sock_type,
        first_fd,
        second_fd,
        first_id,
        second_id,
    );
    0
}

/// bind(fd, addr, addrlen) → 0
pub fn sys_bind(fd: usize, addr_ptr: usize, addr_len: usize) -> isize {
    if addr_len as usize != core::mem::size_of::<SockAddrIn>() {
        return -(Errno::Einval as isize);
    }
    let addr = match copy_addr_from_user(addr_ptr) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let inner = match table.get_mut(&sid) {
        Some(s) => s,
        None => return -(Errno::Ebadf as isize),
    };

    match &inner.state {
        SocketType::Tcp(TcpState::Created) => {
            inner.state = SocketType::Tcp(TcpState::Bound { local: addr });
        }
        SocketType::Udp(UdpState::Created) => {
            inner.state = SocketType::Udp(UdpState::Bound { local: addr });
        }
        _ => return -(Errno::Einval as isize),
    }

    trace_socket("bind", sid, addr.port() as usize);
    0
}

/// listen(fd, backlog) → 0
pub fn sys_listen(fd: usize, backlog: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let inner = match table.get_mut(&sid) {
        Some(s) => s,
        None => return -(Errno::Ebadf as isize),
    };

    let local = match &inner.state {
        SocketType::Tcp(TcpState::Bound { local }) => *local,
        _ => return -(Errno::Einval as isize),
    };

    inner.state = SocketType::Tcp(TcpState::Listening {
        local,
        backlog,
        pending: Vec::new(),
    });
    trace_socket("listen", sid, backlog);
    0
}

/// accept(fd, addr, addrlen) → new_fd
pub fn sys_accept(fd: usize, addr_ptr: usize, addr_len_ptr: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let current_thread = crate::task::current_user_thread();

    let (accepted_id, _local, peer) = loop {
        if current_thread
            .as_deref()
            .is_some_and(thread_has_unblocked_signal)
        {
            return -(crate::syscall::errno::EINTR);
        }
        let mut table = SOCKET_TABLE.lock();
        let listener = match table.get_mut(&sid) {
            Some(s) => s,
            None => return -(Errno::Ebadf as isize),
        };
        match &mut listener.state {
            SocketType::Tcp(TcpState::Listening { pending, .. }) if !pending.is_empty() => {
                let accepted_id = pending.remove(0);
                match table.get(&accepted_id) {
                    Some(accepted) => match &accepted.state {
                        SocketType::Tcp(TcpState::Connected { local, peer, .. }) => {
                            break (accepted_id, *local, *peer);
                        }
                        _ => return -(Errno::Einval as isize),
                    },
                    None => continue,
                }
            }
            SocketType::Tcp(TcpState::Listening { .. }) => {
                let can_wait = !listener.nonblock;
                drop(table);
                if !can_wait || !scheduler_can_block_current() {
                    return -(Errno::Eagain as isize);
                }
                let _ =
                    crate::task::block_current_on_if_from_user_trap(&SOCKET_ACCEPT_WAIT, || {
                        if current_thread
                            .as_deref()
                            .is_some_and(thread_has_unblocked_signal)
                        {
                            return false;
                        }
                        let table = SOCKET_TABLE.lock();
                        table.get(&sid).is_some_and(|listener| {
                            matches!(
                                &listener.state,
                                SocketType::Tcp(TcpState::Listening { pending, .. })
                                    if pending.is_empty()
                            )
                        })
                    });
                continue;
            }
            _ => return -(Errno::Einval as isize),
        }
    };

    let socket_ops = Arc::new(SocketFile { id: accepted_id });
    let path = alloc::format!("socket:{}", accepted_id);
    let new_file = File::new_with_path(OpenFlags::O_RDWR, path, socket_ops);

    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(e) => return e.to_isize(),
    };

    let new_fd = match process.files().allocate(new_file, false) {
        Ok(fd) => fd,
        Err(e) => return e.to_isize(),
    };
    trace_socket("accept", accepted_id, new_fd);

    if addr_ptr != 0 {
        let peer_bytes = peer.as_bytes();
        let _ = copy_to_user(addr_ptr, peer_bytes);
    }
    if addr_len_ptr != 0 {
        let len = (core::mem::size_of::<SockAddrIn>() as u32).to_ne_bytes();
        let _ = copy_to_user(addr_len_ptr, &len);
    }

    new_fd as isize
}

pub fn sys_accept4(fd: usize, addr_ptr: usize, addr_len_ptr: usize, flags: usize) -> isize {
    const SOCK_CLOEXEC: usize = 0o2000000;
    const SOCK_NONBLOCK: usize = 0o0004000;
    if flags & !(SOCK_CLOEXEC | SOCK_NONBLOCK) != 0 {
        return -(Errno::Einval as isize);
    }
    sys_accept(fd, addr_ptr, addr_len_ptr)
}

pub fn sys_getsockname(fd: usize, addr_ptr: usize, addr_len_ptr: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let table = SOCKET_TABLE.lock();
    let addr = match table.get(&sid) {
        Some(inner) => match &inner.state {
            SocketType::Tcp(TcpState::Bound { local })
            | SocketType::Tcp(TcpState::Listening { local, .. })
            | SocketType::Tcp(TcpState::Connected { local, .. }) => *local,
            SocketType::Tcp(TcpState::Created) => SockAddrIn::new([0, 0, 0, 0], 0),
            SocketType::Udp(UdpState::Bound { local }) => *local,
            SocketType::Udp(UdpState::Created) => SockAddrIn::new([0, 0, 0, 0], 0),
        },
        None => return -(Errno::Ebadf as isize),
    };
    drop(table);
    copy_socket_addr_to_user(addr_ptr, addr_len_ptr, addr)
}

pub fn sys_getpeername(fd: usize, addr_ptr: usize, addr_len_ptr: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let table = SOCKET_TABLE.lock();
    let addr = match table.get(&sid) {
        Some(inner) => match &inner.state {
            SocketType::Tcp(TcpState::Connected { peer, .. }) => *peer,
            _ => return -(Errno::Einval as isize),
        },
        None => return -(Errno::Ebadf as isize),
    };
    drop(table);
    copy_socket_addr_to_user(addr_ptr, addr_len_ptr, addr)
}

/// connect(fd, addr, addrlen) → 0
pub fn sys_connect(fd: usize, addr_ptr: usize, addr_len: usize) -> isize {
    if addr_len as usize != core::mem::size_of::<SockAddrIn>() {
        return -(Errno::Einval as isize);
    }
    let addr = match copy_addr_from_user(addr_ptr) {
        Ok(a) => a,
        Err(e) => return e,
    };

    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let local = match table.get(&sid) {
        Some(inner) => match &inner.state {
            SocketType::Tcp(TcpState::Created) => {
                SockAddrIn::new([127, 0, 0, 1], allocate_ephemeral_port())
            }
            SocketType::Tcp(TcpState::Bound { local }) => {
                let mut local = *local;
                if local.port() == 0 {
                    local.sin_port = allocate_ephemeral_port().to_be();
                }
                if is_unspecified(local.addr()) {
                    local.sin_addr = [127, 0, 0, 1];
                }
                local
            }
            SocketType::Tcp(TcpState::Connected { .. }) => return 0,
            _ => return -(Errno::Einval as isize),
        },
        None => return -(Errno::Ebadf as isize),
    };

    if !is_loopback(addr.addr()) {
        return -(Errno::Eafnosupport as isize);
    }

    let mut listener_id = None;
    for (id, candidate) in table.iter() {
        if let SocketType::Tcp(TcpState::Listening {
            local,
            pending,
            backlog,
        }) = &candidate.state
            && socket_matches_listener(*local, addr)
            && pending.len() < (*backlog).max(1)
        {
            listener_id = Some(*id);
            break;
        }
    }
    let listener_id = match listener_id {
        Some(id) => id,
        None => return -(Errno::Eagain as isize),
    };

    let accepted_id = allocate_socket_id();
    let mut accepted = SocketInner::new(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    accepted.state = SocketType::Tcp(TcpState::Connected {
        local: addr,
        peer: local,
        peer_id: sid,
    });

    if table.contains_key(&accepted_id) {
        return -(Errno::Eexist as isize);
    }
    table.insert(accepted_id, accepted);

    if let Some(client) = table.get_mut(&sid) {
        client.state = SocketType::Tcp(TcpState::Connected {
            local,
            peer: addr,
            peer_id: accepted_id,
        });
    } else {
        table.remove(&accepted_id);
        return -(Errno::Ebadf as isize);
    }

    if let Some(listener) = table.get_mut(&listener_id) {
        if let SocketType::Tcp(TcpState::Listening { pending, .. }) = &mut listener.state {
            pending.push(accepted_id);
        }
    }
    trace_socket("connect", sid, addr.port() as usize);
    drop(table);
    SOCKET_ACCEPT_WAIT.wake_all();
    0
}

/// sendto(fd, buf, len, flags, dest_addr, addrlen) → bytes_sent
pub fn sys_sendto(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    flags: usize,
    _dest_addr_ptr: usize,
    _addr_len: usize,
) -> isize {
    // Validate flags: MSG_DONTWAIT(0x40), MSG_NOSIGNAL(0x4000),
    // MSG_DONTROUTE(0x4) are accepted.
    const MSG_DONTWAIT: usize = 0x40;
    const MSG_NOSIGNAL: usize = 0x4000;
    const MSG_DONTROUTE: usize = 0x4;
    if flags & !(MSG_DONTWAIT | MSG_NOSIGNAL | MSG_DONTROUTE) != 0 {
        return -(Errno::Einval as isize);
    }
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut data = [0_u8; 4096];
    let copied = match copy_data_from_user(buf_ptr, buf_len, &mut data) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let mut table = SOCKET_TABLE.lock();
    let sent = match queue_to_peer(&mut table, sid, &data[..copied]) {
        Ok(sent) => sent,
        Err(errno) => return errno.to_isize(),
    };
    drop(table);
    SOCKET_IO_WAIT.wake_all();
    sent as isize
}

/// recvfrom(fd, buf, len, flags, src_addr, addrlen) → bytes_received
pub fn sys_recvfrom(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    flags: usize,
    _src_addr_ptr: usize,
    _addr_len_ptr: usize,
) -> isize {
    // Validate flags: MSG_DONTWAIT(0x40), MSG_PEEK(0x2),
    // MSG_CMSG_CLOEXEC(0x40000000) are accepted.
    const MSG_DONTWAIT: usize = 0x40;
    const MSG_PEEK: usize = 0x2;
    const MSG_CMSG_CLOEXEC: usize = 0x4000_0000;
    if flags & !(MSG_DONTWAIT | MSG_PEEK | MSG_CMSG_CLOEXEC) != 0 {
        return -(Errno::Einval as isize);
    }
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let data = loop {
        if crate::task::current_user_thread()
            .as_deref()
            .is_some_and(thread_has_unblocked_signal)
        {
            return -(crate::syscall::errno::EINTR);
        }
        let mut table = SOCKET_TABLE.lock();
        let inner = match table.get_mut(&sid) {
            Some(s) => s,
            None => return -(Errno::Ebadf as isize),
        };

        if inner.recv_buf.is_empty() {
            if inner.peer_closed {
                return 0;
            }
            let can_wait = flags & MSG_DONTWAIT == 0 && !inner.nonblock;
            drop(table);
            if !can_wait || !scheduler_can_block_current() {
                return -(Errno::Eagain as isize);
            }
            let current_thread = crate::task::current_user_thread();
            let _ = crate::task::block_current_on_if_from_user_trap(&SOCKET_IO_WAIT, || {
                !current_thread
                    .as_deref()
                    .is_some_and(thread_has_unblocked_signal)
            });
            continue;
        }

        let copy_len = buf_len.min(inner.recv_buf.len());
        let data = inner.recv_buf[..copy_len].to_vec();
        if flags & MSG_PEEK == 0 {
            let remaining = inner.recv_buf[copy_len..].to_vec();
            inner.recv_buf = remaining;
        }
        trace_socket("recvfrom", sid, copy_len);
        break data;
    };

    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return -(Errno::Einval as isize),
    };

    if process.mm().copy_to_user(buf_ptr, &data).is_err() {
        return -(Errno::Efault as isize);
    }

    data.len() as isize
}

pub fn sys_sendmsg(fd: usize, msg_ptr: usize, flags: usize) -> isize {
    const MSG_DONTWAIT: usize = 0x40;
    const MSG_NOSIGNAL: usize = 0x4000;
    const MSG_DONTROUTE: usize = 0x4;
    if flags & !(MSG_DONTWAIT | MSG_NOSIGNAL | MSG_DONTROUTE) != 0 {
        return -(Errno::Einval as isize);
    }
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let iov_ptr = match copy_usize_from_user(msg_ptr + 16) {
        Ok(value) => value,
        Err(errno) => return errno,
    };
    let iov_len = match copy_usize_from_user(msg_ptr + 24) {
        Ok(value) => value,
        Err(errno) => return errno,
    };
    if iov_len > 16 {
        return -(Errno::Einval as isize);
    }

    let mut total = 0_usize;
    for index in 0..iov_len {
        let (base, len) = match copy_iov_from_user(iov_ptr, index) {
            Ok(value) => value,
            Err(errno) => return if total != 0 { total as isize } else { errno },
        };
        let mut done = 0_usize;
        while done < len {
            let chunk = (len - done).min(4096);
            let mut data = [0_u8; 4096];
            let copied = match copy_data_from_user(base + done, chunk, &mut data) {
                Ok(copied) => copied,
                Err(errno) => return if total != 0 { total as isize } else { errno },
            };
            let mut table = SOCKET_TABLE.lock();
            match queue_to_peer(&mut table, sid, &data[..copied]) {
                Ok(sent) => {
                    total += sent;
                    done += sent;
                }
                Err(errno) => {
                    return if total != 0 {
                        total as isize
                    } else {
                        errno.to_isize()
                    };
                }
            }
            drop(table);
            SOCKET_IO_WAIT.wake_all();
            if copied == 0 {
                break;
            }
        }
    }
    total as isize
}

pub fn sys_recvmsg(fd: usize, msg_ptr: usize, flags: usize) -> isize {
    const MSG_DONTWAIT: usize = 0x40;
    const MSG_PEEK: usize = 0x2;
    const MSG_CMSG_CLOEXEC: usize = 0x4000_0000;
    if flags & !(MSG_DONTWAIT | MSG_PEEK | MSG_CMSG_CLOEXEC) != 0 {
        return -(Errno::Einval as isize);
    }
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };
    let name_ptr = match copy_usize_from_user(msg_ptr) {
        Ok(value) => value,
        Err(errno) => return errno,
    };
    let name_len = match copy_u32_from_user(msg_ptr + 8) {
        Ok(value) => value as usize,
        Err(errno) => return errno,
    };
    let iov_ptr = match copy_usize_from_user(msg_ptr + 16) {
        Ok(value) => value,
        Err(errno) => return errno,
    };
    let iov_len = match copy_usize_from_user(msg_ptr + 24) {
        Ok(value) => value,
        Err(errno) => return errno,
    };
    if iov_len > 16 {
        return -(Errno::Einval as isize);
    }

    let (data, peer) = loop {
        if crate::task::current_user_thread()
            .as_deref()
            .is_some_and(thread_has_unblocked_signal)
        {
            return -(crate::syscall::errno::EINTR);
        }
        let mut table = SOCKET_TABLE.lock();
        let inner = match table.get_mut(&sid) {
            Some(s) => s,
            None => return -(Errno::Ebadf as isize),
        };
        if inner.recv_buf.is_empty() {
            if inner.peer_closed {
                return 0;
            }
            let can_wait = flags & MSG_DONTWAIT == 0 && !inner.nonblock;
            drop(table);
            if !can_wait || !scheduler_can_block_current() {
                return -(Errno::Eagain as isize);
            }
            let current_thread = crate::task::current_user_thread();
            let _ = crate::task::block_current_on_if_from_user_trap(&SOCKET_IO_WAIT, || {
                !current_thread
                    .as_deref()
                    .is_some_and(thread_has_unblocked_signal)
            });
            continue;
        }
        let total_capacity = (0..iov_len).fold(0_usize, |sum, index| {
            sum.saturating_add(
                copy_iov_from_user(iov_ptr, index)
                    .map(|(_, len)| len)
                    .unwrap_or(0),
            )
        });
        let copy_len = total_capacity.min(inner.recv_buf.len());
        let data = inner.recv_buf[..copy_len].to_vec();
        let peer = match &inner.state {
            SocketType::Tcp(TcpState::Connected { peer, .. }) => *peer,
            _ => SockAddrIn::new([127, 0, 0, 1], 0),
        };
        if flags & MSG_PEEK == 0 {
            let remaining = inner.recv_buf[copy_len..].to_vec();
            inner.recv_buf = remaining;
        }
        trace_socket("recvmsg", sid, copy_len);
        break (data, peer);
    };

    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return -(Errno::Einval as isize),
    };
    let mut written = 0_usize;
    for index in 0..iov_len {
        if written >= data.len() {
            break;
        }
        let (base, len) = match copy_iov_from_user(iov_ptr, index) {
            Ok(value) => value,
            Err(errno) => {
                return if written != 0 {
                    written as isize
                } else {
                    errno
                };
            }
        };
        let chunk = len.min(data.len() - written);
        if process
            .mm()
            .copy_to_user(base, &data[written..written + chunk])
            .is_err()
        {
            return if written != 0 {
                written as isize
            } else {
                -(Errno::Efault as isize)
            };
        }
        written += chunk;
    }
    if name_ptr != 0 && name_len >= core::mem::size_of::<SockAddrIn>() {
        let _ = copy_to_user(name_ptr, peer.as_bytes());
    }
    written as isize
}

/// shutdown(fd, how) → 0
pub fn sys_shutdown(fd: usize, _how: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let peer_id = match table.get(&sid) {
        Some(inner) => match &inner.state {
            SocketType::Tcp(TcpState::Connected { peer_id, .. }) => Some(*peer_id),
            _ => None,
        },
        None => return -(Errno::Ebadf as isize),
    };
    if let Some(peer_id) = peer_id {
        if let Some(peer) = table.get_mut(&peer_id) {
            peer.peer_closed = true;
        }
    }
    if let Some(inner) = table.get_mut(&sid) {
        inner.peer_closed = true;
    }
    drop(table);
    SOCKET_IO_WAIT.wake_all();
    0
}
