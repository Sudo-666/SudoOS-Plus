# SudoOS M16 更新日志

## 概述

本次更新按照 Linux 内核架构模式，补全了设备驱动模型、VirtIO 设备驱动、网络栈、procfs/sysfs/devpts 文件系统、RNG/RTC 子系统，以及 socket 系统调用。

## 新增文件

### 设备驱动

| 文件 | 说明 |
|------|------|
| `kernel/src/device.rs` | Linux 风格的 bus/device/driver 设备驱动模型框架 |

**核心类型：**
- `DeviceType` — Block / Net / Rng / Rtc / Console / Input / Gpu / Sound / Socket
- `Device` — 设备结构体 (name, device_type, resources, private_data)
- `Driver` trait — probe / remove / device_type 生命周期
- `Bus` trait — match_driver / on_device_added
- 全局注册表：`DEVICES`, `DRIVERS`, `BUSES` (IrqSpinLock)

**API：**
- `register_device()` / `register_driver()` / `register_bus()`
- `find_devices_by_type()` / `find_device()` / `for_each_device()`

### RNG 子系统

| 文件 | 说明 |
|------|------|
| `kernel/src/rng.rs` | ChaCha20 DRBG + VirtIO-RNG 硬件熵源 |

**实现细节：**
- 自包含 ChaCha20 实现（~80 行，无外部依赖）：256-bit key + 64-bit nonce + 20 轮 quarter-round
- `EntropyPool` — 基于 ChaCha20 的 DRBG，支持硬件熵播种和退化播种（时钟+计数器混合）
- 每生成 1 MiB 触发重播种信号
- 退化模式：使用 `time::now()` / `time::timer_ticks()` / 栈地址混合播种

**公共 API：**
- `fill_random(buf)` — /dev/urandom 和 sys_getrandom 调用
- `fill_random_blocking(buf)` — /dev/random 语义（阻塞直到熵充足）
- `register_hardware_source(source)` — VirtIO-RNG 设备注册回调

### RTC 子系统

| 文件 | 说明 |
|------|------|
| `kernel/src/rtc.rs` | RTC 实时时钟子系统 |

- `RtcTime` — Unix 秒时间戳
- `RtcHardware` trait — 硬件抽象
- `read_rtc_time()` — 全局读时钟（有硬件则用硬件，否则退化为系统单调时间）
- `register_rtc()` — VirtIO-RTC 设备注册

### procfs 文件系统

| 文件 | 说明 |
|------|------|
| `kernel/src/procfs.rs` | /proc 虚拟文件系统 |

**架构：**
- `ProcFileGenerator` trait — 每个 proc 文件是一个实现 `generate() -> Vec<u8>` 的对象
- 文件内容在每次 read 时动态生成

**已实现文件：**
- `/proc/version` — 内核版本 `SudoOS 0.16 (M16)`
- `/proc/cpuinfo` — CPU 数量和架构名称
- `/proc/meminfo` — MemTotal / MemFree / MemAvailable
- `/proc/uptime` — 系统运行时间（秒）
- `/proc/mounts` — 挂载表
- `/proc/self` — 指向当前 PID 的符号链接

### sysfs 文件系统

| 文件 | 说明 |
|------|------|
| `kernel/src/sysfs.rs` | /sys 虚拟文件系统 |

**目录结构：**
```
/sys/
  kernel/     — version, ostype
  devices/    — list (已注册设备)
  class/      — block, net (设备分类)
```

### 网络子系统

| 文件 | 说明 |
|------|------|
| `kernel/src/net/mod.rs` | 网络子系统入口：NetDevice trait + 接口注册表 |
| `kernel/src/net/socket.rs` | Socket 层：AF_INET/TCP/UDP + 8 个 syscall |
| `kernel/src/net/virtio_net.rs` | VirtIO-Net 设备驱动 (MMIO + PCI) |

**NetDevice trait：**
- `mac_address()` / `mtu()` / `transmit()` / `receive()` / `poll_receive()`

**Socket 支持：**
- 域：AF_INET(2)、AF_INET6(10)
- 类型：SOCK_STREAM(1)、SOCK_DGRAM(2)
- 协议：IPPROTO_TCP(6)、IPPROTO_UDP(17)
- 全局 Socket 表 (BTreeMap)，通过 File path 字段编码 `"socket:<id>"` 实现 fd→socket 关联

**新增系统调用：**
| syscall | 编号 | 说明 |
|---------|------|------|
| socket | 198 | 创建 socket |
| bind | 200 | 绑定地址 |
| listen | 201 | TCP 监听 |
| accept | 202 | TCP 接受连接 |
| connect | 203 | TCP 连接 |
| sendto | 206 | UDP 发送 |
| recvfrom | 207 | UDP/TCP 接收 |
| shutdown | 210 | 关闭连接 |

**VirtIO-Net 驱动：**
- 封装 `VirtIONetRaw<SudoHal, T, 64>`，通过 IrqSpinLock 加锁
- `from_raw()` 工厂函数支持 MMIO 和 PCI 传输
- 预分配 2048 字节接收缓冲区
- 实现 `NetDevice` trait

**依赖：** smoltcp v0.11（TCP/UDP/IPv4/IPv6/ethernet）

### devpts 伪终端

| 文件 | 说明 |
|------|------|
| `kernel/src/devpts.rs` | PTY master/slave 对 |

**实现细节：**
- `PtyMaster` / `PtySlave` — 独立的 FileOperations 实现
- 环形缓冲区（4096 字节）实现双向数据转发
- 阻塞 I/O 支持（WaitQueue + scheduler_is_initialized 守卫）
- poll 支持（IN/OUT/HUP/ERR 事件）
- master 关闭 → slave 读 EOF；slave 关闭 → master 写 EPIPE
- `/dev/ptmx` — 每次 open 创建新 PTY 对，返回 master fd
- `/dev/pts/` — slave 目录

## 修改文件

### `kernel/src/main.rs`
- 新增模块声明：`device`, `rng`, `procfs`, `sysfs`, `net`, `rtc`, `devpts`
- 初始化调用：`device::initialize()`, `rng::initialize()`, `net::initialize()`, `rtc::initialize()`
- 挂载调用：`mount_proc()`, `mount_sys()`
- `mount_sdcard_if_present()` — 重构为错误宽容模式：ext4 superblock 校验失败时优雅跳过，文件不存在时不 panic
- 新增 debug verify 调用：`device::verify()`, `rng::verify()`, `devpts::verify()`, `rtc::verify()`

### `kernel/src/fs/mod.rs`
- `DeviceKind` 枚举扩展：`Random`, `Urandom`, `Ptmx`, `Rtc`
- `NodeState` 枚举扩展：`ProcFile(Arc<dyn ProcFileGenerator>)`
- `MountFsType` 枚举扩展：`Sysfs`
- `initialize()` 创建 `/dev/random`, `/dev/urandom`, `/dev/rtc`, `/dev/ptmx`, `/dev/pts/`
- `open()` 处理 Ptmx — 每次 open 创建新 PTY 对
- `mount()` 支持 `MountFsType::Proc` 和 `MountFsType::Sysfs`
- `populate_proc_root()` / `populate_sysfs_root()` — 构建 proc/sysfs 目录树
- `format_mounts()` — /proc/mounts 格式化输出
- `RegularFile::read()` / `write()` / `seek()` / `fstat()` / `truncate()` — ProcFile 节点支持
- `DeviceFile::read()` — Random/Urandom/Rtc 设备读取实现
- `truncate_node()` / `stat_for_node()` / `ioctl()` / `poll()` — 新增变体覆盖

### `kernel/src/virtio.rs`
- `probe_mmio_region()` — 增加 EntropySource (RNG) 和 Network 设备探测
- `probe_pci_host()` — 增加 EntropySource 和 Network PCI 设备探测
- 新增 `probe_pci_rng()` / `probe_pci_net()` 函数

### `kernel/src/user.rs`
- `sys_getrandom()` — 从 LCG 替换为 ChaCha20 DRBG (`rng::fill_random()`)
- 新增 8 个 socket syscall 常量定义和 dispatch
- `verify_sdcard_sample_thread()` — 文件缺失时优雅跳过而非 panic

### `kernel/src/syscall.rs`
- 新增 12 个 socket syscall 编号：`SOCKET(198)`, `BIND(200)`, `LISTEN(201)`, `ACCEPT(202)`, `CONNECT(203)`, `GETSOCKNAME(204)`, `GETPEERNAME(205)`, `SENDTO(206)`, `RECVFROM(207)`, `SETSOCKOPT(208)`, `GETSOCKOPT(209)`, `SHUTDOWN(210)`

### `vfs/src/lib.rs`
- Errno 枚举新增：`Eafnosupport(97)`, `Enotsock(88)`

### `kernel/Cargo.toml`
- 新增依赖：`smoltcp v0.11`（alloc, medium-ethernet, medium-ip, proto-ipv4/ipv6, socket-tcp/udp/raw）

## 验证结果

### 双架构启动验证

| 架构 | 镜像 | 结果 |
|------|------|------|
| RISC-V (riscv64imac) | alpine-linux-riscv64-ext4fs.img (690MB) | SMOKE_TEST: PASS ✅ |
| LoongArch | alpine-linux-loongarch64-ext4fs.img (690MB) | SMOKE_TEST: PASS ✅ |

### 压力测试

| 测试 | 配置 | 结果 |
|------|------|------|
| Basic Smoke | SMP=1, 256M, no disk | PASS |
| SMP Smoke | SMP=4, 256M, no disk | PASS |
| SD-Card Smoke | SMP=1, 256M, ext4 + virtio-net + rtc | PASS |
| Stress-SMP R1 | 8 cases (SMP=1,2 × 128M,256M × 2 loops) | 8/8 PASS |
| Stress-SMP R2 | 16 cases (SMP=1,2,4 × 128M,256M × 8 loops) | 15/16 PASS |

### 内存泄漏检测

- Page reclaim: verified ✅
- Table reclaim: verified ✅
- Mapping reclaim: verified ✅
- Resource reclaim: verified ✅
- Lockdep class/rank: verified ✅
- Instance-aware lockdep: verified ✅

## 新增 /dev 设备

- `/dev/random` — 阻塞式随机数（ChaCha20 DRBG + VirtIO-RNG 硬件熵）
- `/dev/urandom` — 非阻塞随机数
- `/dev/rtc` — 实时时钟（VirtIO-RTC 硬件或单调时间退化）
- `/dev/ptmx` — PTY master 多路复用器
- `/dev/pts/` — PTY slave 目录

## 新增挂载点

- `/proc` — proc 文件系统 (version, cpuinfo, meminfo, uptime, mounts, self)
- `/sys` — sysfs 文件系统 (kernel/, devices/, class/)

---

*Co-Authored-By: Claude <noreply@anthropic.com>*
