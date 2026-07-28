# SudoOS-Plus 决赛满分 Codex 极限执行计划

> **用途**：直接交给 Codex 在仓库中执行  
> **唯一代码基线**：`https://github.com/Sudo-666/SudoOS-Plus/tree/pym7.26`  
> **目标**：以最短路径让 RISC-V64 与 LoongArch64 的 CAgent、BuildStorm、初赛回归和人工文档同时达到可提交满分状态  
> **生成日期**：2026-07-27  
> **重要事实**：本文是“冲击满分的工程执行书”，不是对尚未运行结果的满分保证。任何 PASS 必须来自真实二进制、真实官方脚本、真实退出码与真实产物，禁止伪造。

---

## 0. 给 Codex 的总指令：先完整阅读，再开始修改

将下面整段作为 Codex 的第一条总任务指令。Codex 必须先输出基线报告，再修改代码。

```text
你正在修复 SudoOS-Plus，使其尽快达到 2026 OS 内核赛决赛满分。

唯一允许作为修改起点的基线：
https://github.com/Sudo-666/SudoOS-Plus/tree/pym7.26

最高目标：
1. RISC-V64 CAgent 10/10，全部获得时间奖励。
2. LoongArch64 CAgent 10/10，全部获得时间奖励。
3. RISC-V64 BuildStorm：
   - rustc/cargo 工具链通过；
   - minibuild 通过；
   - arceos-helloworld 全量构建成功；
   - 产物 >= 500000 字节；
   - 在正确计时下尽可能接近或快于 Linux 基线。
4. LoongArch64 BuildStorm 同样全部通过。
5. 不破坏初赛已有 basic、busybox、libcbench、lua 等通过项。
6. 形成完整设计优化文档、前后实验数据、AI 使用说明和复现步骤。

绝对禁止：
- 禁止 push。
- 禁止 merge、rebase、reset --hard、整分支 cherry-pick。
- 禁止把 vfs 分支整体合并进 pym7.26。
- 禁止修改官方脚本的判分输出以制造 PASS。
- 禁止直接打印 BUILDSTORM_*、testcase cagent * pass 等判分标志冒充真实执行结果。
- 禁止篡改 /proc/uptime、系统时钟或 elapsed_s。
- 禁止通过跳过编译、复用预生成最终产物或缩减正式构建范围获得成功标志。
- 禁止为了“先过”而静默吞掉未知 syscall、页错误、动态重定位错误或 I/O 错误。
- 禁止无关重构、格式化全仓库、改名大批模块。
- 禁止修改备份目录中的源码；只修改实际参与构建的目录。
- 禁止在没有运行验证的情况下声称问题已解决。

版本控制要求：
- 不执行任何远端写操作。
- 不创建无意义提交。
- 每个阶段结束保存：
  1) git diff --stat
  2) git diff
  3) 构建日志
  4) QEMU 日志
  5) 阶段报告
- 如果仓库有未提交用户修改，必须保留，不能覆盖。

工作方法：
- 永远修复“当前最早失败点”，一次只处理一个根因。
- 先建立可复现门禁，再动源码。
- 每次修改必须给出：根因、最小修改、涉及文件、验证命令、验证结果、剩余风险。
- 失败时读取首个错误前后至少 200 行日志，不凭最后 20 行猜测。
- RISC-V 修复必须检查是否需要同步 LoongArch；共享代码优先双架构一次修正。
- 所有新增诊断日志必须有预算或开关，成功后默认关闭，避免拖慢 BuildStorm。
- release 构建和真实 QEMU 配置是最终依据，debug 静态审计只能作为辅助。

执行顺序严格遵循本文档的 G0-G12 门禁。除非当前门禁通过，不得跳到后续性能优化。
```

---

# 1. 满分定义与分值优先级

## 1.1 决赛题目

决赛共两题，每题原始满分 200，折算后各占决赛总分 50%。

### CAgent：200 分

- 10 个 glibc 测试。
- 基础分共 181。
- 每项在“执行时间小于超时的 50%”时获得 10% 时间奖励。
- 全部基础分与奖励合计 200 分。

测试内容：

| 测试 | 主要内核能力 |
|---|---|
| factorial | Bash、进程执行、管道/输出 |
| date | 时钟、日期工具、文件与动态 glibc |
| network | 环回 TCP、`/proc/net` 或兼容查询 |
| cpu | SMP 拓扑、`nproc`、affinity/sysconf |
| kernel | `uname` |
| fs-create | create/open/write/close/stat |
| fs-readwrite | 文件写读、shell、awk 等 |
| fs-directory | mkdir、readdir、文件创建 |
| fs-search | 深目录遍历、find、getdents64、stat |
| fs-usage | statfs/statvfs/df/du 相关语义 |

### BuildStorm：200 分

| 项目 | 分值 | 条件 |
|---|---:|---|
| `rustc --version`、`cargo --version` | 8 | 两者真实运行 |
| `cargo new/build/run` | 12 | 输出真实 `Hello, world!` |
| 全量编译成功 | 40 | 返回 0，产物存在且 >= 500000 字节 |
| 编译速度 | 120 | 仅在全量编译成功后计算 |
| 设计优化文档 | 20 | 人工评审 |

## 1.2 最快得分顺序

严格按照收益/风险排序：

1. **先保住 RISC-V CAgent 已知通过能力。**
2. **立即打通 RISC-V BuildStorm 工具链与 minibuild。**
3. **打通 RISC-V 全量编译，拿到 BuildStorm 的 40 分门槛和速度分资格。**
4. **补 LoongArch 动态加载，复制共享层成果。**
5. **LoongArch CAgent 与 BuildStorm 全量通过。**
6. **优化速度与 CAgent 时间奖励。**
7. **初赛回归和文档 20 分。**

不要先写完整持久化 ext4。决赛单次 QEMU 运行只需要可靠、可扩展、高性能的写时覆盖层；完整 journal/bitmap/inode 回写不是最快路径。

---

# 2. 当前基线必须认清的事实

Codex 在执行前必须重新核实以下内容，不能仅相信本文。

## 2.1 已知代码事实

当前 `pym7.26` 中：

- `Makefile` 已有：
  - `final-cagent-rv`
  - `final-cagent-la`
  - `final-buildstorm-rv`
  - `final-buildstorm-la`
- BuildStorm 正式 QEMU 目标默认：
  - `FINAL_CPUS=8`
  - `FINAL_MEM=8G`
- 调度器：
  - `MAX_TASKS = 128`
  - 退出任务进入 `retired_tasks`
  - `user_join.complete_all()` 当前位于资源销毁阶段
- 文件描述符：
  - `PROCESS_MAX_FDS = 128`
- VFS：
  - 全局 `TREE` 锁
  - `BLOCK_CACHE_BLOCKS = 32`
  - ext4 普通文件首次写入会把整个 lower 文件复制到 `Vec<u8>`
- ext4 读取层：
  - `MAX_EXT4_FILE_BYTES = 256 MiB`
  - `MAX_EXT4_NODES = 65536`
- BuildStorm runner：
  - 当前安装内嵌的 `kernel/src/final_buildstorm_testcode.sh`
  - 当前设置 `CARGO_HOME=/tmp/cargo-cache`
  - 当前内嵌脚本把 `/work` 的若干状态文件改到了 `/tmp`
  - 当前内嵌正式编译 timeout 为 600 秒
- 官方公开脚本：
  - `CARGO_HOME=/root/.cargo`
  - `cd /work/tgoskits`
  - 正式脚本公开版本以 `/proc/uptime` 计时
- LoongArch：
  - 已有部分 `R_LARCH_64` 处理
  - 当前 `exec.rs` 未找到 `DT_RELR`
- 历史日志表明：
  - RISC-V CAgent 曾真实完整 PASS
  - RISC-V BuildStorm 曾进入 rustc 编译
  - 某轮子进程完成退出清理后，上层同步调用不返回
  - LoongArch 动态链接器存在 PIF/取指错误风险

## 2.2 必须立即消除的错误认知

- “有 `PT_INTERP`”不等于 glibc 动态程序完整可运行。
- “rustc 启动过”不等于 minibuild 成功。
- “minibuild 开始编译”不等于 `Hello, world!` 成功。
- “脚本打印结束”不等于 judge 判定成功。
- “ext4 可以写”可能只是内存 overlay，不等于磁盘持久化 ext4。
- `vfs` 分支公开实现的核心价值是 VFS/tmpfs 结构和测试，不应假设它能直接替换当前决赛文件系统。
- 增大内存并不能修复引用泄漏、等待丢失、地址冲突或错误 ABI。
- 关闭 panic 只能隐藏问题，不能构成修复。

---

# 3. 总体门禁图

```text
G0 基线可复现
  ↓
G1 官方脚本与判分环境一致
  ↓
G2 退出/等待/回收闭环稳定
  ↓
G3 RV CAgent 10/10 稳定
  ↓
G4 RV rustc/cargo 工具链
  ↓
G5 RV minibuild 完整成功
  ↓
G6 RV BuildStorm 全量编译成功
  ↓
G7 LA 动态 glibc 基础程序
  ↓
G8 LA CAgent 10/10
  ↓
G9 LA rustc/cargo + minibuild
  ↓
G10 LA BuildStorm 全量编译成功
  ↓
G11 双架构性能优化
  ↓
G12 初赛回归 + 文档 + 最终打包
```

**规则：某个 Gate 未通过时，只允许为诊断该 Gate 做必要修改。**

---

# 4. G0：冻结和重建可信基线

## 4.1 目标

在修改任何源码前，确定：

- 当前真实 HEAD
- 工作区状态
- 参与构建的真实源码目录
- 工具链
- 两架构构建是否通过
- 当前四个决赛目标的真实日志
- 当前失败的最早位置

## 4.2 Codex 操作

在仓库根目录执行并保存到 `artifacts/fullscore/baseline/`：

```bash
set -euxo pipefail

mkdir -p artifacts/fullscore/baseline

date -Iseconds | tee artifacts/fullscore/baseline/date.txt
git rev-parse HEAD | tee artifacts/fullscore/baseline/head.txt
git branch --show-current | tee artifacts/fullscore/baseline/branch.txt
git status --short | tee artifacts/fullscore/baseline/status.txt
git log -20 --oneline --decorate | tee artifacts/fullscore/baseline/log20.txt
git diff --stat | tee artifacts/fullscore/baseline/diff-stat.txt
git diff > artifacts/fullscore/baseline/preexisting.diff

rustc --version | tee artifacts/fullscore/baseline/host-rustc.txt || true
cargo --version | tee artifacts/fullscore/baseline/host-cargo.txt || true
qemu-system-riscv64 --version | tee artifacts/fullscore/baseline/qemu-rv.txt || true
qemu-system-loongarch64 --version | tee artifacts/fullscore/baseline/qemu-la.txt || true
```

确认构建实际不读取备份目录：

```bash
grep -R "sudoos-direct-fix-backup\|sudoos-next-fix-backup" \
  Cargo.toml Makefile Makefile.project kernel arch mm runtime sync vfs scripts \
  > artifacts/fullscore/baseline/backup-reference.txt || true
```

构建：

```bash
make all 2>&1 | tee artifacts/fullscore/baseline/make-all.log
test -s kernel-rv
test -s kernel-la
sha256sum kernel-rv kernel-la | tee artifacts/fullscore/baseline/kernel-sha256.txt
```

静态门禁：

```bash
make oscomp-audit 2>&1 | tee artifacts/fullscore/baseline/oscomp-audit.log || true
make oscomp-newtest-full-audit 2>&1 | tee artifacts/fullscore/baseline/newtest-audit.log || true
make oscomp-full-contest-preflight 2>&1 | tee artifacts/fullscore/baseline/preflight.log || true
```

## 4.3 当前测试矩阵

优先真实平台镜像。若本地镜像路径存在：

```bash
RUN=$(date +%Y%m%d-%H%M%S)

make final-cagent-rv FINAL_RUN_ID=$RUN \
  2>&1 | tee artifacts/fullscore/baseline/final-cagent-rv-$RUN.host.log || true

make final-buildstorm-rv FINAL_RUN_ID=$RUN \
  2>&1 | tee artifacts/fullscore/baseline/final-buildstorm-rv-$RUN.host.log || true

make final-cagent-la FINAL_RUN_ID=$RUN \
  2>&1 | tee artifacts/fullscore/baseline/final-cagent-la-$RUN.host.log || true

make final-buildstorm-la FINAL_RUN_ID=$RUN \
  2>&1 | tee artifacts/fullscore/baseline/final-buildstorm-la-$RUN.host.log || true
```

如果完整 BuildStorm 太慢，仍要先跑到第一个确定失败点；不得只跑 30 秒就猜根因。

## 4.4 基线报告格式

创建 `artifacts/fullscore/baseline/REPORT.md`：

```markdown
# Baseline Report

- Branch:
- HEAD:
- Working tree:
- Toolchain:
- RV build:
- LA build:
- RV CAgent:
- LA CAgent:
- RV BuildStorm:
  - toolchain:
  - minibuild:
  - full build:
  - earliest failure:
- LA BuildStorm:
  - toolchain:
  - minibuild:
  - full build:
  - earliest failure:
- Existing user changes preserved:
- No remote write performed: yes/no
```

## 4.5 G0 通过标准

- `make all` 成功。
- `kernel-rv`、`kernel-la` 均非空。
- 四项测试至少都有一份当前 HEAD 的日志；不能用旧日志代替。
- 明确记录每项“最早失败点”。
- 没有修改源码。

---

# 5. G1：恢复官方脚本和判分环境一致性

## 5.1 根因

当前 BuildStorm runner 使用修改过的内嵌脚本和自定义环境。这样可以帮助诊断，但不能证明正式官方脚本会成功，也可能掩盖：

- `/root/.cargo` 写语义
- `/work` 写语义
- 正式 timeout
- 正式路径解析
- 正式输出格式

## 5.2 最快正确方案

### 生产模式

生产 `final-buildstorm` 必须满足二选一：

1. 直接执行镜像内官方脚本；或
2. 使用一份与官方公开脚本**字节一致**的 vendored 副本。

若使用副本：

- 文件名明确写 `official`
- 顶部记录来源 commit/URL
- 加入 SHA256 检查脚本
- 不允许在该文件中改路径、timeout、判分标志

### 诊断模式

可保留自定义诊断，但必须：

- 仅在单独 boot arg 下启用，例如：
  - `sudoos.oscomp=final-buildstorm-diag`
- 不打印正式判分 PASS 标志
- 不影响生产模式
- 不覆盖官方输出文件
- 成功后默认关闭

## 5.3 需要检查和修改的文件

优先检查：

```text
kernel/src/user.rs
kernel/src/final_buildstorm_testcode.sh
Makefile
scripts/qemu_log_wait.py
kernel/src/main.rs
```

## 5.4 具体改动

1. 将 `kernel/src/final_buildstorm_testcode.sh` 替换为官方公开脚本原文。
2. 新增：

```text
scripts/verify-final-script-sha256.sh
```

脚本做：

```bash
sha256sum kernel/src/final_buildstorm_testcode.sh
# 与仓库记录的官方 SHA 比对
```

3. `verify_final_buildstorm_thread()` 中：
   - 不设置 `CARGO_HOME=/tmp/cargo-cache`
   - 环境使用：
     - `HOME=/root`
     - `RUSTUP_HOME=/root/.rustup`
     - `CARGO_HOME=/root/.cargo`
   - 保证 `/root/.cargo` 在现有 overlay 上可写。
   - 保证 `/work` 在现有 overlay 上可写。
4. `/tmp` 保持独立可写 tmpfs。
5. `mount proc/sysfs/devtmpfs` 允许脚本执行；重复 mount 应返回 Linux-like 合理结果，而不是 panic。
6. 正式 runner 不先执行会改变缓存状态的 minibuild diagnostic。
   - 原因：诊断构建可能预热 cache，影响时间与正式语义。
7. 单独 diagnostic 模式才跑额外 minibuild。
8. Makefile 正式 gate 按公开判分标志等待。

## 5.5 写语义测试

在启动正式脚本前，可在不污染正式构建目录的 preflight 中验证：

```sh
test -d /root/.cargo
test -d /work/tgoskits
test -r /root/.cargo/bin/cargo
test -x /root/.cargo/bin/cargo

echo x > /root/.cargo/.sudoos-write-probe
cat /root/.cargo/.sudoos-write-probe
rm /root/.cargo/.sudoos-write-probe

echo x > /work/.sudoos-write-probe
cat /work/.sudoos-write-probe
rm /work/.sudoos-write-probe
```

preflight 失败时输出明确 errno 和路径，但不得打印正式 `BUILDSTORM_* ok`。

## 5.6 G1 通过标准

- 官方脚本副本 SHA 校验通过。
- 生产 runner 不改写官方环境。
- `/root/.cargo`、`/work`、`/tmp` 均真实可写。
- 诊断模式与生产模式完全分离。
- `make all` 双架构通过。
- CAgent 不受影响。

## 5.7 本阶段 Codex 专用提示词

```text
只执行 G1。不要修调度器，不要修 ELF，不要优化 VFS。

目标：
- 让生产 final-buildstorm 使用与官方公开脚本字节一致的脚本和正式环境。
- 将现有自定义 minibuild 迁到独立 final-buildstorm-diag 模式。
- 保证 /root/.cargo、/work、/tmp 真实可写。
- 不制造任何判分标志。
- 保留现有用户修改，不 push、不 merge、不提交。

完成后输出：
1. 修改文件列表。
2. 官方脚本 SHA256。
3. 生产与诊断路径差异。
4. 双架构 make all 结果。
5. 写路径 preflight 结果。
6. git diff --stat。
7. 剩余最早失败点。
```

---

# 6. G2：彻底修复退出、等待、完成通知与异步回收

## 6.1 这是当前最高优先级内核问题

历史确定性症状：

- 子进程用户态已退出。
- close-files、robust-list、clear_child_tid 已完成。
- 调度器完成退出切换。
- 上层 `run_rootfs_program_with_cwd()` 或 runner 没有返回。

当前结构风险：

- `Task.user_join` 直到 `destroy_resources()` 才 `complete_all()`。
- `destroy_resources()` 依赖 reaper。
- “用户任务退出可见”和“内核资源彻底回收”被绑定。
- reaper 调度、栈销毁、引用释放任一延迟都可能让同步执行者卡住。

## 6.2 正确状态机

必须明确实现：

```text
Running
  -> Exiting
  -> SwitchedOut
  -> ExitVisible
  -> Retired
  -> Reclaiming
  -> Reclaimed
```

### ExitVisible 的含义

在这一时刻：

- 退出任务已经不在任何 CPU 上运行。
- 不会再访问自己的内核栈。
- exit status 已提交。
- 父进程/wait4/同步 runner 可观察退出。
- task 对象和内核栈可以仍待 reaper 回收。

### Reclaimed 的含义

- 内核栈释放。
- Task 对象释放。
- user Thread 强引用释放。
- 相关 MM/TLB 生命周期处理完成。
- 资源统计回到基线。

## 6.3 最小修改策略

不要大改调度器。

推荐：

1. 将现有 `user_join` 语义明确改为 `exit_visible_completion`。
2. 在 `complete_switch()` 的 `SwitchDisposition::Exit` 分支中：
   - 旧任务已完成上下文切换后；
   - 在把任务交给 retired/reaper 前或后；
   - 安全地调用 `exit_visible_completion.complete_all()`。
3. `destroy_resources()` 不再负责唤醒等待退出的调用者。
4. 如调试/测试需要，新增独立 `reclaim_completion`，但生产路径一般不需要上层等待彻底回收。
5. `run_program_image_with_cwd()` 等同步调用只等待 ExitVisible 和 Process exit status，不等待栈销毁。
6. reaper 继续异步回收。
7. 避免 completion 对 Task/Thread/Process 形成强引用环。
8. 明确 Acquire/Release：
   - exit status 和线程状态先 Release 发布；
   - completion wake 后等待者 Acquire 读取。
9. completion 不能在仍运行于旧任务栈时触发导致旧任务对象被立即释放。
10. wake 时不能持有 scheduler 锁进入可能重新调度的路径。

## 6.4 必须审计的函数

Codex 全文追踪：

```text
Task::destroy_resources
Scheduler::complete_switch
Scheduler::prepare_exit / exit_current
reap_retired_tasks
task reaper 主循环
run_kernel_thread_sync
spawn_user_task 或等价函数
run_program_image_with_cwd
wait4
Thread/Process exit status 发布
Completion::wait
Completion::complete_all
WaitQueue::wake_*
clear_child_tid_on_exit
cleanup_robust_list_on_exit
```

## 6.5 诊断计数器

短期加入低成本原子计数器，成功后保留统计但关闭高频日志：

```text
tasks_spawned
tasks_exit_visible
tasks_retired
tasks_reclaimed
join_wait_begin
join_wait_end
retired_backlog
retired_outstanding
live_user_threads
live_kernel_threads
live_processes
```

每个正式脚本结束只打印一行摘要，不逐任务刷串口：

```text
task-lifecycle-summary spawned=... visible=... retired=... reclaimed=... backlog=... outstanding=...
```

正式性能模式可关闭。

## 6.6 必做压力测试

新增可由 boot arg 启动的内部压力测试，不能影响正式比赛路径。

### T1：顺序短进程

```text
重复 10000 次：
  /bin/true
  等待退出
```

### T2：shell 短命令

```text
重复 2000 次：
  /bin/sh -c 'exit 0'
```

### T3：管道

```text
重复 2000 次：
  /bin/sh -c 'echo x | cat >/dev/null'
```

### T4：并发退出

```text
每轮并发 64 个 /bin/true
重复 200 轮
```

### T5：信号退出

- SIGTERM
- SIGKILL
- timeout 工具杀子进程
- server 后台进程被 kill

### T6：clone/futex

- 线程创建
- `CLONE_CHILD_CLEARTID`
- futex wake
- group exit

## 6.7 不变量

每轮结束必须断言或记录：

```text
spawned == exit_visible
retired_backlog 最终为 0
retired_outstanding 最终为 0
live_user_threads 回到初值
live_processes 回到初值
内核页空闲数不持续下降
没有 waiter 永久 Blocked
没有 exited task 留在 run_queue
没有已退出 task 仍是 cpu.current
```

## 6.8 G2 通过标准

- 历史 BuildStorm diagnostic 子进程退出后，调用点必定打印返回码。
- 上述压力测试全部通过。
- RV SMP=1、2、8 均通过。
- LA 至少静态构建和小规模运行通过。
- CAgent 连续 10 次全部结束。
- 没有 panic、hang、引用泄漏增长。
- reaper 是否及时运行不再影响父进程看到退出。

## 6.9 本阶段 Codex 专用提示词

```text
只执行 G2，专注退出/等待/回收闭环。

你必须先画出当前调用链：
run_rootfs_program_with_cwd
-> 用户任务 spawn
-> 用户任务 exit
-> complete_switch
-> retired queue
-> reaper
-> Completion
-> 调用者返回

根据真实源码给出引用和锁关系，再做最小修改。
目标是把 ExitVisible 与 Reclaimed 分开。
禁止提前销毁仍在使用的内核栈。
禁止通过 busy-wait、无限 yield 或增加 timeout 掩盖丢失唤醒。
禁止全局关闭 lockdep。
禁止修改官方脚本。

必须新增生命周期压力测试和汇总计数。
完成后运行双架构 make all、RV SMP 1/2/8 压力、CAgent 连续测试。
不要 push，不要 merge，不要提交。
```

---

# 7. G3：锁死 RISC-V CAgent 200/200

## 7.1 目标

不是“脚本 PASS 一次”，而是：

- 10/10 每次通过。
- 连续 20 轮无 reject。
- 每项尽量低于奖励阈值。
- 没有后台任务、server、文件或 FD 泄漏。

## 7.2 判定解析

为 CAgent 新增 host 侧严格解析脚本：

```text
scripts/final-cagent-gate.py
```

输入串口日志，检查：

- START/END 均存在且顺序正确。
- 10 个测试名各出现一次。
- pass=10。
- reject=0。
- 无 panic。
- 无 OOM。
- 无 watchdog timeout。
- server 被正常终止。
- summary 完整。
- 每项 duration。
- 输出基础分估算和时间奖励估算。

不得只 grep 一个 END 标志。

## 7.3 每项失败分流

### factorial

检查：

- bash fork/exec/wait
- pipe
- stdout 重定向
- integer shell tool

### date

检查：

- `clock_gettime`
- `gettimeofday`
- realtime epoch 是否合理
- timezone 文件读取
- `date +%s%3N`
- 64 位时间结构体布局

### network

检查：

- `simple_llm_server` bind/listen
- agent connect
- accept
- read/write/poll
- localhost 地址
- TCP established 状态查询
- `ss` 或 `/proc/net/tcp` 兼容

最快方案：若用户态 `ss` 依赖 `/proc/net/tcp`，实现正确的最小 procfs 表，不要伪造固定连接数。

### cpu

检查：

- `uname -m`
- `nproc`
- `sched_getaffinity`
- `/proc/cpuinfo` 或 sysconf
- 返回实际可用 CPU 数

### kernel

`uname` 至少：

```text
sysname=Linux
release=6.x
machine=riscv64
version 包含合理 SMP 字段
```

### fs-create/readwrite/directory/search

检查：

- openat
- O_CREAT/O_TRUNC/O_APPEND
- write/read/lseek
- close
- mkdirat
- getdents64 offset 语义
- stat/newfstatat
- unlinkat
- symlink 跟随
- cwd
- 并发目录操作

### fs-usage

检查：

- statfs/fstatfs
- block size
- total/free/available
- 不返回溢出或全 0 导致工具异常

## 7.4 时间奖励优化

CAgent 时间主要受：

- 串口日志
- 进程创建/退出
- 文件全局锁
- 目录递归
- loopback poll/wakeup
- timeout/sleep 精度

操作：

1. 关闭每 syscall/每 exec/每页错误日志。
2. CAgent 启动前不做大规模无关 ext4 全树展开。
3. loopback socket wake 使用事件驱动，不靠长时间轮询。
4. shell 等待使用 wait queue，不 10ms/100ms 周期轮询。
5. `nanosleep` 定时精度稳定。
6. `find` 路径按需加载目录，并缓存 dentry。

## 7.5 G3 通过标准

- 20 轮 RISC-V CAgent：
  - 200 个 testcase 全 pass
  - 0 reject
  - 0 panic
  - 0 hang
- 每轮结束：
  - task/process/fd 计数回落
  - 临时文件清理
- 奖励时间估算达到 200/200；若未达到，单独列最慢测试。

---

# 8. G4：RISC-V 工具链 8 分

## 8.1 顺序测试

不要一上来跑全编译。按以下命令逐个 gate：

```sh
/bin/true
/bin/echo hello
/bin/sh -c 'echo hello'
/lib64/ld-linux-riscv64-lp64d.so.1 --help
/root/.cargo/bin/rustc --version
/root/.cargo/bin/cargo --version
```

每个命令记录：

- exec path
- interpreter path
- exit code
- signal
- 缺失 syscall
- page fault 地址和 VMA
- 动态重定位统计
- live process 回收结果

## 8.2 动态 ELF 审计

检查：

- ET_EXEC
- ET_DYN
- `PT_LOAD`
- `PT_INTERP`
- 主程序 load bias
- interpreter load bias
- 不重叠 VMA
- `AT_PHDR`
- `AT_PHENT`
- `AT_PHNUM`
- `AT_BASE`
- `AT_ENTRY`
- `AT_PAGESZ`
- `AT_RANDOM`
- `AT_EXECFN`
- uid/gid
- platform/hwcap
- 用户栈 16 字节对齐
- 初始寄存器 ABI
- TLS

## 8.3 关键 syscall 收集

对 `rustc --version` 开启**有预算的未知 syscall 日志**：

```text
unknown-syscall arch=... nr=... count=...
```

同一 syscall 最多打印 3 次，最后汇总次数。

优先修真实返回错误的 syscall，不要对所有未知 syscall 返回 0。错误返回应遵循 Linux：

- 不支持：`-ENOSYS`
- 参数错误：`-EINVAL`
- 地址错误：`-EFAULT`
- 资源不足：`-ENOMEM`/`-EAGAIN`

## 8.4 G4 通过标准

正式官方脚本出现真实：

```text
BUILDSTORM_TOOLCHAIN ok
```

且：

- rustc/cargo 均打印正确版本
- 退出码 0
- 无 signal
- 无泄漏增长
- 连续 20 次通过

---

# 9. G5：RISC-V minibuild 12 分

## 9.1 正式链路

官方行为：

```sh
rm -rf /tmp/minibuild
cargo new --vcs none /tmp/minibuild
cd /tmp/minibuild
cargo build
/tmp/minibuild/target/debug/minibuild
```

## 9.2 逐层 gate

### M1：创建项目

检查：

- mkdir
- create/write Cargo.toml
- create/write src/main.rs
- fsync/sync 可合理返回
- rename
- chmod/fchmodat
- stat

### M2：依赖解析

检查：

- `/root/.cargo` 可写
- Cargo config 可读
- 离线 mode
- lock file create
- directory traversal
- canonicalize/readlink

### M3：rustc 编译

重点：

- 大型动态库 mmap
- clone 线程
- futex
- thread-local storage
- signals
- pipes/socketpair
- poll
- eventfd
- memfd 如被使用
- file mmap
- truncate
- temporary files
- rename atomicity

### M4：链接

重点：

- 大量 open FD
- mmap 输入文件
- sparse/large output
- lseek
- pwrite/read
- rename
- executable permission

### M5：运行产物

必须真实输出：

```text
Hello, world!
```

## 9.3 优先修复项

### 提高上限

先把明显过低的固定上限安全提高：

```text
MAX_TASKS: 128 -> 至少 1024
PROCESS_MAX_FDS: 128 -> 至少 1024
```

更稳妥：

- `MAX_TASKS=4096`
- `PROCESS_MAX_FDS=4096`

注意：

- 不能在栈上生成巨大数组。
- FileTable 若 const generic 直接内嵌大数组，评估每进程内存放大。
- 优先将 FD 表改成动态 `Vec<Option<FdEntry>>` 或分段表。
- 任务表分配失败返回 `EAGAIN`，不能 panic。
- FD 分配失败返回 `EMFILE`，不能 panic。

最快折中：

1. 若 FileTable 是 heap 上 Box/Vec，直接提高。
2. 若每 Process 内嵌固定大数组，先改动态增长。
3. 默认 soft limit 1024，hard limit 4096。

### futex

至少正确：

- WAIT
- WAKE
- WAIT_BITSET
- WAKE_BITSET
- PRIVATE_FLAG
- timeout
- compare-before-sleep
- 原子入队，防 lost wakeup
- `CLONE_CHILD_CLEARTID` wake
- robust owner-died 基本处理

按日志再补：

- REQUEUE
- CMP_REQUEUE
- WAKE_OP

### clone

正确处理：

- CLONE_VM
- CLONE_FS
- CLONE_FILES
- CLONE_SIGHAND
- CLONE_THREAD
- CLONE_SETTLS
- CLONE_PARENT_SETTID
- CLONE_CHILD_SETTID
- CLONE_CHILD_CLEARTID
- 用户栈
- TLS 寄存器
- tid 写回
- group exit

### mmap

必须支持：

- anonymous/private
- file/private
- file/shared 所需最小语义
- fixed/noreplace 如被调用
- munmap 子区间拆分
- mprotect
- brk
- EOF 最后一页补零
- 页错误按 offset 读取
- fork COW 或正确复制
- executable mappings

## 9.4 G5 通过标准

官方脚本真实输出：

```text
BUILDSTORM_MINIBUILD ok
```

且：

- 产物是真实新编译。
- 运行输出恰为 Hello, world!
- 连续 20 次通过。
- 无 task/fd/mm 泄漏增长。
- 不依赖先跑自定义预热构建。

---

# 10. G6：RISC-V BuildStorm 全量编译成功

## 10.1 这是最大正确性门槛

只有全量编译成功，才获得：

- 40 分编译成功分
- 120 分速度分资格

## 10.2 运行纪律

每次全量运行前记录：

```bash
git rev-parse HEAD
sha256sum kernel-rv
sha256sum 官方镜像
nproc
QEMU 参数
开始时间
```

QEMU 最低兼容 gate：

```text
-smp 8 -m 8G
```

若平台已明确使用更大内存/更多核，再增加平台同配置测试，但不能只在大内存下验证。

## 10.3 构建阶段分段

官方脚本包含：

1. toolchain
2. minibuild
3. `cargo build -p tg-xtask`
4. 清理目标架构目录
5. `cargo xtask arceos build ...`
6. 找产物
7. 产物大小检查

为诊断建立阶段标记，但不修改判分脚本：

```text
PHASE tg-xtask-start/end
PHASE resolve
PHASE compile
PHASE link
PHASE artifact-check
```

可以由内核/host 日志分析推断，不能伪造官方结果。

## 10.4 首错分类

### A. ENOENT

检查：

- lazy ext4 lookup
- symlink
- cwd
- interpreter
- deep path
- dir entry 缓存
- 路径长度

### B. ENOSYS

按 syscall 编号和调用程序排序统计。

### C. EINVAL

重点：

- mmap flags
- futex op
- clone flags
- fcntl
- ioctl
- prlimit
- sched affinity
- statx mask

### D. EFAULT / page fault

输出：

```text
pid tid pc fault_addr access
VMA start/end/flags/kind/file_offset
mapping owner
```

先判断：

- 地址是否应有 VMA
- VMA 是否重叠
- 页是否已映射
- 权限是否正确
- 文件 offset 是否正确
- TLB 是否刷新
- fork/exec 后是否使用旧 MM

### E. OOM

记录：

- total pages
- free pages
- allocated anon
- file cache
- ext4 overlay
- kernel stacks
- page tables
- process count

### F. hang

周期性 watchdog 只打印摘要：

```text
uptime
current tasks
blocked reason
runqueue per cpu
retired backlog
futex waiters
I/O pending
last userspace progress
```

不得用 watchdog 强行判成功。

## 10.5 VFS 正确性优先项

### renameat2

至少：

- flags=0
- RENAME_NOREPLACE
- RENAME_EXCHANGE（若实际调用）
- 源目标同目录和跨目录
- 替换普通文件
- 目录非空限制
- 原子可见性

### unlink-open-file

必须满足：

```text
fd=open(path)
unlink(path)
path lookup -> ENOENT
fd 仍可 read/write
close(fd) 后对象释放
```

### getdents64

- 正确 `d_reclen`
- 对齐
- `d_type`
- 目录 position/cookie
- 多次小 buffer 调用不会重复或漏条目

### metadata

- inode 稳定
- mode/type
- size
- nlink
- mtime/ctime 至少单调合理
- statx mask
- symlink lstat/stat 区分

## 10.6 文件覆盖层：先正确再优化

当前整个文件首次写入复制是慢，但先确认能全编译。

若全量编译因内存放大失败，立即进入页级 overlay，见 G11；否则先取得第一次成功日志，再优化。

## 10.7 G6 通过标准

官方脚本真实输出：

```text
BUILDSTORM_COMPILE mode=multi ok=true ...
```

并满足：

- `rc=0`
- `cores` 正确
- `bytes >= 500000`
- 产物来自本轮构建
- `/proc/uptime` 真实
- 连续 3 次成功
- 失败重跑率为 0
- 不依赖增量 target；正式脚本会清理目标架构目录

首次成功后立即保存：

```text
artifacts/fullscore/rv-first-success/
  head.txt
  kernel-sha256.txt
  image-sha256.txt
  qemu-command.txt
  serial.log
  host.log
  buildstorm.build.out.tail.txt
  metrics.md
  diff.patch
```

---

# 11. G7：LoongArch 动态 glibc 完整链

## 11.1 不要先跑 Cargo

严格按顺序：

```text
ld.so --help
/bin/true
/bin/echo hello
/bin/sh -c 'echo hello'
simple_llm_server --help 或启动/退出
agent_lite --help
rustc --version
cargo --version
```

## 11.2 DT_RELR 必须实现

在 `kernel/src/exec.rs` 中加入并正确处理：

```text
DT_RELR
DT_RELRSZ
DT_RELRENT
```

### RELR 解码规则

对每个机器字：

- 最低位为 0：
  - 该值是一个相对重定位地址。
  - 对 `load_bias + entry` 所指位置加 load bias。
  - 更新当前位置为该地址后的一个字长。
- 最低位为 1：
  - 剩余位是 bitmap。
  - bitmap 每个置位表示从当前位置开始对应机器字需要相对重定位。
  - 展开后当前位置推进 `(word_bits - 1) * word_size`。
- 检查：
  - 表大小是 entry size 的倍数。
  - 地址加法无溢出。
  - 目标落在可写/可重定位映射中。
  - 不越用户地址空间。
  - 不重复错误应用。

不要直接复制未经理解的补丁。为 RELR 解码写纯函数单元测试：

- 单地址 entry
- 单 bitmap
- 混合 entry
- 空表
- 越界
- 溢出
- 非法 size

## 11.3 其他 LoongArch 关键项

检查：

- ELF machine
- R_LARCH_RELATIVE
- R_LARCH_64
- symbol=0 与非 0
- RELA addend
- PLT/JMPREL
- TLS program headers
- `CLONE_SETTLS`
- 用户 tp 寄存器
- FPU enable/保存/恢复
- 用户栈 ABI
- syscall 参数和返回寄存器
- signal frame
- icache 同步
- TLB invalidate
- executable page 权限

## 11.4 VMA 地址策略

固定 load bias 容易碰撞。

至少保证：

- 主 PIE
- interpreter
- stack
- heap/brk
- mmap 区
- signal trampoline
- vdso（若有）
- TLS

互不重叠。

最快方案：

- 实现一个从用户 mmap 区分配 gap 的函数。
- interpreter 和匿名 mmap 使用同一冲突检测。
- 每次创建 VMA 前检查全范围。
- 失败返回明确 `ENOMEM`，不覆盖旧 VMA。

## 11.5 PIF 诊断

LoongArch 取指异常打印一次完整信息：

```text
pid/tid
ERA/PC
BADV
ESTAT/ECODE/ESUBCODE
当前 MM/ASID
PC 所属 VMA
VMA flags
页表叶 PTE
目标物理地址
最近 exec image/interpreter
```

同时用 ELF 工具在 host 上确认故障 PC 对应：

- ld.so
- 主程序
- PLT/GOT
- RELR 目标
- 错误 load bias

## 11.6 G7 通过标准

LoongArch 连续运行：

```text
ld.so --help    20 次
/bin/true       1000 次
/bin/sh -c true 500 次
rustc --version 20 次
cargo --version 20 次
```

全部退出 0，无 PIF、无页错误、无泄漏。

---

# 12. G8：LoongArch CAgent 200/200

复用 RISC-V gate，不允许 LoongArch 特判绕过正式脚本。

## 12.1 必须重点检查

- bash 动态加载
- loopback TCP
- 多核 12 CPU 情况下 `MAX_CPUS`
- FPU 上下文
- 用户 tp
- `date +%s%3N`
- timeout 信号
- 并发 agent waitpid
- server kill
- `/proc/net/tcp`
- `nproc` 返回评测可用 CPU 数

## 12.2 G8 通过标准

- 连续 20 轮 10/10。
- 0 reject。
- 时间奖励估算 200/200。
- SMP=8 必过。
- 如平台为 SMP=12，再额外 SMP=12 必过。
- 无架构专用 fallback 替换真实 CAgent 程序。

---

# 13. G9：LoongArch 工具链与 minibuild

完整复制 G4/G5，但不能假设“共享代码修过 RV 就自动正确”。

重点：

- LoongArch rustc 动态库
- TLS
- pthread
- atomic/libatomic
- linker
- mmap executable
- clone/futex
- signal
- large file I/O

## G9 通过标准

官方脚本真实：

```text
BUILDSTORM_TOOLCHAIN ok
BUILDSTORM_MINIBUILD ok
```

连续 10 次。

---

# 14. G10：LoongArch BuildStorm 全量编译

执行 G6 同样流程。

## 14.1 平台矩阵

最低公开兼容门：

```text
-smp 8 -m 8G
```

若实际评测平台明确：

```text
LoongArch -smp 12 -m 36G
```

则最终还必须按该配置跑。

## 14.2 G10 通过标准

- 真实全量构建成功。
- 产物 >= 500000。
- 连续 3 次。
- 无 PIF、OOM、hang。
- 正确 `cores`。
- 记录编译时间。

---

# 15. G11：速度满分优化

**只有 RV 和 LA 都至少有一次全量成功后才进入本阶段。**

## 15.1 先测量

不得凭感觉优化。

新增低开销 phase metrics：

```text
exec count
clone count
context switches
futex wait/wake
page faults
file reads bytes/ops
file writes bytes/ops
block reads
block cache hits/misses
dentry hits/misses
ext4 lower reads
overlay dirty bytes/pages
peak live tasks
peak fds
peak allocated pages
```

正式判分模式默认不逐项打印，仅结束摘要。

## 15.2 第一优先：关闭日志

release 正式模式关闭：

- 每 syscall 日志
- 每 exec 映射日志
- 每 relocation 日志
- 每 page fault 日志
- 每进程 cleanup 日志
- 每 ext4 node 展开日志
- 每 socket read/write 日志

保留：

- 首个 fatal
- 最终摘要
- 判分脚本原始输出

## 15.3 第二优先：页级 ext4 COW overlay

### 当前问题

首次写 lower ext4 文件时整个文件复制到 `Vec<u8>`。

### 目标结构

```rust
struct Ext4Overlay {
    lower_fs: Arc<Ext4FileSystem>,
    lower_ino: u32,
    logical_size: u64,
    dirty_pages: BTreeMap<u64, Box<[u8; PAGE_SIZE]>>,
    truncated: bool,
}
```

或更高效的页索引容器。

### read

对每个范围：

1. 超过 logical_size -> EOF。
2. dirty page -> 内存。
3. clean page -> lower ext4 按需读。
4. 处理跨页。
5. 最后一页长度截断。

### write

1. 计算涉及页。
2. 对部分覆盖页：
   - 首次写先从 lower 读该页。
3. 对完整覆盖新页：
   - 不必读 lower。
4. 写入 dirty page。
5. 更新 logical_size。
6. 不复制未修改部分。

### truncate

- 缩小：
  - 更新 logical_size。
  - 删除范围外 dirty pages。
  - 最后页尾部清零。
- 扩大：
  - 逻辑零填充，不立刻分配全部页。

### mmap

文件私有映射应从 lower/overlay 统一读页。

### 验收

- 256MiB lower 文件修改 1 字节，overlay 新增内存接近 4KiB，而不是 256MiB。
- Cargo 全编译结果不变。
- 峰值内存明显下降。
- 编译时间改善或不退化。

## 15.4 第三优先：block/page cache

当前 32 块过小。

策略：

- 先统计命中率。
- 增至合理数量，例如 4096-16384 块，结合内存。
- LRU/clock。
- 不在 IRQ spinlock 中执行慢块 I/O。
- 连续块合并读。
- 可对源码和 `.rlib/.rmeta` 做 read-ahead。
- cache key 包含设备和块号。
- 正确处理 eviction。

## 15.5 第四优先：VFS 锁拆分

当前全局 `TREE` 锁可能让 8/12 核 Cargo 串行化。

优化顺序：

1. 路径 lookup 只读不拿全局写锁。
2. 每目录 node 锁保护 children/whiteouts。
3. rename 同时锁两个目录时统一 inode 顺序。
4. block I/O 不持全局 TREE 锁。
5. 文件 read/write 不持目录锁。
6. cache 查找用更细锁。

先用锁等待统计确认，不要盲改。

## 15.6 第五优先：调度/futex

- 每 CPU runqueue 已存在，检查全局 scheduler 锁争用。
- 减少无意义 IPI。
- futex 按 `(mm_id, user_address)` 哈希分桶。
- wake 不扫描所有任务。
- timeout 使用 timer queue。
- 避免周期性轮询。
- timeslice 进行 A/B 测试：2/4/8 ticks。
- 确保所有 CPU 真正参与 rustc 并行任务。

## 15.7 第六优先：fork/exec/mm

- exec 销毁旧 MM 不阻塞全局。
- 页表按需分配。
- fork 使用 COW，避免复制大型地址空间。
- 文件页共享只读物理页。
- TLB shootdown 只发给活跃 CPU mask。
- ASID 正确复用。

## 15.8 性能回归规则

每项优化至少跑：

- 3 次 RV
- 3 次 LA
- 取中位数
- 同一镜像、同一 QEMU、同一 host
- 记录方差

若：

- 时间退化 > 3%
- 或出现一次不稳定失败

则撤回该优化，除非它修复正确性问题。

## 15.9 速度分目标

公开公式：

```text
120 × clamp((2B - t) / B, 0, 1)
```

目标：

- `t <= B`：120 分。
- 首次成功后先争取 `t < 1.5B`。
- 再逐项靠近 `B`。

禁止修改 uptime。

---

# 16. G12：初赛回归、文档和交付

## 16.1 初赛回归

至少验证双架构 glibc/musl：

- basic
- busybox
- libcbench
- lua

若时间允许补：

- lmbench
- cyclictest
- iozone
- iperf
- netperf
- ltp/libctest

原则：

- 决赛修复不得让历史通过项下降。
- LoongArch 不能通过隐藏/跳过测试保持表面稳定。

## 16.2 设计优化文档 20 分

创建：

```text
docs/FINAL_2026_BUILDSTORM_OPTIMIZATION.md
```

必须包含：

### 1. 根因分析 6 分

至少写：

- 退出可见与资源回收耦合
- LoongArch DT_RELR/动态装载
- ext4 整文件 COW 内存放大
- VFS 全局锁
- block cache 过小
- task/fd 固定上限
- futex/clone/mmap 问题

每项附：

- 症状
- 最小复现
- 日志
- 根因时序图

### 2. 设计实现 6 分

附：

- 状态机
- 数据结构
- 锁顺序
- VMA 地址布局
- 页级 overlay
- cache
- 双架构共享边界
- 关键 diff/文件

### 3. 实验 4 分

表格：

| Arch | 版本 | Toolchain | Minibuild | Full build | 时间 | 峰值内存 | 备注 |
|---|---|---:|---:|---:|---:|---:|---|

至少：

- 修复前
- 第一次成功
- 性能优化后
- 3 次中位数
- Linux 基线

### 4. AI 使用和复现 4 分

如实说明：

- Codex 用于代码检索、假设生成、补丁草拟、测试脚本和日志归类。
- 人工做了哪些设计确认。
- 每个 AI 补丁如何 review。
- 完整复现命令。
- 工具链、镜像、commit SHA。
- 不声称 AI 自动保证正确性。

## 16.3 最终交付清单

```text
kernel-rv
kernel-la
README/设计文档
AI 使用说明
复现脚本
官方脚本 SHA
RV CAgent 最终日志
LA CAgent 最终日志
RV BuildStorm 最终日志
LA BuildStorm 最终日志
初赛回归日志
性能对比表
源码 diff
```

---

# 17. Codex 每轮工作的固定流程

Codex 每次只能完成一个最小任务。

## 17.1 开始前

```text
1. 读取本阶段目标。
2. git status。
3. 读取相关代码完整调用链。
4. 读取当前最早错误前后 200 行。
5. 写出根因假设，按可信度排序。
6. 选择最小可验证假设。
```

## 17.2 修改中

```text
1. 最小 diff。
2. 不修改无关文件。
3. 共享代码考虑双架构。
4. 所有错误路径返回明确 errno。
5. 不在持有 spinlock 时睡眠、分配大内存或执行块 I/O。
6. 不引入强引用环。
7. 不无限循环等待。
8. 日志有预算。
```

## 17.3 修改后

```bash
git diff --check
make all
运行对应最小测试
运行对应架构 gate
运行另一个架构构建
git diff --stat
```

## 17.4 报告格式

每次写入：

```text
artifacts/fullscore/iterations/NNN-<task>/REPORT.md
```

模板：

```markdown
# Iteration NNN

## Objective

## Baseline failure

## Root cause

## Changed files

## Design

## Why this is safe

## Commands run

## Results

## Evidence

## Regression checks

## Remaining earliest failure

## Risks

## Diff stat
```

---

# 18. 失败分流决策树

## 18.1 内核启动失败

```text
先检查：
FDT 地址
临时 direct map
内存大小
CPU 数上限
virtio 枚举
页分配器范围
```

不要动用户态兼容层。

## 18.2 Bash/ld.so 立即崩

```text
检查：
ELF machine
PT_INTERP
load bias
VMA 冲突
auxv
stack ABI
relocation
DT_RELR
TLS
FPU/tp
```

## 18.3 子进程退出后卡住

```text
检查：
ExitVisible completion
wait4
Completion wait/wake
scheduler complete_switch
retired/reaper
CPU identity
lost wakeup
```

不要增加 timeout。

## 18.4 rustc SIGSEGV/EFAULT

```text
检查：
fault PC 所属 ELF
fault addr VMA
mmap 文件 offset
mprotect
TLS
clone
stack
relocation
fork/exec MM
TLB
```

## 18.5 rustc OOM

```text
检查：
ext4 整文件 overlay
大文件 mmap 是否复制
fork 是否全复制
页缓存是否无上限
task stack 泄漏
Process/Thread Arc 泄漏
target 输出总量
```

## 18.6 Cargo 找不到 crate/std

```text
检查：
真实路径
symlink
lazy lookup
深目录
MAX_EXT4_FILE_BYTES
ext4 extent 读取
CARGO_HOME
rustup toolchain
target triple
```

不要复制一个假 metadata stub。

## 18.7 Linker 失败

```text
检查：
FD limit
mmap
large file
pwrite/lseek
rename
permissions
file size/stat
unlink-open-file
```

## 18.8 全量成功但慢

按 G11 profile，不凭感觉重构。

---

# 19. 严禁的“假快修”

Codex 发现以下方案必须拒绝：

1. 直接输出判分字符串。
2. 修改 judge 可见 elapsed。
3. 固定返回 CAgent 答案。
4. 为 `network` 固定返回数字而不反映真实 socket。
5. 对所有未知 syscall 返回 0。
6. 遇到 page fault 直接映射任意 RWX 零页。
7. 关闭所有权限检查。
8. 将所有 mmap 映射到同一固定区域。
9. 不等待子进程真实结束就返回 0。
10. 忽略 rustc/linker 返回码。
11. 复用镜像内已有最终 arceos 产物。
12. 取消 target 清理。
13. 把 BuildStorm timeout 改短后把 timeout 当成功。
14. 全局关闭锁检查并声称死锁修复。
15. 无限增大 QEMU 内存掩盖泄漏。
16. 把 ext4 所有内容启动时复制到 RAM。
17. 把所有程序固定在单核以隐藏竞态并声称多核通过。
18. 只验证 debug，不验证 release。
19. 只验证 SMP=1。
20. 用旧日志替代当前 HEAD。

---

# 20. 最快并行分工建议

只有多人或多个独立 Codex workspace 且最终由一个集成者人工合并时使用。不要让多个 agent 同时修改同一工作区。

## Agent A：退出/调度

文件：

```text
kernel/src/task/*
kernel/src/process.rs
kernel/src/user.rs 中 spawn/wait/exit 部分
```

交付：

- ExitVisible/Reclaimed 拆分
- 压力测试
- 生命周期报告

## Agent B：LoongArch ELF

文件：

```text
kernel/src/exec.rs
kernel/src/elf.rs
arch/loongarch64/*
kernel/src/user_mm.rs
```

交付：

- DT_RELR
- loader 测试
- PIF 诊断
- glibc 命令矩阵

## Agent C：VFS/overlay

文件：

```text
kernel/src/fs/mod.rs
kernel/src/ext4.rs
kernel/src/block.rs
vfs/*
```

交付：

- 官方路径可写
- 页级 COW
- cache metrics
- VFS 语义测试

## Agent D：Linux ABI

文件：

```text
kernel/src/user.rs
kernel/src/net/*
kernel/src/procfs 或相关模块
```

交付：

- futex/clone/mmap/statx/rename
- CAgent 严格 gate
- Cargo syscall 缺口报告

## 集成顺序

```text
A -> G2
B -> G7
D -> G4/G5
C correctness -> G6
C performance -> G11
```

任何 agent 不能 push。通过 patch 文件交给集成者，按 Gate 顺序应用。

---

# 21. 最终一次性 Codex 执行指令

当你希望 Codex 从头连续推进，可直接粘贴下面指令：

```text
严格执行仓库根目录的《SudoOS-Plus 决赛满分 Codex 极限执行计划》。

从 G0 开始。不要询问我是否继续；在当前可用环境中持续做到能完成的最高 Gate。
但必须遵守：
- 每次只修当前最早失败点。
- 不 push、不 merge、不 rebase、不 reset --hard、不整分支 cherry-pick。
- 不覆盖用户已有修改。
- 不伪造测试输出、时间、产物或返回码。
- 生产模式运行官方脚本原文。
- 不先写完整 ext4。
- 不进行无关重构。
- 每 1 个 Gate 生成完整 REPORT.md。
- 每次修改后 make all，并检查另一个架构。
- 有镜像时运行真实 QEMU；没有镜像时完成静态、单元和构建验证，并明确写“未运行真实镜像”，不能声称通过。
- 如果长测试失败，分析第一个错误前后 200 行，并写入失败分流。
- 如果当前环境无法完成某项验证，继续完成不依赖该环境的实现、测试和报告，不要停在空泛建议。

当前最高优先级：
1. G1 官方脚本一致性。
2. G2 ExitVisible/Reclaimed 分离。
3. G3 锁死 RV CAgent。
4. G4/G5 RV toolchain/minibuild。
5. G6 RV 全量编译。
6. G7 LoongArch DT_RELR/动态 glibc。
7. G8-G10 LoongArch 满分。
8. G11 性能。
9. G12 回归与文档。

最终返回：
- 完成到哪个 Gate。
- 真实通过项。
- 未验证项。
- 当前最早失败点。
- 所有修改文件。
- 所有执行命令。
- 日志路径。
- diff stat。
- 下一条唯一任务。
```

---

# 22. 立即执行的第一批任务

Codex 收到文档后，第一批只做以下事项：

## Batch 1

1. 完成 G0 基线报告。
2. 校验当前官方脚本差异。
3. 建立生产/诊断模式分离方案。
4. 画出完整退出调用链。
5. 证明 `complete_all()` 的当前触发位置和等待者。
6. 给出 G2 最小 patch 设计。
7. 不做性能优化。
8. 不动 vfs 分支。
9. 不 push。

## Batch 2

Batch 1 报告完成后立即：

1. 实施 G2。
2. 加生命周期压力测试。
3. RV SMP=1/2/8 验证。
4. CAgent 连续验证。
5. BuildStorm diagnostic 确认子进程退出后调用点返回。
6. 保存所有证据。

## Batch 3

1. 正式官方 BuildStorm toolchain。
2. 正式 minibuild。
3. 按首错修 futex/clone/mmap/VFS。
4. 不跑全量直到 minibuild 连续通过。

## Batch 4

1. RV 全量编译。
2. 取得第一次真实成功。
3. 再开始 LA DT_RELR 或并行 agent 已准备的 LA patch。
4. 双架构成功后做性能。

---

# 23. 最终满分验收表

只有全部打勾才允许声称“满分候选版本”。

## 构建

- [ ] `make all`
- [ ] `kernel-rv` 非空
- [ ] `kernel-la` 非空
- [ ] clean workspace 可复现构建
- [ ] 无网络依赖

## RISC-V CAgent

- [ ] 10/10
- [ ] 连续 20 轮
- [ ] 时间奖励全部达到
- [ ] 0 panic
- [ ] 0 hang
- [ ] 0 泄漏增长

## LoongArch CAgent

- [ ] 10/10
- [ ] 连续 20 轮
- [ ] 时间奖励全部达到
- [ ] SMP=8
- [ ] 平台核数配置
- [ ] 0 PIF

## RISC-V BuildStorm

- [ ] 官方脚本原文
- [ ] TOOLCHAIN ok
- [ ] MINIBUILD ok
- [ ] COMPILE ok=true
- [ ] bytes >= 500000
- [ ] 连续 3 轮
- [ ] elapsed 真实
- [ ] 性能达到目标

## LoongArch BuildStorm

- [ ] 官方脚本原文
- [ ] TOOLCHAIN ok
- [ ] MINIBUILD ok
- [ ] COMPILE ok=true
- [ ] bytes >= 500000
- [ ] 连续 3 轮
- [ ] elapsed 真实
- [ ] 性能达到目标

## 内核稳定性

- [ ] ExitVisible/Reclaimed 分离
- [ ] 10000 短进程
- [ ] 并发退出
- [ ] signal/timeout
- [ ] clone/futex
- [ ] retired backlog 清零
- [ ] task/process/fd/mm 无持续泄漏

## 文件系统

- [ ] `/root/.cargo` 可写
- [ ] `/work` 可写
- [ ] `/tmp` 可写
- [ ] rename
- [ ] unlink-open-file
- [ ] getdents64
- [ ] stat/statx
- [ ] file mmap
- [ ] 页级 COW 或证明整文件 COW 不影响成功和内存

## 动态加载

- [ ] RV glibc
- [ ] LA glibc
- [ ] PT_INTERP
- [ ] RELA/REL
- [ ] DT_RELR
- [ ] TLS
- [ ] auxv
- [ ] stack ABI
- [ ] VMA 无碰撞

## 回归

- [ ] RV glibc 初赛
- [ ] RV musl 初赛
- [ ] LA glibc 初赛
- [ ] LA musl 初赛
- [ ] 历史得分项无下降

## 文档

- [ ] 根因分析
- [ ] 设计实现
- [ ] 实验数据
- [ ] Linux 基线
- [ ] AI 使用
- [ ] 完整复现
- [ ] commit/image/toolchain SHA

---

# 24. 参考来源

- SudoOS-Plus `pym7.26`：
  - https://github.com/Sudo-666/SudoOS-Plus/tree/pym7.26
- SudoOS-Plus `vfs`：
  - https://github.com/Sudo-666/SudoOS-Plus/tree/vfs
- 官方 final-2026 tests：
  - https://github.com/oscomp/testsuits-for-oskernel/tree/final-2026
- 官方 BuildStorm 脚本：
  - https://raw.githubusercontent.com/oscomp/testsuits-for-oskernel/final-2026/scripts/buildstorm_testcode.sh
- 官方 CAgent 脚本：
  - https://raw.githubusercontent.com/oscomp/testsuits-for-oskernel/final-2026/scripts/cagent_testcode.sh

---

# 25. 最后原则

最快满分不是“同时改最多代码”，而是：

```text
最早失败点
→ 最小复现
→ 最小正确补丁
→ 当前 Gate 连续稳定
→ 双架构回归
→ 下一个 Gate
```

当前最应该立刻执行的顺序仍然是：

```text
官方脚本一致性
→ 退出/等待闭环
→ RV minibuild
→ RV 全量编译
→ LoongArch DT_RELR
→ LA 全量编译
→ 页级 COW/缓存/锁性能
→ 初赛回归与文档
```

任何偏离这条主线的完整 ext4、全仓重构、备份分支合并、评分输出修改或未经 profile 的性能工程，都会降低最快拿满分的概率。
