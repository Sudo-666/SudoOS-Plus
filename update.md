# SudoOS M16 更新日志

## 概述

本次更新按照 Linux 内核架构模式，补全了设备驱动模型、VirtIO 设备驱动、网络栈、procfs/sysfs/devpts 文件系统、RNG/RTC 子系统、socket 系统调用，以及竞赛评测所需的动态 ext4 测试扫描和关键 syscall (futex)。

---

## 提交历史

| 提交 | 说明 |
|------|------|
| `672525d` | M16: 设备驱动模型 + RNG + RTC + procfs + sysfs + net + socket + devpts |
| `74d8d6b` | 动态 ext4 根目录扫描，替换硬编码测试路径 |
| `c8da176` | futex syscall + 全量 ext4 文件安装 + 环境变量补全 |

---

## 新增文件

### 设备驱动

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/device.rs` | ~250 | Linux 风格 bus/device/driver 设备模型框架 |

**核心类型：**
- `DeviceType` — Block / Net / Rng / Rtc / Console / Input / Gpu / Sound / Socket
- `Device` — 设备结构体 (name, device_type, resources, compatible, driver, private_data)
- `Driver` trait — probe / remove / device_type 生命周期
- `Bus` trait — match_driver / on_device_added
- 全局注册表：`DEVICES`, `DRIVERS`, `BUSES`

**API：** `register_device()` / `register_driver()` / `register_bus()` / `find_devices_by_type()` / `find_device()` / `for_each_device()`

### RNG 子系统

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/rng.rs` | ~250 | ChaCha20 DRBG + VirtIO-RNG 硬件熵源 |

- 自包含 ChaCha20（80 行，无外部依赖）：256-bit key + 64-bit nonce + 20 轮
- `EntropyPool` — 支持硬件熵播种和退化播种（`time::now()` / `timer_ticks()` / 栈地址）
- 每 1 MiB 触发重播种信号
- 公共 API：`fill_random(buf)` / `fill_random_blocking(buf)` / `register_hardware_source(source)`

### RTC 子系统

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/rtc.rs` | ~100 | RTC 实时时钟 |

- `RtcTime` / `RtcHardware` trait
- `read_rtc_time()` — 有硬件用硬件，否则退化到单调时间
- `register_rtc()` — VirtIO-RTC 设备注册

### procfs 文件系统

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/procfs.rs` | ~140 | /proc 虚拟文件系统 |

- `ProcFileGenerator` trait — `generate() -> Vec<u8>` 动态生成
- `/proc/version` — `SudoOS 0.16 (M16)`
- `/proc/cpuinfo` — CPU 数量和架构
- `/proc/meminfo` — MemTotal / MemFree / PageSize
- `/proc/uptime` — 系统运行秒数
- `/proc/mounts` — 挂载表
- `/proc/self` — 指向当前 PID 的符号链接

### sysfs 文件系统

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/sysfs.rs` | ~140 | /sys 虚拟文件系统 |

**目录结构：**
```
/sys/kernel/   — version, ostype
/sys/devices/  — list (已注册设备)
/sys/class/    — block, net (设备分类)
```

### 网络子系统

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/net/mod.rs` | ~100 | NetDevice trait + 接口注册表 |
| `kernel/src/net/socket.rs` | ~450 | Socket 层 (AF_INET/TCP/UDP) + 8 syscall |
| `kernel/src/net/virtio_net.rs` | ~90 | VirtIO-Net 驱动 (MMIO + PCI) |

**NetDevice trait：** `mac_address()` / `mtu()` / `transmit()` / `receive()` / `poll_receive()`

**Socket 支持：**
- 域：AF_INET(2)、AF_INET6(10)，类型：SOCK_STREAM(1)、SOCK_DGRAM(2)
- 协议：IPPROTO_TCP(6)、IPPROTO_UDP(17)
- 全局 Socket 表 (BTreeMap)，通过 File path `"socket:<id>"` 实现 fd→socket 关联

**VirtIO-Net：** 封装 `VirtIONetRaw<SudoHal, T, 64>`，`from_raw()` 工厂函数

### devpts 伪终端

| 文件 | 行数 | 说明 |
|------|------|------|
| `kernel/src/devpts.rs` | ~360 | PTY master/slave 对 |

- `PtyMaster` / `PtySlave` — 独立 FileOperations
- 环形缓冲区 (4096B) 双向转发
- 阻塞 I/O + poll 支持 (IN/OUT/HUP/ERR)
- `/dev/ptmx` — 每次 open 创建新 PTY 对
- `/dev/pts/` — slave 目录
- `safe_wake()` — scheduler 初始化前安全唤醒

---

## 修改文件

### `kernel/src/main.rs`
- 新增模块：`device`, `rng`, `procfs`, `sysfs`, `net`, `rtc`, `devpts`
- 初始化调用：`device::initialize()`, `rng::initialize()`, `net::initialize()`, `rtc::initialize()`
- 挂载：`mount_proc()` → `/proc`, `mount_sys()` → `/sys`
- `mount_sdcard_if_present()` — 重构为动态 ext4 根目录扫描
  - 验证 ext4 superblock (magic 0xef53)
  - 调用 `ext4::list_root_directory()` 列出根目录条目
  - 查找 busybox（多种候选名）
  - 收集所有 `*_testcode.sh` 脚本存入 `SCANNED_TEST_SCRIPTS`
- 新增 `pub(crate) static SCANNED_TEST_SCRIPTS` — 动态扫描结果

### `kernel/src/fs/mod.rs`
- `DeviceKind` 扩展：`Random`, `Urandom`, `Ptmx`, `Rtc`
- `NodeState` 扩展：`ProcFile(Arc<dyn ProcFileGenerator>)`
- `MountFsType` 扩展：`Sysfs`
- `initialize()` 创建 `/dev/random`, `/dev/urandom`, `/dev/rtc`, `/dev/ptmx`, `/dev/pts/`
- `open()` Ptmx 拦截 — 每次 open 创建新 PTY 对
- `mount()` 支持 `Proc` 和 `Sysfs` 挂载类型
- `populate_proc_root()` / `populate_sysfs_root()` — 嵌套目录构建
- `format_mounts()` — 挂载表序列化
- `RegularFile` — read/write/seek/fstat/truncate 支持 ProcFile 节点
- `DeviceFile` — read/write/poll/ioctl 覆盖所有新 DeviceKind

### `kernel/src/virtio.rs`
- `probe_mmio_region()` — 增加 EntropySource (RNG) 和 Network 设备
- `probe_pci_host()` — 增加 EntropySource 和 Network PCI 设备
- 新增 `probe_pci_rng()` / `probe_pci_net()` 函数

### `kernel/src/ext4.rs`
- 新增 `Ext4DirEntry` 结构体（轻量级，仅 name/ino/file_type）
- 新增 `list_root_directory()` — 列出根目录条目，不递归加载
- 新增 `read_root_dir_entries()` — 解析目录块，跳过 . / ..

### `kernel/src/user.rs`
- `sys_getrandom()` — LCG → ChaCha20 DRBG
- 新增 8 个 socket syscall 常量和 dispatch
- 新增 `SYS_FUTEX` + `SYS_MKNODAT` 常量和 dispatch
- `sys_futex()` — FUTEX_WAIT (WaitQueue 阻塞) + FUTEX_WAKE (wake_all)
- `sys_mknodat()` — stub 返回 ENOSYS
- `verify_sdcard_all_scripts()` — 重写为使用 `SCANNED_TEST_SCRIPTS` 动态扫描
- `verify_sdcard_all_scripts_thread()` — 全量安装 ext4 根目录文件，设置完整环境变量
- `verify_sdcard_sample_thread()` — 文件缺失时优雅跳过

### `kernel/src/syscall.rs`
- Socket 编号：SOCKET(198)/BIND(200)/LISTEN(201)/ACCEPT(202)/CONNECT(203)/GETSOCKNAME(204)/GETPEERNAME(205)/SENDTO(206)/RECVFROM(207)/SETSOCKOPT(208)/GETSOCKOPT(209)/SHUTDOWN(210)
- 新增：FUTEX(98)/MKNODAT(33)

### `vfs/src/lib.rs`
- Errno 新增：`Eafnosupport(97)`, `Enotsock(88)`

### `kernel/Cargo.toml`
- 新增 `smoltcp v0.11`（alloc, medium-ethernet, medium-ip, proto-ipv4/ipv6, socket-tcp/udp/raw）

---

## Syscall 实现状态

### 已实现 (80+ 个)

| 分类 | Syscall |
|------|---------|
| 文件 I/O | read, write, readv, writev, pread64, openat, close, lseek |
| 文件系统 | stat, fstat, newfstatat, statx, getdents64, mkdirat, unlinkat, renameat, symlinkat, linkat, readlinkat, ftruncate, getcwd, chdir, faccessat, fsync |
| 挂载 | mount, umount2 |
| 进程 | clone, execve, exit, exit_group, wait4, getpid, getppid, gettid, getuid, geteuid, getgid, getegid, setsid, setpgid, getpgid, getsid, set_tid_address, set_robust_list |
| 信号 | kill, tkill, tgkill, rt_sigaction, rt_sigprocmask, rt_sigreturn, rt_sigtimedwait |
| 内存 | mmap, munmap, mprotect, brk |
| 时间 | nanosleep, clock_gettime, gettimeofday, times |
| 管道 | pipe2, dup, dup3, fcntl |
| I/O 复用 | ppoll, pselect6 |
| Socket | socket, bind, listen, accept, connect, sendto, recvfrom, shutdown |
| 同步 | **futex** (FUTEX_WAIT/WAKE) |
| 其他 | getrandom (ChaCha20), uname, sysinfo, prlimit64, sched_yield, ioctl |

### 已定义未 dispatch

getsockname(204), getpeername(205), setsockopt(208), getsockopt(209)

### 返回 ENOSYS

mknodat(33), epoll_create1(20), epoll_ctl(21), epoll_pwait(22), sendmsg(211), recvmsg(212),
eventfd, timerfd, signalfd, inotify, futex 高级操作 (requeue/PI)

---

## 缺页中断分析

### 处理流程

```
trap.rs → 用户态? → user::handle_fault()
                    → resolve_user_fault()
                       ├─ MapAnonymous   → 分配零页 ✅
                       ├─ GrowStack     → 栈增长 ✅
                       ├─ LoadFile      → BSS 零填 (eager load 已处理) ✅
                       ├─ CopyOnWrite   → Fatal (-EFAULT) ⚠️ 不会触发
                       ├─ Protection    → Fatal (-EFAULT)
                       └─ Segfault      → Fatal (-EFAULT)
                 → 内核态? → handle_page_fault() → panic!
```

### COW 分析

- `fork_clone_eager()` — 全量拷贝父进程所有物理页面，不产生 COW
- `mmap` file-backed — eager copy 文件数据，不用 COW
- `CopyOnWriteUnsupported` — 仅当 VMA flags 含 `COPY_ON_WRITE` 且 write fault on present page 时触发，当前从不设置该标志

**结论：缺页中断处理逻辑正确。** 竞赛测试超时原因不在缺页，而在硬编码路径和缺失 futex syscall（已在 `74d8d6b` / `c8da176` 修复）。

---

## 竞赛评测适配

### 动态 ext4 测试扫描

旧代码硬编码 `/musl/busybox` 等路径，竞赛磁盘布局不同导致全部跳过。

**新流程：**
1. 验证 ext4 superblock magic (0xef53 at offset 1080)
2. `ext4::list_root_directory()` 轻量级列出根目录
3. 收集所有 `*_testcode.sh` → `SCANNED_TEST_SCRIPTS`
4. 安装**全部**根目录文件到 VFS
5. 查找 busybox（`/bin/busybox` → `/busybox` → `/bin/sh`）
6. 依次执行每个脚本，输出 `#### OS COMP TEST GROUP START/END xxx ####` 标记

### 环境变量

```
PATH=.:/:/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin
LD_LIBRARY_PATH=.:/:/lib:/usr/lib:/usr/local/lib
HOME=/
```

### 目录创建

`/var`, `/var/tmp`, `/tmp`, `/dev`, `/proc`, `/sys`, `/etc`

---

## 新增 /dev 设备

| 设备 | 说明 |
|------|------|
| `/dev/random` | 阻塞式随机数 (ChaCha20 + VirtIO-RNG) |
| `/dev/urandom` | 非阻塞随机数 |
| `/dev/rtc` | 实时时钟 (VirtIO-RTC 或单调时间退化) |
| `/dev/ptmx` | PTY master 多路复用器 |
| `/dev/pts/` | PTY slave 目录 |

## 新增挂载点

| 挂载点 | 内容 |
|--------|------|
| `/proc` | version, cpuinfo, meminfo, uptime, mounts, self |
| `/sys` | kernel/, devices/, class/ |

---

## 测试结果

### 双架构启动

| 架构 | 镜像 | 结果 |
|------|------|------|
| RISC-V | alpine-linux-riscv64-ext4fs.img (690MB) | SMOKE_TEST: PASS ✅ |
| LoongArch | alpine-linux-loongarch64-ext4fs.img (690MB) | SMOKE_TEST: PASS ✅ |

### 压力测试

| 测试 | 配置 | 结果 |
|------|------|------|
| Basic Smoke | SMP=1, no disk | PASS |
| SMP Smoke | SMP=4, no disk | PASS |
| SD-Card Smoke | SMP=1, ext4+virtio-net+rtc | PASS |
| Stress-SMP R1 | 8 cases (SMP=1,2 × 128,256M × 2 loops) | 8/8 PASS |
| Stress-SMP R2 | 16 cases (SMP=1,2,4 × 128,256M × 8 loops) | 15/16 PASS |
| Lockdep | 锁顺序 + 递归检测 | 0 violations |
| Memory | 页面/表/映射回收 | 0 leaks |

---

*Co-Authored-By: Claude <noreply@anthropic.com>*
