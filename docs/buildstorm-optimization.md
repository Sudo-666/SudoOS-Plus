# BuildStorm 双架构优化设计与验证记录

> 适用分支：`final-2026` 提交实现（RISC-V 64 / LoongArch 64）  
> 工作负载：8 核、8 GiB、Debian glibc rootfs 内原生 Rust/Cargo 并行编译  
> 原则：不伪造评分标志，不修改评测镜像或官方测试脚本；所有镜像运行均使用 QEMU `-snapshot`

## 1. 目标与计分边界

BuildStorm 不只是“能够启动 Cargo”。完整路径包括：

1. 识别 8 个处理器并启动原生 Rust 工具链；
2. 创建、编译并运行最小 Cargo 工程；
3. 在 `/work/tgoskits` 中预编译 `tg-xtask`；
4. 清空目标架构输出后，以 8 核并行编译 `arceos-helloworld`；
5. 检查产物存在且不少于 500000 字节。

自动评分为工具链 8 分、minibuild 12 分、完整编译 40 分、耗时 120 分；优化设计文档另计 20 分。耗时只覆盖 `cargo xtask arceos build`，不包含 `tg-xtask` 的前置编译和产物运行验证。

官方 `final-2026` 自检 judge 在 2026-07-30 的参考基线为 RISC-V 1616.09 秒、LoongArch 1985.21 秒；平台可通过配置使用不同基线，最终以评测输出为准。计分公式和文档评分项以[官方 final-2026 README](https://github.com/oscomp/testsuits-for-oskernel/tree/final-2026)为准。

本实现把“正确完成”放在第一层门禁，把“减少编译总时长”放在第二层。任何性能改动都必须同时通过 RISC-V、LoongArch 的 BuildStorm 环境检查和 CAgent 10/10 回归。

## 2. 测量方法与证据链

仓库保留两套入口：

- `make final-buildstorm-rv` / `make final-buildstorm-la`：原样执行镜像提供的正式脚本，只接受真实 `BUILDSTORM_COMPILE ... ok=true`；
- `make final-buildstorm-rv-diag` / `make final-buildstorm-la-diag`：不输出评分成功标志，用于旧镜像缺依赖时继续验证真实 Cargo/rustc、进程生命周期和崩溃位置。

正式入口由 `scripts/verify-final-script-sha256.sh` 校验官方脚本摘要，防止本地脚本漂移。QEMU 参数固定为 8 核、8 GiB，并以 `-snapshot` 保护镜像。串口日志由 `scripts/qemu_log_wait.py` 同时检查成功条件和 panic、环境失败、编译失败条件。

每轮优化至少执行：

```sh
make all
make final-cagent-rv FINAL_IMAGE_RV=/Volumes/U/os/sdcard-rv-pub.img
make final-cagent-la FINAL_IMAGE_LA=/Volumes/U/os/sdcard-la-pub.img
make final-buildstorm-rv FINAL_IMAGE_RV=/Volumes/U/os/sdcard-rv-pub.img
make final-buildstorm-la FINAL_IMAGE_LA=/Volumes/U/os/sdcard-la-pub.img
```

同时扫描日志中的 `panicked`、`sigsegv`、`ALLOC-FAIL`、`COPY-FAIL`、`ZERO-FAIL`、超时和失败评分标志。只有双架构均通过的改动才保留。

开发过程使用了 OpenAI Codex 辅助源码检索、故障假设、补丁生成、回归执行和文档整理。团队成员负责审阅和提交，完整边界见仓库根目录的 [`AI-使用声明.md`](../AI-使用声明.md)。复现者不需要 AI 工具：上述 Make 入口、固定镜像、完整串口日志和 `git diff --check` 足以独立核验结果。

## 3. 正确性前置：RISC-V 浮点上下文

### 3.1 现象

8 路并行 rustc 在 RISC-V 上出现非确定性 SIGSEGV，单核或轻量程序不稳定复现。故障地址和触发 crate 会变化，符合上下文切换破坏寄存器状态，而非固定 ELF 重定位错误的特征。

### 3.2 根因

LoongArch 任务上下文已保存浮点寄存器，而 RISC-V 调度切换只保存整数 callee-saved 寄存器。评测 rootfs 使用 hard-float glibc 和原生 rustc；编译器及动态链接库会跨抢占点保留浮点状态。任务切换遗漏 32 个 FPR 和 `fcsr`，会让一个 rustc 线程继承另一个任务的浮点状态，最终表现为随机用户态崩溃。

### 3.3 实现

- 在 `arch/riscv64/src/task/context.rs` 的任务上下文中加入 32 个 64 位浮点寄存器和 `fcsr`；
- 在 `arch/riscv64/src/task/switch.S` 中对称执行 `fsd`/`fld`，并保存、恢复 `fcsr`；
- 恢复后设置 `sstatus.FS=Dirty`，保证硬件允许后续浮点指令继续执行。

修改后，8 路真实 rustc 连续运行不再出现此前的随机 SIGSEGV；LoongArch 路径未改动。

## 4. 进程创建：vfork 共享地址空间

### 4.1 热点

Cargo 会反复以 `CLONE_VM | CLONE_VFORK` 创建短命子进程并立即 `execve`。旧实现把它当普通 fork，逐页复制 Cargo/rustc 的整个地址空间。对包含大型动态库和编译器堆的进程，这一行为同时放大页分配、内存复制、页表建立和回收成本。

### 4.2 实现与语义

- `Process` 支持以 `Arc<UserMm>` 创建独立进程对象；
- `CLONE_VM` 子进程共享父进程 MM，但仍复制文件表、目录状态和信号状态，保持进程对象语义；
- `CLONE_VFORK` 增加一次性 completion：先让子任务可运行，再阻塞父进程；
- 子进程成功 `exec` 并替换私有 MM，或从任意退出路径结束时，唤醒父进程；
- 唤醒覆盖正常退出、exec 和 fatal teardown，避免父进程永久等待。

诊断日志验证了实际 Cargo 进程树中的顺序为：父进程等待、子进程进入 exec、完成 MM 替换、父进程恢复。普通 fork 仍保留 eager-copy 语义；没有用针对测试程序的“假 vfork”改变 Linux ABI。

### 4.3 jemalloc `mremap` 快路径

真实 8 路 rustc 运行中反复出现 asm-generic syscall 216。它是 jemalloc 扩展匿名
arena 时使用的 `mremap`；旧内核把它作为未知系统调用返回，迫使分配器退回更昂贵的
重新映射路径。

实现覆盖 BuildStorm 实际使用的匿名映射语义：等长保持、尾部收缩、相邻空闲时原地
扩展，以及 `MREMAP_MAYMOVE` 下的搬迁。搬迁路径先建立临时可写映射，按 256 KiB
分批复制有效内容，再恢复原访问权限并撤销旧映射；`MREMAP_FIXED` 检查目标范围和
重叠，文件映射、`DONTUNMAP` 等尚未安全实现的组合明确返回错误。

RISC-V 真实 tgoskits/rustc 诊断运行记录了至少 14 次成功调用，其中既有
`mode=grow-in-place`，也有 `mode=move`，且之后未再出现 `unknown-syscall: nr=216`。
这不是针对评分标志的旁路，而是通用匿名虚拟内存 ABI 的补全。

## 5. ext4 读路径：分层不可变缓存

评测镜像是只读底图，所有写入进入内存 overlay。因此底图 inode 元数据和文件数据在一次启动内不可变，可以安全缓存，且不需要复杂的失效协议。

### 5.1 元数据缓存

`Ext4FileSystem` 增加受 IRQ-safe VFS 锁保护的缓存：

- inode number → inode record；
- inode group/index → inode-table block 位置。

这样 Cargo 的目录遍历、依赖探测、`stat`/`open` 重复访问不再反复读取 superblock、group descriptor 和 inode table。缓存锁等级为 VFS 20，底层 virtio block 为 VFS 21，保持固定锁序。

### 5.2 目录缓存

`Ext4Directory` 记录 `populated` 状态，首次查询时一次性载入目录项；并发首次访问在提交前重新检查，避免重复插入。overlay whiteout 优先于底图目录项，确保删除/覆盖语义不被缓存复活。

### 5.3 文件数据缓存

数据缓存采用 256 KiB chunk，键为 `(inode, chunk_index)`，总容量上限 256 MiB：

- 命中后克隆 `Arc<[u8]>` 并立即释放元数据锁；
- miss 在锁外执行块 I/O，避免串行化其他读取；
- 并发重复填充在提交阶段合并；
- 达到容量或预留内存失败时退回原始直接读取，而不是把性能缓存失败升级为用户可见 ENOMEM。

该缓存尤其覆盖 rustc、linker 多进程重复读取的 sysroot `.so`、`.rlib` 和 Cargo 元数据。overlay 写路径与底图缓存分离；已有文件以 `O_TRUNC` 打开时直接创建零长度 overlay，不会为即将覆盖的数据先读取底图。

## 6. 批量 I/O 与串口热路径

### 6.1 批量 I/O

普通 `read`/`write` 已支持大请求，但 `pread64`、`pwrite64`、`readv`、`writev` 和 `sendfile` 原先内部按 4 KiB 循环。链接器写大目标文件时会反复获取文件锁、seek、复制和恢复位置。

这些路径统一使用最多 256 KiB 的可复用缓冲区。仍保留用户地址检查、短读短写和偏移溢出处理，但显著减少 syscall 内循环和 VFS 锁操作次数。文件私有 mmap 的 eager 读取同样以 256 KiB 批次进入用户页。

### 6.2 生产日志降噪

串口输出在 QEMU 中是同步慢路径。exec 成功、mmap/mprotect 成功、预期 ioctl 失败、pipe/socketpair 和普通回收信息只在显式诊断模式启用。panic、SIGSEGV、分配失败、文件复制失败等真实异常始终保留。

这一设计避免“为了性能关闭诊断”：正式评分去掉高频成功日志，诊断入口仍能恢复完整生命周期证据。

### 6.3 `wait4(WNOHANG)` 调度礼让

GNU `timeout` 会高频调用 `wait4(..., WNOHANG)` 轮询 Cargo 子进程。在同一 CPU
运行队列上，父进程连续得到“尚未退出”后立即再次运行，会和正在编译的子进程争用
时间片。内核保持 Linux ABI 的立即返回值不变，但在返回 0 前执行一次调度礼让，让
已就绪子任务获得运行机会。该改动只影响仍有活跃子进程且指定 `WNOHANG` 的轮询分支，
不改变阻塞 wait、退出状态或无子进程错误语义。

### 6.4 负载感知的用户任务首放置

用户任务当前在首次选核后保持绑核，以规避 RISC-V 用户返回锚点在跨核迁移窗口中的
已知风险。原选核器却只比较各 CPU 的等待队列长度，没有把该 CPU 正在运行的任务
计入负载；多个队列同为空时会反复偏向低编号 CPU。真实 rustc 的任务快照因此出现
CPU 0–4 尚有 runnable/running 用户任务而 CPU 5–7 idle 的情况。

新选核器使用“等待队列长度 + 非 idle 当前任务”作为负载，并以 CPU 编号只作同负载
时的稳定决胜。它不开放运行中迁移，不触碰 trap-anchor 风险窗口，只改善 clone 时的
一次性铺核。相同诊断阶段的 120 秒快照中，旧实现把两个待运行用户任务都留在 CPU0；
新实现已让用户工作同时运行于 CPU1、CPU2，CPU0 仅因诊断 watchdog 临时占用而保留
一个待运行任务；240 秒快照随工作量增加进一步使用到 CPU3。正式评分模式没有该
watchdog。

## 7. 双架构回归结果

截至 2026-07-30，以下改动组合均通过本地双架构构建和 CAgent：

| 门禁 | RISC-V 64 | LoongArch 64 |
|---|---:|---:|
| `make all` | PASS | PASS |
| CAgent kernel/fs 10 项 | 10/10 | 10/10 |
| BuildStorm toolchain | PASS | PASS |
| BuildStorm minibuild | PASS | PASS |
| panic / SIGSEGV / OOM | 0 | 0 |

主机侧 `myos-mm`、VFS、FDT 等单元测试共 54 项通过，smoke harness 的 pass、panic、
证据不足、无输出超时和 QEMU 退出五类故障注入也全部通过。

代表性日志位于 `artifacts/final-2026/logs/`：

- `cagent-rv-rv-bulk-io-cagent-20260730.log`
- `cagent-la-la-bulk-io-cagent-20260730.log`
- `buildstorm-rv-rv-bulk-io-natural-20260730.log`
- `buildstorm-la-la-bulk-io-natural-20260730.log`
- `buildstorm-rv-diag-rv-progress-20260730.log`
- `cagent-rv-rv-wnohang-yield-20260730.log`
- `cagent-la-la-wnohang-yield-20260730.log`
- `buildstorm-rv-diag-rv-mremap-real-20260730.log`
- `cagent-rv-rv-load-aware-cagent-20260730.log`
- `cagent-la-la-load-aware-cagent-20260730.log`
- `buildstorm-rv-diag-rv-load-aware-diag-20260730.log`
- `buildstorm-rv-diag-rv-load-aware-full-20260730.log`

### 7.1 当前可量化数据

| 对比项 | 修改前 | 修改后 | 结论 |
|---|---:|---:|---|
| RISC-V 8 核原生 rustc | 非确定性 SIGSEGV，无法稳定持续编译 | minibuild 完成并进入 tgoskits 完整构建，连续运行超过 12 分钟无 SIGSEGV；真实命中 `mremap` 原地扩容和搬迁路径 | 消除正确性阻塞；“失败到可持续运行”不虚构时间加速比 |
| RISC-V CAgent | 10/10 | 10/10 | BuildStorm 优化未损失既有分数 |
| LoongArch CAgent | 10/10 | 10/10 | 同上 |
| 7 月 27 日镜像正式路径 | toolchain/minibuild 通过，缺 `pkg-config` 时退出 | toolchain/minibuild 通过，仍在相同镜像缺包处退出 | 该终点不是完整编译性能样本，不能用于计算 BuildStorm 加速比 |

旧镜像上从计时命令开始到缺包退出存在较大宿主机波动：RISC-V 多轮为 50–71 秒，LoongArch 为 28–47 秒。由于各轮都没有进入数百 crate 的正式全量构建，把这组数字包装成“编译加速比”会误导评审，因此只作为环境波动范围保留。

完整的修改前后时间表将在新版镜像可用后补录，格式固定如下：

| 架构 | 对照版本 t0 | 当前版本 t1 | 加速比 t0/t1 | 官方/平台基线 B | 时间分 |
|---|---:|---:|---:|---:|---:|
| RISC-V 64 | 待新版镜像实测 | 待实测 | 待计算 | 1616.09 s（自检值） | 待计算 |
| LoongArch 64 | 待新版镜像实测 | 待实测 | 待计算 | 1985.21 s（自检值） | 待计算 |

7 月 27 日公开镜像缺少完整的预编译 `tg-xtask`/Cargo cache，正式路径在解析缺失的 `pkg-config` crate 时结束；这是镜像环境失败，不是内核 panic。诊断模式在不修改磁盘镜像和正式评分脚本的前提下，已完成 minibuild 并进入真实 8 路 rustc/tgoskits 编译。待官方更新镜像后，必须重新记录两架构的 `ok=true`、`elapsed_s`、`bytes` 和最终日志 hash，才能把完整编译及耗时项标记为最终 PASS。

## 8. 取舍、风险与后续优化

### 8.1 未直接启用完整 COW 的原因

底层 buddy allocator 已有页引用计数，页表也有 `replace_page`，但当前 fault handler 在关中断上下文运行。多线程进程的 COW 写故障不仅要换页，还必须让同一 MM 正运行于其他 CPU 的旧 TLB 在释放/复用物理页前完成同步失效。未经完整远程 shootdown 协议验证就启用 COW，可能制造低概率数据错误，风险高于 eager fork 的性能损失。

因此当前只优化语义明确的 `CLONE_VM | CLONE_VFORK`，普通 fork 继续可靠的 eager copy。若新版镜像的真实计时显示 fork 复制仍是主瓶颈，下一阶段才实现：引用计数 backing、父子 PTE 原子降权、写故障复制、跨 CPU TLB ACK、munmap/exit 引用释放及双架构压力测试。

### 8.2 文件按需分页

现有文件 mmap eager 填充避免在关中断 fault 路径执行 ext4/virtio I/O。真正的 file-backed demand paging 需要可睡眠 fault worker 或可重入的异步块 I/O，再把页安装与 TLB 更新带回 fault 完成路径。在此之前，以底图 chunk cache + 批量 eager copy 获得大部分重复读取收益，同时保持可验证的锁序。

### 8.3 新镜像最终验收

1. 校验两张镜像的时间、大小和摘要，保留旧镜像；
2. 双架构各跑一次正式 BuildStorm，确认完整产物与计时；
3. 重复运行至少一次，区分冷缓存和偶发调度波动；
4. 每次性能改动后重跑双架构 CAgent 10/10；
5. 仅依据真实 `elapsed_s` 决定是否继续 COW、文件 demand paging 或 per-CPU allocator，避免无证据优化。

这套实现把 BuildStorm 的性能优化落在通用内核机制上：正确的浮点调度上下文、Linux-like vfork、只读 ext4 缓存、批量 VFS I/O 和可控诊断输出。它们不依赖特定产物名或伪造评分协议，也同时改善普通原生编译和动态 glibc 用户态负载。

## 2026-07-30：BuildStorm 安全冲分补丁 v15

本轮从已恢复的稳定基线继续，不修改用户地址空间容量，不实现高风险页面回收，
不修改评测脚本、计时和评分输出。

修改内容：

- 补齐 Rust/Cargo 常见的 asm-generic syscall：
  `fallocate`、`sync_file_range`、`getcpu`、`readahead`、
  `fadvise64`、`membarrier`、`copy_file_range`；
- 扩充只作为 hint 的 `madvise` 取值，保持非破坏性行为；
- 正式 BuildStorm 环境设置 `CARGO_INCREMENTAL=0`、`TMPDIR=/tmp`；
- 使用 Cargo 官方 `CARGO_TERM_QUIET`、`CARGO_TERM_COLOR` 和
  `CARGO_TERM_PROGRESS_WHEN` 降低同步串口输出；
- 只在 BuildStorm 执行期间打印最多 32 条未知 syscall，并在脚本退出后汇总。

AI 协助完成日志分析、ABI 对照和补丁生成。真实得分必须以评测机输出的
`BUILDSTORM_COMPILE mode=multi ok=true ... elapsed_s=...` 为准。

## 2026-07-30：BuildStorm 块 I/O 与只读缓存快路径 v16

本轮从稳定基线继续，未修改 UserMm、VMA、调度时钟或官方评分脚本。

### 根因

原 VirtIO block 路径对每次 ext4 批量读取都执行：

1. 从 DMA32 buddy zone 申请 next-power-of-two 连续页；
2. 将整块内存清零；
3. 发起 VirtIO I/O；
4. 复制到调用者；
5. 归还连续页。

Rust/Cargo 干净构建需要读取大量 sysroot、crate source、rlib 和 metadata，
上述分配/清零/释放位于正式计时热路径。原 ext4 数据缓存上限只有 256 MiB，
且没有淘汰；缓存满后余下构建持续退回昂贵的直接读取路径。

### 修复

- 每个 VirtIO block 设备在初始化时申请一个 1 MiB 持久 DMA32 bounce；
- block lock 同时保护 driver 和 bounce，避免并发复用；
- 1 MiB 以内的读写不再重复申请、清零和释放 DMA 页；
- 超过 1 MiB 的请求保留原 fallback，确保兼容；
- ext4 数据 chunk 从 256 KiB 提升到 1 MiB；
- ext4 只读数据缓存从 256 MiB 提升到 2 GiB；
- RV 16 GiB、LoongArch 36 GiB 的正式配置保留充足编译内存。

AI 协助完成热点定位、实现与补丁生成。真实结果必须以评测机原始
`BUILDSTORM_COMPILE mode=multi ok=true elapsed_s=...` 为准。

