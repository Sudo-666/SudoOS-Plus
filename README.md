# SudoOS (MyOS)

一个使用 **Rust** 编写的业余操作系统内核，目标平台为 **RISC-V 64** 和 **LoongArch 64**。

启动期内存管理已成形，双架构均运行于高半内核且通过 MMU 硬件验证。LoongArch TLB refill 已接入，vmalloc/ioremap 均可硬件访问。SMP bring-up、抢占式内核调度、IPI 协作和 kernel-wide TLB shootdown 已在 RISC-V/LoongArch QEMU smoke 中验证。

## 运行效果

**RISC-V 64**

```
make run ARCH=riscv64
```

```
MyOS
  architecture : riscv64    boot cpu: 0    device tree: 0x8fe00000

fdt:             model: riscv-virtio,qemu   compatible: riscv-virtio
  cpu: 1    memory: 256 MiB    virtio-mmio: 8 devices

physical memory:         253 MiB usable
virtual memory:          Sv39, kernel @ 0xffffffff80000000

kernel image mapping:
  text             phys 0x80400000 → virt 0xffffffff80000000
  rodata           phys 0x80415000 → virt 0xffffffff80015000
  data/bss/stack   phys 0x8041f000 → virt 0xffffffff8001f000

RISC-V final address space:
  current PC      : 0xffffffff800035b0
  high text       : 0xffffffff80000000
  direct map      : verified
  low boot mapping: removed
```

**LoongArch 64**

```
make run ARCH=loongarch64
```

```
MyOS
  architecture : loongarch64    device tree: 0x100000    system table: 0x200

LoongArch DMW:
  DMW0 (uncached): 0x8000000000000001   DMW1 (cached): 0x9000000000000011
  high execution: verified

fdt:             compatible: linux,dummy-loongson3   cpu: 1   memory: 256 MiB

physical memory:         251 MiB usable
virtual memory:          LA64 DMW + 4-level paging, kernel @ 0x9000000000400000

kernel image mapping:
  text             phys 0x400000 → virt 0x9000000000400000
  rodata           phys 0x41b000 → virt 0x900000000041b000
  data/bss/stack   phys 0x425000 → virt 0x9000000000425000
```

## ELF 布局验证

RISC-V ELF program headers（`llvm-readelf -lW`）：

```
Entry point: 0x80200000

Type  VirtAddr            PhysAddr            Flg
LOAD  0x80200000          0x80200000          R E   (.boot.text)
LOAD  0x80201000          0x80201000          R     (.boot.rodata)
LOAD  0x80202000          0x80202000          RW    (.boot.bss, NOBITS)
LOAD  0xffffffff80000000  0x80400000          R E   (.text)
LOAD  0xffffffff80015000  0x80415000          R     (.rodata)
LOAD  0xffffffff8001f000  0x8041f000          RW    (.bss + .boot_stack)
```

关键不变量：
- boot 段 `VirtAddr == PhysAddr`，全部位于 `0x80200000` 附近
- kernel 段 `VirtAddr` 位于 `0xffffffff80000000`，`PhysAddr` 位于 `0x80400000`
- ELF 无运行时重定位

## 目标架构

| 架构 | 页表 | 内核链接 | 内核物理 | Boot 物理 | MMU | 状态 |
|------|------|---------|---------|----------|-----|------|
| RISC-V 64 | Sv39 (3级) | `0xffffffff80000000` | `0x80400000` | `0x80200000` | SATP | ✅ |
| LoongArch 64 | LA64 (4级) + DMW | `0x9000000000400000` | `0x400000` | `0x200000` | DMW | ✅ |

## Crate 架构

```
kernel ← boot, fdt, mm, runtime, arch-riscv64, arch-loongarch64
```

| Crate | 路径 | 职责 |
|-------|------|------|
| `myos-kernel` | `kernel/` | 入口 → 设备发现 → 内存初始化 → 页表构造 → MMU 切换 |
| `myos-boot` | `boot/` | `BootInfo` builder, `BootAddress` |
| `myos-fdt` | `firmware/fdt/` | FDT 验证 + parser 封装 + 设备枚举 |
| `myos-mm` | `mm/` | `PhysAddr`, `MemoryMap`, `PagePermissions`, `MappingOptions`, `EarlyFrameAllocator` |
| `myos-runtime` | `runtime/` | `ByteConsole` trait + `ConsoleWriter<C>` |
| `arch-riscv64` | `arch/riscv64/` | 两阶段启动汇编、Sv39 页表、direct-map、UART |
| `arch-loongarch64` | `arch/loongarch64/` | DMW 窗口启动、EFI system table、LA64 页表 |

## 启动流程

```
RISC-V:                               LoongArch:
  OpenSBI → _start (0x80200000)          QEMU → _start (0x200000)
    ├─ 临时 Sv39 页表构造                  ├─ DMW 窗口配置
    ├─ satp 写入                           ├─ CRMD.PG=1
    ├─ jr KERNEL_VIRT_BASE                ├─ jirl → cached DMW
    └─ __riscv_high_entry                  └─ rust_entry (高地址)
      ├─ BSS 清零 / gp / sp
      └─ rust_entry (高地址)
           └─ kernel_main
                 ├─ FDT 解析
                 ├─ 物理内存映射 (6 步排除)
                 ├─ 虚拟布局 + 页表策略
                 ├─ EarlyFrameAllocator + BootPageTable
                 ├─ 内核镜像映射 (高半)
                 ├─ direct-map 构造
                 ├─ switch_sv39_root / DMW 验证
                 ├─ trap vector 安装
                 ├─ IRQ dispatch 初始化
                 ├─ time/tick 计数初始化
                 └─ wfi / idle 0
```

## 单元测试

```bash
cargo test -p myos-mm    # 24 tests
```

## 构建 & 运行

```bash
make build ARCH=riscv64    # 构建
make run ARCH=riscv64      # 构建 + 运行
make run-riscv64           # 快捷命令
make run-loongarch64

make debug ARCH=riscv64    # GDB (端口 1234)
cargo test -p myos-mm      # 单元测试 (24 tests)
make check                 # source tree + fmt + host tests + 双架构 build/clippy
make smoke-all             # 双架构 QEMU 串口 smoke
make smoke-smp-all         # 双架构 SMP QEMU smoke
make stress-smp            # 可配置 SMP smoke 矩阵
make verify                # check + smoke-all + smoke-smp-all
make clean
```

常用 stress 示例：

```bash
make stress-smp STRESS_ARCHES="riscv64 loongarch64" STRESS_SMPS="1 2 4 8"
make stress-smp STRESS_ARCHES="riscv64 loongarch64" STRESS_PROFILES="debug release" STRESS_MEMS="64M 256M 1G"
```

stress 日志会写入 `build/stress-smp/`，每个 case 保存配置和串口输出，便于定位偶发失败。

## 工程文档

- [`docs/boot-order.md`](docs/boot-order.md)：当前启动顺序和初始化依赖。
- [`docs/context-rules.md`](docs/context-rules.md)：early/task/idle/hardirq/panic 上下文规则。
- [`docs/locking.md`](docs/locking.md)：当前锁、锁顺序草案和 M5 lockdep-lite 前置约束。
- [`docs/cpu-lifecycle.md`](docs/cpu-lifecycle.md)：CPU discovered/online/active/IPI-ready 生命周期。
- [`docs/scheduler-state-machine.md`](docs/scheduler-state-machine.md)：任务状态机、M4C/M4C2 verifier 证明边界。

## 当前进度

| 子系统 | 状态 | 说明 |
|--------|------|------|
| 构建系统 | ✅ | Cargo workspace（8 crates + 5 vendor），双架构 build/clippy/smoke 全绿 |
| RISC-V 两阶段启动 | ✅ | 低物理 entry.S → 静态 Sv39 → 高半内核 |
| LoongArch DMW 启动 | ✅ | DMW0/1 窗口 → cached high execution |
| FDT 设备枚举 | ✅ | model/compatible/cpu/memory/virtio-mmio |
| 物理内存映射 | ✅ | 6 步排除管线, MemoryMap 事务语义 |
| 虚拟内存布局 | ✅ | Sv39 + LA64 双策略 |
| 页表策略 | ✅ | W^X 强制, MappingOptions 校验 |
| 早期帧分配器 | ✅ | checkpoint/restore |
| 启动页表构造 | ✅ | BootPageTable + map_page + translate |
| 内核镜像映射 | ✅ | 高半 text/rodata/data |
| RAM direct-map | ✅ | Sv39 页表 / DMW 窗口 |
| MMU 启用 | ✅ | RISC-V SATP/Sv39 + LoongArch TLB refill 均已硬件验证 |
| 低地址映射撤销 | ✅ | RISC-V: removed / LoongArch: DMW2=0 |
| trap 分发 | ✅ | 双架构 TrapFrame、frame guard、连续异常 + 寄存器恢复自检、breakpoint 验证 |
| IRQ 子系统 | ✅ | 统一 InterruptSource dispatch，未处理 IRQ fail-fast |
| time/tick | ✅ | monotonic tick 计数 + 周期 timer armed + timer IRQ 验证 |
| 物理页分配器 | ✅ | buddy allocator, DMA32/Normal zone, page refcount, early handoff 校验 |
| 内核堆 | ✅ | global allocator (alloc crate), slab 小对象, buddy-backed large allocation |
| VMA/address space | ✅ | VmAreaSet、AddressSpace、brk metadata、mmap gap search |
| page fault policy | ✅ | anonymous/file/device/COW/protection/segv 分类模型 |
| page fault handler | ✅ | 双架构 fault trap 解码、统一 PageFault pipeline、kernel fail-fast; user demand paging 待 P4b |
| runtime page table | ✅ | 双架构 buddy-backed table pages、map/protect/unmap/translate，RISC-V 写入活动页表 |
| kernel vmalloc | ✅ | vmalloc/vfree/ioremap/iounmap API，guard page，双架构硬件生命周期验证 |
| TLB 模型 | ✅ | RISC-V sfence.vma + LoongArch TLB refill/invtlb；kernel TLB request v2 + request ID/target/completion mask |
| IRQ-safe 锁 | ✅ | IrqSpinLock 保存/恢复本地中断状态，嵌套锁自检 |
| SMP bring-up | ✅ | 双架构 secondary CPU 启动、online/ready mask、per-CPU trap/timer/stack、IPI delivery |
| 内核调度器 | ✅ | 抢占式 per-CPU FIFO round-robin、idle task、wait queue、work stealing、任务迁移、资源回收 |
| lockdep | ✅ | LockClass/LockRank、instance-aware runtime 违规检测、IRQ-off 周期追踪 |
| WaitQueue / Completion | ✅ | 阻塞/唤醒语义、switching-out race 检测、complete-all、quiescent reinit |
| IPI mailbox | ✅ | publish/coalesce/drain、AtomicU64 doorbell、payload 消息传递 |
| cross-CPU call-function | ✅ | 预分配请求槽、many-target/single-target 回调、completion ordering |
| TLB shootdown | ✅ | 显式 request ID、target/completion mask、page/range/long-range fallback、remote ACK |
| CPU lifecycle | ✅ | discovered/online/active/IPI-ready 显式状态机、IPI_READY_MASK 与 ONLINE_MASK 解耦 |
| tracked spin lock | ✅ | Rank-aware SpinLock、IRQ-enabled contention、migration pinning、instance-aware lockdep |
| idle/IPI 确定性验证 | ✅ | target timer disable、IRQ-disabled recheck、pending IPI at wait、single reschedule IPI |
| M5 并发基础 | ✅ | m5-quick 4/4 PASS、双架构 SMP=1/2/4/8 smoke 全绿、200 轮 pressure test |
| 系统调用 | ⬜ | syscall 表, U-mode |
| 设备驱动 | ⬜ | virtio-blk, virtio-net |
| 文件系统 | ⬜ | VFS, tmpfs/ext4（lwext4 适配层） |

## 下一步

M5 已冻结。M6 从 timer queue / timeout / workqueue 开始，随后进入用户态。

## M5 之后完整路线图

M5 完成后内核底座已具备：双架构启动、高半内核、SMP、抢占、等待队列、IPI、TLB shootdown、
lockdep、并发验证。但还没有真正的 Process、用户页表、syscall ABI、ELF 加载、VFS、
fork/exec/wait、pipe/signal、块设备和根文件系统。

接下来的目标按依赖顺序分为以下几层。

### 目标阶梯

| 目标 | 里程碑 | 核心依赖 |
|------|--------|---------|
| hello-user | M7/M8 | 最小用户态 + syscall write/exit |
| 简易 Shell | M13 | Process、ELF、VFS、fork/exec/wait、pipe、基础 signal、console TTY |
| 静态 BusyBox | M14 | musl 静态编译、更完整 syscall、mmap/brk、signal/ioctl/poll |
| Vim | M14 | 与 BusyBox 同组 syscall：terminal I/O + signal + 文件读写 |
| 本地 Git | M17 | VFS 正确性（fsync/rename/lockfile）+ mmap + zlib |
| GitHub clone/push | M18 | TCP/IP、DNS、TLS、CA、libcurl |

### 推荐最近执行顺序

```
M5  ← 已完成
  ↓
M6  timer queue / timeout / workqueue
  ↓
M7  U-mode / PLV3 + write/exit
  ↓
M8  per-process AddressSpace + user fault
  ↓
M9  Process/Thread + Linux syscall ABI
  ↓
M10 ELF + execve + initramfs
  ↓
M11 VFS + fd table
  ↓
M12 fork/exec/wait/pipe/signal
  ↓
M13 console TTY + 简易 shell
  ↓
M14 静态 BusyBox
  ↓
M15 virtio-blk + ext4
  ↓
M16 动态 musl
  ↓
M17 本地 Git
  ↓
M18 网络栈 + TLS + GitHub
```

**建议 M10（ELF/initramfs）和 M11（VFS/fd table）换序**：先做最小 VFS
（tmpfs + devfs + 几个 fd syscall）再 execve，这样 hello-user 之后立刻能
`open("/dev/console")` 和 `write(fd)`，不用把文件操作硬编码在 syscall 里。

### M6：时间、超时和工作队列

在进入用户态前先补齐内核基础设施：

- monotonic clock
- one-shot timer
- timer queue
- sleep
- WaitQueue timeout
- Completion timeout
- delayed work
- workqueue
- I/O timeout 基础

**为什么先做**：`nanosleep` 需要 timer queue，驱动需要 timeout，网络需要
retransmission timer，异步资源回收需要 workqueue，用户进程不能依赖 scheduler
tick 轮询等待。

**验收**：`sleep 10ms`、`sleep 1s`、取消 timer、timer 与 task exit 并发、
多 CPU 同时添加 timer、无 tick 时 one-shot 唤醒。

### M7：最小用户模式

只做最小闭环：内核创建地址空间 → 映射一页用户代码 → 映射用户栈 → 进入
U-mode/PLV3 → 用户执行 syscall → 内核返回用户态 → `sys_exit`。

先只实现 `write(1, "hello user\n", 11)` + `exit(0)`。

需要：
- RISC-V U-mode 入口 / LoongArch PLV3 入口
- 用户 trap frame
- syscall entry/return
- 用户栈
- 用户地址检查
- `copy_from_user/copy_to_user`
- `sys_write`
- `sys_exit`

这一阶段不要做 fork、动态链接、ext4。

### M8：真正的用户 MM

每进程独立页表根、内核高半映射共享、地址空间切换、ASID、`mm.active_cpus`、
per-mm TLB shootdown、用户 anonymous page、demand paging、用户栈增长、brk、
mmap、munmap、mprotect、page fault recovery、用户指针异常恢复。

建议对象结构：
```rust
struct AddressSpace {
    root: PageTableRoot,
    asid: Asid,
    active_cpus: AtomicCpuMask,
    vmas: VmaTree,
    page_table_lock: SpinLock<()>,
}
```

### M9：Process、Thread 和 syscall ABI

把 kernel task 扩展成：
```
Process
 ├── AddressSpace
 ├── FileTable
 ├── SignalState
 ├── Credentials
 ├── cwd/root
 └── Threads
```

每个 Thread 拥有 trap frame、kernel stack、user stack、TLS、signal mask、
scheduler state。

ABI：尽量采用 Linux 通用 64 位 syscall ABI（RISC-V 用 Linux RISC-V ABI、
LoongArch 用 Linux LoongArch ABI），syscall number、参数寄存器、错误返回
尽量兼容 Linux，这样 musl、BusyBox 只需极少补丁。

### M10：ELF、execve 和 initramfs

ELF64 loader、PT_LOAD、RX/RW 权限、BSS 清零、用户栈布局、argc/argv/envp、
auxiliary vector、`execve()`、内嵌 initramfs。

第一阶段使用静态链接，无动态链接器，无共享库。

用户栈至少需要 argc、argv[]、envp[]、auxv[]、AT_PAGESZ、AT_ENTRY、AT_PHDR、
AT_PHNUM、AT_RANDOM。完成后内核可以启动 `/init`。

### M11：文件描述符和 VFS

```rust
struct FileTable;
struct File;
trait FileOperations;
trait InodeOperations;
trait SuperBlockOperations;
```

先实现 initramfs、tmpfs、devfs、console device、`/dev/null`、`/dev/zero`、
`/dev/console`。

核心 syscall：`openat`、`close`、`read`、`write`、`pread/pwrite`、`lseek`、
`fstat`、`newfstatat`、`getdents64`、`mkdirat`、`unlinkat`、`renameat`、
`readlinkat`、`chdir`、`getcwd`、`dup`、`dup3`、`fcntl`、`ioctl`。

为了稳健运行 Git，文件系统必须支持：原子 rename、正确 truncate、
fsync/fdatasync、文件锁定语义、symlink、hardlink、可执行位、时间戳、目录一致性。

### M12：fork、exec、wait、pipe、signal

Shell 的核心不是 ELF，而是进程控制。

**第一版 fork**：完整地址空间复制，不立即做 COW。速度慢但状态机简单。稳定后再
升级到 COW。

**Signal 最小集合**：SIGCHLD、SIGINT、SIGTERM、SIGKILL、SIGPIPE、SIGSEGV、
SIGILL、SIGBUS、`rt_sigaction`、`rt_sigprocmask`、`rt_sigreturn`。

**Pipe**：pipe buffer、读端等待、写端等待、EOF、SIGPIPE、关闭引用计数、
poll readiness。

### M13：TTY 和基础 Shell

console TTY、canonical/raw mode、echo、backspace、Ctrl-C、foreground process
group、session、process group、`setsid`、`setpgid`、`tcsetpgrp`、
`TIOCGPGRP/TIOCSPGRP`。

第一版可以暂时不做完整 job control，只支持前台命令：`/bin/sh`、`ls`、`cat`、
`echo`、`cd`、`pwd`、`prog1 | prog2`、`prog > file`。

### M14：静态 BusyBox

```text
CONFIG_STATIC=y
CONFIG_ASH=y
CONFIG_FEATURE_SH_STANDALONE=y
CONFIG_FEATURE_PREFER_APPLETS=y
```

第一批 applet：sh、echo、cat、ls、pwd、mkdir、rm、cp、mv、ln、sleep、true、
false、mount、dmesg、ps。不要一开始启用所有网络工具。

**BusyBox 常用 syscall 最小集合**：

| 类别 | syscall |
|------|---------|
| 进程 | clone/fork、execve、exit_group、wait4、getpid、getppid、getpgid、setpgid、setsid |
| 内存 | brk、mmap、munmap、mprotect |
| 文件 | openat、close、read、write、lseek、fstat、newfstatat、getdents64、dup、dup3、pipe2、fcntl、ioctl |
| Signal | rt_sigaction、rt_sigprocmask、rt_sigreturn、kill |
| 时间 | clock_gettime、nanosleep |
| musl 基础 | set_tid_address、set_robust_list、getrandom、uname |

### M15：块设备和 ext4

```text
virtio-mmio → virtqueue → DMA → virtio-blk → block layer → buffer/page cache → ext4
```

已有 lwext4，它应该接在 VFS → ext4 adapter → block layer → virtio-blk 链路上。

实体机还需要 cache coherent/non-coherent DMA、DMA barrier、地址宽度、
scatter-gather、IOMMU 后续支持、中断 affinity。

### M16：动态链接和 musl

静态 BusyBox 通过后再做：PT_INTERP、动态链接器启动、shared object mmap、TLS、
relocations、`mprotect` RELRO。`dlopen` 和 vDSO 可后置。

### M17：本地 Git

目标：`git init`、`git add`、`git commit`、`git status`、`git log`、`git checkout`、
`git branch`。

重点验证：大量小文件、深目录、原子 `.lock → rename`、fsync、mmap pack/index、
stat 时间精度、symlink、executable bit、zlib、SHA 实现、环境变量、子进程、pipe、
临时文件。**一个看似能用但 rename/fsync 错误的文件系统可能直接损坏仓库。**

### M18：网络 Git

```text
git clone https://github.com/... / git fetch / git push
```

需要：Ethernet、ARP/NDP、IPv4/IPv6、ICMP、UDP、TCP、socket、bind/connect/listen/accept、
sendmsg/recvmsg、getsockopt/setsockopt、poll/ppoll、nonblocking I/O；
DNS（UDP + TCP fallback + resolver timeout）；TLS（cryptographic RNG、wall clock、
monotonic clock、CA certificate store、TLS library、libcurl）。

没有正确的真实时间和随机数，HTTPS Git 不能算可靠。

### 最重要的阶段性验收点

```
 1. hello-user
 2. /init 启动
 3. exec 两个不同程序
 4. fork + wait
 5. pipe + shell 重定向
 6. 静态 BusyBox ash
 7. BusyBox 在 ext4 根目录运行
 8. 本地 git init/add/commit
 9. TCP socket
10. HTTPS git clone
```

每通过一个阶段，都做双架构、SMP、内存泄漏和长期压力验证。

### 交接注意

- `build/` 已加入 `.gitignore`，之前清理缓存时有大量 tracked build artifacts staged for deletion；不要把这些当成源码删除事故。
- 当前 `myos-mm` 单测数是 24。
- `kernel vmalloc` 已有稳定 token API：`vmalloc/vfree/ioremap/iounmap`。释放函数消费 token，避免双重释放。
- RISC-V vmalloc 映射写入当前活动页表；LoongArch runtime page table/TLB refill 已接入，vmalloc VA 已可硬件解引用。
- page fault trap 入口已经接入；当前 kernel fault fail-fast，user demand paging 不能在 P4b 前假装完成。
- SMP bring-up、抢占调度、wait queue、work stealing、任务迁移和 kernel-wide TLB shootdown 已完成；后续 P7 指的是用户地址空间级别的 range/ASID 优化。
- 不要为了快速完成把用户态、VFS、SMP 依赖伪造成 stub 成品；这些必须按依赖顺序接入。


## M5 并发基础封版

M5 已完成 CPU 生命周期、instance-aware lockdep、WaitQueue/Completion、
task reaper、timer-off idle 唤醒、per-CPU IPI mailbox、CallFunction request
slot，以及显式 TLB request ID/target/completion mask。

```bash
make harness-test
make m5-quick
make m5-full
make m5-release M5_SOAK_LOOPS=200 M5_RELEASE_SOAK_LOOPS=20
```

每个 smoke case 都生成结构化 `result.json`，超时会区分仍在推进、真正停滞、
无输出、QEMU 提前退出和内核 panic。完整规则见 `docs/m5-completion.md`。

M5 后进入 M6：monotonic/one-shot timer、timer queue、timeout 和通用
workqueue，随后开始最小用户态与 syscall 闭环。
