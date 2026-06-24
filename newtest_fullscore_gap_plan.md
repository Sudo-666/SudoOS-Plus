# SudoOS-Plus `newtest` 离满分缺口分析与修复路线图

> 分析时间：2026-06-24  
> 分支：GitHub `Sudo-666/SudoOS-Plus` 的 `newtest`  
> 依据：`newtest` 分支源码、用户上传的完整 RISC-V / LoongArch 评测输出。  
> 注意：本文档不是“保证满分清单”。OS 赛评测分值映射未公开，下面按日志中已经暴露的失败点和 Linux ABI 兼容层缺口排序，目标是从当前约 48 分稳定向上推进。

---

## 0. 当前事实：已经不是 boot 阶段问题

### 已经完成并通过的部分

两架构都已经能进入内核主流程，完成：

- 高半内核启动。
- 物理页分配器、内核堆、trap、time、virtio block/net。
- VFS/devfs/procfs/sysfs 初始化。
- ext4 sdcard 识别。
- 测试脚本扫描。
- BusyBox/shell 启动。
- M7-M15 内部 gate 输出基本完整。

从日志看，`sdcard` 阶段已经能扫描 59 个目录、发现 128 个测试脚本：

```text
sdcard:
  mount         : /dev/vda (ext4)
  mounted tree  : /mnt/sdcard (lazy file install)
  root entries  : 3
  scanned dirs   : 59
  test scripts  : 128
```

因此，当前不是“扫不到测试”，也不是“系统跑不起来”。当前已经进入真实用户态程序兼容性阶段。

---

## 1. 最高优先级结论

当前离满分主要差在四层：

| 优先级 | 类别 | 典型日志 | 判断 |
|---|---|---|---|
| P0 | Linux ABI 指纹与比赛运行时稳定性 | `FATAL: kernel too old`、`lock order violation: tty.console -> scheduler` | 最快涨分，必须先修 |
| P1 | 脚本/目录/VFS 语义 | `Exec format error`、`No such file or directory`、`Function not implemented` | 影响 BusyBox/basic/libctest |
| P2 | ELF 动态链接与 PIE | `PT_INTERP` 被拒绝、`Invalid argument`、低地址执行 fault | glibc 大程序核心瓶颈 |
| P3 | 线程/futex/TLS/网络/性能类 syscall | `pthread` fault、`sched_getaffinity: Function not implemented`、netperf/cyclictest fail | 后期大分项 |

短期不要直接写完整动态链接大工程。先把 P0/P1 做稳，避免评测进程提前退出、内核 panic 或大量脚本假失败。

---

## 2. P0：立即修复项

### 2.1 `uname.release` 太低导致 glibc 报 `FATAL: kernel too old`

#### 现象

RISC-V 输出中大量出现：

```text
FATAL: kernel too old
Unknown signal (core dumped)
```

很多 glibc 测试脚本因此直接 exit 250。

#### 根因

glibc 程序启动时会检查内核版本。`newtest` 目前 `sys_uname` 返回的版本如果过低，会触发 glibc 的最低内核版本检查。即使程序是静态 glibc，也可能在 libc startup 阶段退出。

#### 修复策略

将 `sys_uname` 的 `release` 伪装为较新的 Linux，例如：

```text
sysname  = "Linux"
nodename = "sudoos"
release  = "6.12.0"
version  = "#1 SMP PREEMPT_DYNAMIC"
machine  = "riscv64" / "loongarch64"
```

#### 代码规范

- 这是 Linux ABI 兼容，不是伪造测试结果。
- 不要打印 `SudoOS` 给用户态 `uname`，评测脚本一般期待 Linux-like。
- 架构字段必须按目标架构区分：
  - RISC-V: `riscv64`
  - LoongArch: `loongarch64`

#### 验收标准

运行后日志中 `FATAL: kernel too old` 应大幅减少或消失。

---

### 2.2 初始栈 auxv 不完整

#### 现象

源码中 `exec.rs` 已有基础 auxv：

```text
AT_PHDR
AT_PHENT
AT_PHNUM
AT_BASE
AT_FLAGS
AT_ENTRY
AT_PAGESZ
AT_SECURE
AT_RANDOM
AT_EXECFN
```

但缺少 libc 常用项：

```text
AT_UID
AT_EUID
AT_GID
AT_EGID
AT_CLKTCK
AT_PLATFORM
AT_HWCAP
AT_HWCAP2
```

#### 影响

部分 libc 或 runtime 会根据 auxv 判断平台、时钟 tick、权限模型、硬件能力。缺项不一定必崩，但会增加兼容性问题。

#### 修复策略

在 `build_initial_stack()` 的 auxv 中补：

```rust
const AT_UID: usize = 11;
const AT_EUID: usize = 12;
const AT_GID: usize = 13;
const AT_EGID: usize = 14;
const AT_CLKTCK: usize = 17;
const AT_PLATFORM: usize = 15;
const AT_HWCAP: usize = 16;
const AT_HWCAP2: usize = 26;
```

值建议：

```text
AT_UID/AT_EUID/AT_GID/AT_EGID = 0
AT_CLKTCK = 100
AT_PLATFORM = stack 上字符串 "riscv64" 或 "loongarch64"
AT_HWCAP/HWCAP2 = 0 起步，后续可填真实能力
```

#### 代码规范

- `AT_PLATFORM` 不能直接传内核静态地址，必须把字符串 push 到用户栈，auxv 放用户虚拟地址。
- auxv 构造前必须 `try_reserve`，保持 no_std OOM 可控。
- auxv 顺序不强制，但必须以 `AT_NULL, 0` 结束。

---

### 2.3 console lockdep 不能在比赛运行时杀内核

#### 现象

RISC-V 后段进入 Lua/TTY 场景后 panic：

```text
lock order violation: held=tty.console(Console/#1) new=scheduler(Scheduler/#1)
```

这会让后续所有测试直接没机会跑。

#### 根因

`lockdep.rs` 当前 rank 中 `Scheduler = 20`、`Console = 80`，普通 lock order 检查遇到 `Console -> Scheduler` 就 panic。TTY/console 路径在用户读写、阻塞、唤醒、调度之间容易交错，比赛运行时不应该由 console lockdep 直接杀内核。

#### 修复策略

有两种可选方案：

方案 A：在 lockdep order check 中将 `Console` 视为 neutral rank，不参与普通 rank 递增检查。

方案 B：只在 `CONTEST=1` 或 `RUN_OSCOMP=1` 时降级为 warning，不 panic。

推荐 A，最小且不影响其它核心锁顺序。

#### 代码规范

- 不要全局关闭 lockdep。
- 只对 `LockRank::Console` 做例外。
- 仍保留 recursive lock detection、IRQ-off 时间统计、其它 rank 检查。

---

## 3. P1：脚本执行、VFS、基础 Linux syscall

### 3.1 脚本 `#!` / `ENOEXEC` fallback

#### 现象

LoongArch 输出中出现：

```text
sh: can't execute './run-static.sh': Exec format error
sh: can't execute './run-dynamic.sh': Exec format error
```

这说明脚本文件存在，但 `execve()` 认为它不是 ELF，也没有走 Linux 的脚本解释器路径。

#### Linux-like 行为

Linux 内核会处理以 `#!` 开头的脚本：

```text
#!/bin/sh
```

内核将其转换为：

```text
argv = ["/bin/sh", script_path, old_argv[1..]]
```

#### 修复步骤

1. 在 `execve` 读入目标文件后，先判断前两个字节是否为 `#!`。
2. 解析第一行，提取 interpreter 和 optional arg。
3. 防止递归过深，设置 `BINPRM_MAX_RECURSION = 4`。
4. 重新读取 interpreter 文件并走 ELF exec。
5. 保留原 argv[1..]。
6. 如果不是 ELF 且不是 shebang，返回 `-ENOEXEC`，不要返回 `-EINVAL`。

#### 验收标准

`run-static.sh`、`run-dynamic.sh` 不再直接 `Exec format error`。

---

### 3.2 VFS 基础写操作和 rename/mkdir/mknod

#### 现象

RISC-V BusyBox-musl 中出现：

```text
touch: test.txt: Function not implemented
mv: can't rename 'test_dir': Function not implemented
```

说明 `utimensat`、`renameat/renameat2` 或对应 VFS hook 仍不完整。

#### 修复步骤

1. `utimensat`：
   - 如果目标存在，更新时间可以 no-op 返回 0。
   - 如果 `AT_SYMLINK_NOFOLLOW` 当前不支持，可先按普通路径处理。
   - 对 `touch newfile`，真正创建依赖 `openat(O_CREAT)`，不是 `utimensat` 单独完成。

2. `renameat2`：
   - flags 为 0 时复用 `renameat`。
   - `RENAME_NOREPLACE` 若目标存在返回 `-EEXIST`。
   - 不支持的 flags 返回 `-EINVAL`。
   - tmpfs 需要实现目录项 move。

3. `mknodat`：
   - 常规文件不应走 `mknod`。
   - 对 `/dev/null` 等特殊节点可允许 no-op 或映射到 devfs。
   - 不支持的设备类型返回合理 errno。

#### 代码规范

- 不要把所有未实现都返回 0。
- 有状态操作必须真的修改 VFS，否则后续 `stat/open/read` 会不一致。
- tmpfs 的 inode/link count/name cache 要一致。

---

### 3.3 `statfs/fstatfs/syslog/ioctl` 兼容

#### 现象

日志出现：

```text
df: /dev: Function not implemented
dmesg: klogctl: Function not implemented
hwclock: RTC_RD_TIME: Not a tty
```

#### 修复步骤

1. `statfs/fstatfs`：
   - tmpfs/devfs/procfs/sysfs 都返回合理 `struct statfs`。
   - f_type 可用常见 magic：
     - tmpfs: `0x01021994`
     - proc: `0x9fa0`
     - sysfs: `0x62656572`
     - devtmpfs 可复用 tmpfs magic
   - blocks/free 可以近似，不影响大多数测试。

2. `syslog/klogctl`：
   - `SYS_SYSLOG` 支持 type=3/4/10。
   - 可返回空 ring buffer 或固定内核日志长度。
   - 不要 ENOSYS。

3. `/dev/rtc` ioctl：
   - 支持 `RTC_RD_TIME` 返回固定时间结构。
   - 不认识的 ioctl 返回 `-ENOTTY`，不要误返回 `-EINVAL`。

#### 验收标准

BusyBox `df/dmesg/hwclock` 至少不应因为 ENOSYS/ENOTTY 崩掉；失败可以是测试逻辑失败，但不应中断 shell。

---

### 3.4 `getdents64` / 当前目录语义

#### 现象

早期日志中出现：

```text
find: .: Invalid argument
du: can't open '.': Invalid argument
ps: can't open '/proc': Invalid argument
```

后续某些场景改善，但 `.`、`..`、目录 fd、procfs 遍历仍是稳定风险。

#### 修复步骤

1. `openat(AT_FDCWD, ".", O_DIRECTORY)` 必须成功。
2. `getdents64` 对目录 fd 返回：
   - `.`
   - `..`
   - 当前目录下条目
3. `d_off` 单调递增，重复调用能继续读取。
4. `d_type` 填：
   - `DT_REG = 8`
   - `DT_DIR = 4`
   - `DT_LNK = 10`
5. procfs 的 `/proc`、`/proc/self`、`/proc/self/fd` 至少可枚举。

#### 代码规范

- fd offset 必须由 fd table 维护。
- 目录读取不可一次性假返回所有后丢状态。
- 对 buffer 太小返回已写部分或 `-EINVAL`，不要越界。

---

## 4. P2：ELF 动态链接与 PIE

### 4.1 当前源码明确拒绝 `PT_INTERP`

`exec.rs` 中 `prepare_elf()` 解析 ELF 后调用动态 handoff 检查；注释说明当前只记录 ET_DYN/PT_INTERP/PT_DYNAMIC 元信息，完整 interpreter/relocation/TLS 路径还没实现。函数 `reject_dynamic_handoff_if_needed()` 对 `elf.interpreter.is_some()` 直接返回 `ExecError::DynamicInterpreterUnsupported`。

这意味着带 `PT_INTERP` 的 glibc 动态程序不可能完整运行。

### 4.2 为什么不能用 musl 跑 glibc

glibc 程序依赖：

- glibc loader：`ld-linux-*`
- `libc.so.6`
- symbol version
- TLS 初始化
- relocation
- pthread/futex 约定

musl loader 不能稳定替代 glibc loader。短期可以优先跑 `/musl` 测试，但不能用 musl 直接吃掉 glibc 全部测试。

### 4.3 最小动态 ELF 闭环

#### 目标

让 `execve("/mnt/sdcard/glibc/foo")` 遇到 `PT_INTERP` 时：

```text
main ELF 加载到用户地址空间
interpreter ld-linux 加载到用户地址空间
用户入口 = interpreter entry
auxv 告诉 ld-linux 主程序入口、program header、AT_BASE
```

#### 步骤

1. `elf.rs`：
   - 保留并校验 `PT_INTERP`。
   - 暴露 interpreter 字符串。
   - 暴露 `PT_DYNAMIC`、`PT_TLS`。
   - 对 ET_DYN 计算 load bias。

2. `exec.rs`：
   - `PreparedExec` 增加：
     - `main_entry`
     - `interp_entry`
     - `interp_base`
     - `main_phdr`
     - `main_phent`
     - `main_phnum`
   - 如果 `PT_INTERP` 存在：
     - 从 VFS 读取 interpreter。
     - map interpreter 的 PT_LOAD。
     - entry 设置为 interpreter entry。
     - auxv:
       - `AT_BASE = interp_base`
       - `AT_ENTRY = main_entry`
       - `AT_PHDR/AT_PHENT/AT_PHNUM = main ELF`
   - 如果无 `PT_INTERP`：
     - 保持静态 ELF 路径。

3. `VFS` 路径兼容：
   - glibc loader 可能请求 `/lib/ld-linux-riscv64-lp64d.so.1`。
   - 在 sdcard mount 阶段建立 symlink 或 path alias：
     - `/lib`
     - `/lib64`
     - `/usr/lib`
     - 指到 `/mnt/sdcard/glibc/lib` 或 `/mnt/sdcard/musl/lib`
   - 不要复制超大文件，优先 VFS alias/symlink。

4. `mmap`：
   - 动态 loader 会用 `mmap` 映射 `.so`。
   - 支持 file-backed MAP_PRIVATE。
   - 支持 `PROT_READ|PROT_EXEC`、`PROT_READ|PROT_WRITE`。
   - `MAP_FIXED_NOREPLACE` 可先实现基本语义。
   - W^X 必须保留。

5. relocation：
   - 静态 PIE 已有部分 relocation 逻辑，但动态 linker 由用户态 ld-linux 做。
   - 内核不需要替用户态做 glibc relocation。
   - 但必须让 ld-linux 能 mmap/read/close/fstat libc.so，并能 mprotect RELRO。

#### 验收标准

- glibc 程序不再 `Invalid argument` / `DynamicInterpreterUnsupported`。
- `ld-linux` 入口能跑起来。
- 如果失败，失败点应转移到具体 syscall，例如 `openat/readlinkat/mmap/futex`，而不是 ELF 装载阶段。

---

## 5. P3：线程、futex、TLS、clone

### 5.1 现象

LoongArch 和 RISC-V 的 libcbench/musl 测试中出现：

```text
b_pthread_createjoin_serial1
user fatal fault: address=0x0 access=Write

b_malloc_thread_stress
user fatal fault: address=0x28 access=Read
```

这说明线程/TLS/futex/clone 语义还不稳定。

### 5.2 必须完善的 syscall

| syscall | 最低语义 |
|---|---|
| `clone` | 正确处理 child stack、TLS、CLONE_VM、CLONE_THREAD、CLONE_SETTLS、CLONE_CHILD_CLEARTID |
| `set_tid_address` | 保存 clear_child_tid，退出时写 0 并 futex wake |
| `set_robust_list` | 可先记录并返回 0 |
| `futex` | FUTEX_WAIT / WAKE / PRIVATE_FLAG / timeout |
| `rt_sigaction` | 至少保存 handler/mask/flags |
| `rt_sigprocmask` | 正确保存线程 signal mask |
| `rt_sigreturn` | 恢复 trap frame |
| `nanosleep/clock_gettime` | 支持 timeout 和 monotonic |

### 5.3 架构注意点

#### RISC-V

- syscall args: `a0-a5`，syscall nr: `a7`，return: `a0`。
- TLS register: `tp`。
- 用户态 trap 返回必须恢复 `sepc/sstatus/sp/tp`。

#### LoongArch

- syscall args: 通常 `a0-a5`，syscall nr 根据 ABI 走 `a7` 或指定寄存器，必须与当前 trap 入口实现一致。
- TLS register 要确认当前实现没有被 CpuId 或 kernel per-cpu 复用污染。
- 之前项目中过 LoongArch `tp/r21` 类 bug，后续 clone/TLS 必须架构单测覆盖。

### 5.4 验收标准

- `libcbench-musl` 中 pthread 项不再低地址 fault。
- `b_malloc_thread_*`、`b_pthread_*` 不再触发 user fatal fault。
- `cyclictest` 不再因为 affinity/scheduler 参数直接失败。

---

## 6. P4：调度、affinity、实时测试

### 6.1 现象

RISC-V cyclictest-musl 中：

```text
libnuma: Warning: Unable to determine max cpu (sched_getaffinity: Function not implemented); guessing.
unable to get scheduler parameters
```

### 6.2 修复步骤

1. `sched_getaffinity(pid, cpusetsize, mask)`：
   - 单核返回 bit0。
   - `cpusetsize < sizeof(usize)` 返回 `-EINVAL`。
   - pid=0 当前线程。
   - 其它 pid 可先查进程表或返回 `-ESRCH`。

2. `sched_setaffinity`：
   - 单核可接受包含 bit0 的 mask 并返回 0。
   - 不包含 bit0 返回 `-EINVAL`。

3. `sched_getscheduler`：
   - 返回 `SCHED_OTHER = 0`。
   - 若实现 realtime，可支持 `SCHED_FIFO/RR` stub。

4. `sched_setscheduler`：
   - 可接受 `SCHED_OTHER`。
   - 对 `SCHED_FIFO/RR` 可先记录但仍用 RR 内核调度。
   - 不支持参数返回 `-EINVAL`，不要 ENOSYS。

5. `sched_getparam` / `sched_setparam`：
   - 结构体 priority 读写。
   - `SCHED_OTHER` priority=0。

### 6.3 验收标准

- `libnuma` 不再打印 `sched_getaffinity: Function not implemented`。
- cyclictest 至少能跑出结果，而不是启动阶段失败。

---

## 7. P5：网络测试

### 7.1 现象

netperf/iperf 中有：

```text
netperf UDP_STREAM end: fail
TCP_STREAM end: fail
Unknown signal
```

### 7.2 必需语义

| 层 | 工作 |
|---|---|
| socket fd | fd table 中 socket/file 分开，但统一 read/write/poll |
| UDP | socket/bind/sendto/recvfrom |
| TCP | socket/bind/listen/accept/connect/read/write/shutdown |
| poll/ppoll/select | smoltcp socket readiness |
| getsockopt/setsockopt | 常用选项 stub：SO_REUSEADDR、TCP_NODELAY、SO_ERROR |
| ioctl | FIONBIO、SIOCGIFCONF/SIOCGIFADDR 可最小实现 |
| netdev | eth0 IP 配置；必要时给 10.0.2.15/24 + gateway 10.0.2.2 |

### 7.3 验收标准

- netserver 能常驻后台。
- netperf 客户端不再启动即 fault。
- UDP/TCP basic case 至少能完成一次 loopback 或 QEMU user-net 通路。

---

## 8. P6：文件系统与 ext4/VFS 完整性

### 8.1 当前表现

目录扫描、文件加载已经能让 `/glibc`、`/musl` 下很多文件进入 VFS。但仍有：

- 某些子目录未展开：如 LoongArch 中 `./basic`、`./lua`、`./lmbench_all` 仍找不到。
- tmpfs 写操作、rename、touch、rmdir 语义不稳定。
- procfs/sysfs/devfs 只是最小文件集合。

### 8.2 修复路线

1. `sdcard` materialization：
   - 执行脚本前展开脚本所在目录。
   - 对 `basic/`, `lib/`, `lua/`, `ltp/`, `lmbench/` 等目录展开两层。
   - 维护 `expanded_dirs` set，避免重复展开。
   - 不要一次性递归整个 ext4，避免 OOM。

2. VFS 路径解析：
   - 支持 `.`, `..`, symlink。
   - 支持 cwd 相对路径。
   - 支持 trailing slash 规则。

3. tmpfs：
   - 创建、删除、rename、truncate、append、目录 link count。
   - 文件 offset、O_APPEND、O_TRUNC、O_CREAT、O_EXCL。

4. procfs：
   - `/proc`
   - `/proc/self`
   - `/proc/self/fd`
   - `/proc/meminfo`
   - `/proc/cpuinfo`
   - `/proc/uptime`
   - `/proc/mounts`
   - `/proc/stat`
   - `/proc/sys/kernel/osrelease`

5. devfs：
   - `/dev/null`
   - `/dev/zero`
   - `/dev/random`
   - `/dev/urandom`
   - `/dev/console`
   - `/dev/rtc`
   - `/dev/tty`
   - `/dev/ptmx`
   - `/dev/pts`

---

## 9. P7：信号语义

### 9.1 现象

日志中大量：

```text
Unknown signal
Unknown signal (core dumped)
Hangup
```

这说明 shell 能观察到子进程异常退出，但 signal number、core dump、wait status 编码不够 Linux-like。

### 9.2 修复点

1. wait status 编码：
   - 正常退出：`status << 8`
   - signal killed：`signal & 0x7f`
   - core dumped：`signal | 0x80`
   - stopped/continued 后续可补。

2. 常用 signal number：
   - SIGHUP=1
   - SIGINT=2
   - SIGQUIT=3
   - SIGILL=4
   - SIGABRT=6
   - SIGFPE=8
   - SIGKILL=9
   - SIGSEGV=11
   - SIGPIPE=13
   - SIGTERM=15
   - SIGCHLD=17

3. fatal user fault：
   - page fault execute/read/write 映射为 SIGSEGV。
   - illegal instruction 映射为 SIGILL。
   - bad syscall 可按 SIGSYS 或 ENOSYS 处理。

4. shell job control：
   - `kill`, `tkill`, `tgkill` 需要能找到任务。
   - 不支持 process group 时，也不要返回奇怪 signal。

---

## 10. 工程规范：后续代码必须怎么写

### 10.1 禁止事项

- 禁止直接伪造测试脚本 PASS。
- 禁止对核心 syscall 盲目返回 0。
- 禁止把 glibc 程序强行交给 musl loader。
- 禁止为一个测试写硬编码路径特判，例如只识别 `unixbench_testcode.sh`。
- 禁止隐藏 kernel panic；比赛态可以把 debug verifier 降级，但真实资源泄漏仍要可审计。

### 10.2 允许的 Linux-like stub

这些可先返回合理值：

```text
uname
sysinfo
getrlimit/prlimit64
getrusage
times
getuid/geteuid/getgid/getegid
getppid
setsid/getpgid/setpgid
set_robust_list
sched_getaffinity/sched_setaffinity 单核语义
sched_getscheduler/sched_setscheduler 基础语义
statfs/fstatfs
syslog/klogctl 最小语义
部分 ioctl 白名单
```

### 10.3 不能假成功的 syscall

这些必须真做状态：

```text
execve
mmap/munmap/mprotect
brk
clone
wait4
futex
openat/read/write/lseek
getdents64
rt_sigreturn
socket/connect/sendto/recvfrom
rename/unlink/mkdir/rmdir
```

### 10.4 patch 结构规范

每个补丁必须包含：

```text
1. 代码修改
2. audit 脚本
3. Makefile target
4. smoke/contest 输出中的目标字符串
5. 回滚点
```

推荐命名：

```text
scripts/oscomp-newtest-pX-xxx-audit.py
make oscomp-newtest-pX-xxx-audit
```

### 10.5 audit 规则

audit 不应该只 grep 注释，必须检查：

- 关键函数存在。
- 关键分支不返回 ENOSYS。
- 不存在测试特判字符串。
- auxv / syscall number / errno 都按 Linux asm-generic ABI。
- 双架构 cfg 都覆盖。

### 10.6 日志规范

对 `execve` 加一次性诊断：

```text
execve: path=/mnt/sdcard/glibc/foo kind=ET_DYN interp=/lib/ld-linux-...
execve: failed path=... err=DynamicInterpreterUnsupported
execve: failed path=... err=UnsupportedRelocation(type=...)
```

但要做限流：

```text
每种错误最多打印 8 次
```

避免串口输出撑爆评测日志。

---

## 11. 推荐里程碑顺序

### M16-P0：ABI startup hotfix

目标：

- `FATAL: kernel too old` 消失。
- auxv 更完整。
- console lockdep 不再 panic。

文件：

```text
kernel/src/user.rs
kernel/src/exec.rs
kernel/src/lockdep.rs
scripts/oscomp-newtest-p0-abi-audit.py
Makefile
```

验收：

```bash
make oscomp-newtest-p0-abi-audit
make all
```

日志验收：

```text
grep -c "FATAL: kernel too old" Riscv输出.txt
grep -c "lock order violation" Riscv输出.txt
```

---

### M16-P1：binfmt_script + ENOEXEC

目标：

- `run-static.sh` / `run-dynamic.sh` 不再 Exec format error。
- `.sh` 可直接 `execve`。
- shell fallback 正常。

文件：

```text
kernel/src/exec.rs
kernel/src/user.rs
scripts/oscomp-newtest-p1-binfmt-script-audit.py
```

---

### M16-P2：VFS write/rename/statfs/ioctl

目标：

- BusyBox `df/dmesg/touch/mv/rmdir/hwclock` 明显改善。
- `Function not implemented` 明显减少。

文件：

```text
kernel/src/user.rs
kernel/src/fs/mod.rs
kernel/src/devfs.rs or fs device module
scripts/oscomp-newtest-p2-vfs-abi-audit.py
```

---

### M16-P3：affinity/scheduler policy

目标：

- cyclictest 不再因 `sched_getaffinity` / scheduler 参数启动失败。
- libnuma warning 消失或降级。

文件：

```text
kernel/src/user.rs
kernel/src/scheduler.rs
scripts/oscomp-newtest-p3-sched-abi-audit.py
```

---

### M16-P4：PT_INTERP dynamic ELF

目标：

- glibc 动态程序能进入 ld-linux。
- `DynamicInterpreterUnsupported` 消失。
- 失败点转移到具体 syscall，而不是 exec 装载阶段。

文件：

```text
kernel/src/elf.rs
kernel/src/exec.rs
kernel/src/fs/mod.rs
scripts/oscomp-newtest-p4-dynamic-elf-audit.py
```

---

### M16-P5：TLS / clone / futex

目标：

- libcbench pthread/malloc thread 项减少 fault。
- Lua/netperf/iozone 中低地址 execute/read/write fault 减少。
- `Unknown signal` 减少。

文件：

```text
kernel/src/user.rs
kernel/src/process.rs
kernel/src/task.rs
kernel/src/signal.rs
kernel/src/user_mm.rs
arch/*/trap.rs
```

---

### M16-P6：network/poll/select

目标：

- iperf/netperf basic UDP/TCP 通过。
- ppoll/pselect/socket readiness 不再假阻塞或直接失败。

文件：

```text
kernel/src/net.rs
kernel/src/user.rs
kernel/src/fs/fd.rs
```

---

## 12. 每次提交前检查清单

```bash
cargo fmt --all --check
make clippy-riscv64
make clippy-loongarch64
make all
make oscomp-newtest-pX-xxx-audit
```

如果本地有 sdcard：

```bash
qemu-system-riscv64 ... > rv.log
qemu-system-loongarch64 ... > la.log

grep -E "FATAL|panic|lock order|Function not implemented|Exec format|No such file|Unknown signal|user fatal fault" rv.log | head -100
grep -E "FATAL|panic|lock order|Function not implemented|Exec format|No such file|Unknown signal|user fatal fault" la.log | head -100
```

提交说明格式：

```text
contest: fix <subsystem> for newtest runtime

- why: quote exact failing log
- what: Linux-like behavior implemented
- audit: make oscomp-newtest-pX-xxx-audit
- risk: what is deliberately stubbed vs fully implemented
```

---

## 13. 总结

`newtest` 当前已经具备一个可继续冲分的基础：双架构启动、sdcard 扫描、脚本执行、部分 BusyBox/musl 测试已经跑起来。

离满分的核心差距不是一个点，而是完整 Linux 用户态 ABI：

1. 先修 P0：`uname`、auxv、console lockdep，避免 glibc 自杀和内核 panic。
2. 再修 P1/P2：binfmt_script、VFS 写语义、statfs/ioctl，吃 BusyBox/basic 分。
3. 再修 P3/P4：affinity、动态 ELF、PT_INTERP，打开 glibc 大程序入口。
4. 最后修 P5/P6：TLS/clone/futex/network/poll，把性能和网络大测例跑稳。

不要假 PASS。可以做 Linux-like stub，但核心状态型 syscall 必须真实维护状态，否则后面会以更隐蔽的方式崩溃。
