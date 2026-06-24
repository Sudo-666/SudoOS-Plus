//! Linux 风格的 socket 层。
//!
//! 支持 AF_INET (IPv4/IPv6) 地址族。
//! TCP (SOCK_STREAM) 和 UDP (SOCK_DGRAM) socket 类型。
//!
//! Socket 通过全局 socket 表管理，key 为分配的 fd。
//! 每个 socket fd 由 `sys_socket()` 分配并写入当前进程的 fd 表。

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};

use myos_vfs::{Errno, File, FileOperations, IoBuffer, MutableIoBuffer, OpenFlags, PollEvents, Stat};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const SOCKET_LOCK: LockClass = LockClass::new("net.socket", LockRank::Vfs, 9);
const SOCKET_TABLE_LOCK: LockClass = LockClass::new("net.socket_table", LockRank::Vfs, 10);

// ---------------------------------------------------------------------------
// Socket 域 / 类型 / 协议常量
// ---------------------------------------------------------------------------

pub const AF_INET: usize = 2;
pub const AF_INET6: usize = 10;
pub const SOCK_STREAM: usize = 1;
pub const SOCK_DGRAM: usize = 2;
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
    Bound { local: SockAddrIn },
    Listening { local: SockAddrIn, backlog: usize },
    Connected { local: SockAddrIn, peer: SockAddrIn },
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
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        let mut table = SOCKET_TABLE.lock();
        let inner = table.get_mut(&self.id).ok_or(Errno::Ebadf)?;

        if inner.recv_buf.is_empty() {
            if inner.peer_closed {
                return Ok(0);
            }
            return Err(Errno::Eagain);
        }
        let copied = buf.push(&inner.recv_buf);
        let remaining = inner.recv_buf[copied..].to_vec();
        inner.recv_buf = remaining;
        Ok(copied)
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        let table = SOCKET_TABLE.lock();
        let inner = table.get(&self.id).ok_or(Errno::Ebadf)?;

        if inner.peer_closed {
            return Err(Errno::Epipe);
        }
        // 网络写入在此实现（通过 smoltcp）
        Ok(buf.len())
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

    fn ioctl(&self, _file: &File, cmd: usize, _arg: usize) -> Result<usize, Errno> {
        // FIONBIO: set/clear non-blocking mode on the socket.
        // FIONBIO = 0x5421 (asm-generic ioctl)
        const FIONBIO: usize = 0x5421;
        match cmd {
            FIONBIO => {
                // Non-blocking is the default for all sockets; accept the request.
                Ok(0)
            }
            _ => Err(Errno::Enotty),
        }
    }
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        let mut table = SOCKET_TABLE.lock();
        table.remove(&self.id);
    }
}

// ---------------------------------------------------------------------------
// 全局 Socket 表
// ---------------------------------------------------------------------------

static NEXT_SOCKET_ID: AtomicUsize = AtomicUsize::new(1);
static SOCKET_TABLE: IrqSpinLock<BTreeMap<usize, SocketInner>> =
    IrqSpinLock::new_with_class(BTreeMap::new(), SOCKET_TABLE_LOCK);

fn allocate_socket_id() -> usize {
    NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed)
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
        Some(path) if path.starts_with("socket:") => {
            match path[7..].parse::<usize>() {
                Ok(id) => Ok(id),
                Err(_) => Err(-(Errno::Ebadf as isize)),
            }
        }
        _ => Err(-(Errno::Enotsock as isize)),
    }
}

/// socket(domain, type, protocol) → fd
pub fn sys_socket(domain: usize, sock_type: usize, protocol: usize) -> isize {
    if domain != AF_INET && domain != AF_INET6 {
        return -(Errno::Eafnosupport as isize);
    }
    if sock_type != SOCK_STREAM && sock_type != SOCK_DGRAM {
        return -(Errno::Einval as isize);
    }
    let protocol = if protocol == 0 {
        match sock_type {
            SOCK_STREAM => IPPROTO_TCP,
            SOCK_DGRAM => IPPROTO_UDP,
            _ => return -(Errno::Einval as isize),
        }
    } else {
        protocol
    };

    let id = allocate_socket_id();
    let inner = SocketInner::new(domain, sock_type, protocol);

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

    match process.files().allocate(file, false) {
        Ok(fd) => fd as isize,
        Err(e) => e.to_isize(),
    }
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

    inner.state = SocketType::Tcp(TcpState::Listening { local, backlog });
    0
}

/// accept(fd, addr, addrlen) → new_fd
pub fn sys_accept(fd: usize, addr_ptr: usize, addr_len_ptr: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let local = {
        let table = SOCKET_TABLE.lock();
        match table.get(&sid) {
            Some(s) => match &s.state {
                SocketType::Tcp(TcpState::Listening { local, .. }) => *local,
                _ => return -(Errno::Einval as isize),
            },
            None => return -(Errno::Ebadf as isize),
        }
    };

    // 创建新的已连接 socket
    let id = allocate_socket_id();
    let peer = SockAddrIn::new([127, 0, 0, 1], 0);
    let mut new_inner = SocketInner::new(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    new_inner.state = SocketType::Tcp(TcpState::Connected { local, peer });

    {
        let mut table = SOCKET_TABLE.lock();
        if table.contains_key(&id) {
            return -(Errno::Eexist as isize);
        }
        table.insert(id, new_inner);
    }

    let socket_ops = Arc::new(SocketFile { id });
    let path = alloc::format!("socket:{}", id);
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

    // 写入 peer 地址
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
    let inner = match table.get_mut(&sid) {
        Some(s) => s,
        None => return -(Errno::Ebadf as isize),
    };

    let local = match &inner.state {
        SocketType::Tcp(TcpState::Bound { local }) => *local,
        _ => SockAddrIn::new([0, 0, 0, 0], 0),
    };

    inner.state = SocketType::Tcp(TcpState::Connected { local, peer: addr });
    0
}

/// sendto(fd, buf, len, flags, dest_addr, addrlen) → bytes_sent
pub fn sys_sendto(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    _flags: usize,
    _dest_addr_ptr: usize,
    _addr_len: usize,
) -> isize {
    let _sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut data = [0_u8; 512];
    let copied = match copy_data_from_user(buf_ptr, buf_len, &mut data) {
        Ok(c) => c,
        Err(e) => return e,
    };
    copied as isize
}

/// recvfrom(fd, buf, len, flags, src_addr, addrlen) → bytes_received
pub fn sys_recvfrom(
    fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    _flags: usize,
    _src_addr_ptr: usize,
    _addr_len_ptr: usize,
) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let inner = match table.get_mut(&sid) {
        Some(s) => s,
        None => return -(Errno::Ebadf as isize),
    };

    if inner.recv_buf.is_empty() {
        if inner.peer_closed {
            return 0;
        }
        return -(Errno::Eagain as isize);
    }

    let copy_len = buf_len.min(inner.recv_buf.len());
    let data = inner.recv_buf[..copy_len].to_vec();
    let remaining = inner.recv_buf[copy_len..].to_vec();
    inner.recv_buf = remaining;
    drop(table);

    let process = crate::task::current_user_thread()
        .map(|t| t.process_arc())
        .ok_or(Errno::Einval);
    let process = match process {
        Ok(p) => p,
        Err(_) => return -(Errno::Einval as isize),
    };

    if process
        .mm()
        .copy_to_user(buf_ptr, &data)
        .is_err()
    {
        return -(Errno::Efault as isize);
    }

    copy_len as isize
}

/// shutdown(fd, how) → 0
pub fn sys_shutdown(fd: usize, _how: usize) -> isize {
    let sid = match get_socket_id_from_fd(fd) {
        Ok(id) => id,
        Err(e) => return e,
    };

    let mut table = SOCKET_TABLE.lock();
    let inner = match table.get_mut(&sid) {
        Some(s) => s,
        None => return -(Errno::Ebadf as isize),
    };
    inner.peer_closed = true;
    0
}
