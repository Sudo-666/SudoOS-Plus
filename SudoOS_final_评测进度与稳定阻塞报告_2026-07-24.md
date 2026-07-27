# SudoOS-Plus `final` 分支评测进度与稳定阻塞报告

> **用途**：团队同步、任务拆分、后续调试依据  
> **报告日期**：2026-07-24  
> **代码基线**：GitHub `Sudo-666/SudoOS-Plus` 的 `final` 分支  
> **GitHub 当前提交**：`62facdd134f76d2bc8943636e69553fa1080c8f8`（`final first push`）  
> **评测依据**：连续多次刷新评测机器后得到的 `86 / 87 / 88` 三轮日志  
> **重要说明**：本文严格区分“源码事实”“日志事实”和“分析推断”。

---

## 1. 执行摘要

当前 `final` 版本已经取得三个明确进展：

1. **RISC-V CAgent 已真实完整通过**：
   - 官方 CAgent 脚本运行完成；
   - 十项工作负载实际执行；
   - 主 Bash 进程完成 `close-files / robust / ctid` 清理；
   - 输出 `PASS`、`completed=1`、`pass=1`；
   - 不再发生最初的 `current user Thread` panic。‘
   

2. **RISC-V BuildStorm 已进入真实 Rust 编译链**：
   - lazy ext4 overlay 初始化成功；
   - `/bin/sh`、Cargo、rustc、动态加载器均成功启动；
   - `librustc_driver` 成功映射；
   - `cargo new` 成功；
   - `cargo build` 确实启动了 `minibuild-diag` 编译。

3. **前两轮退出清理 panic 已被逐步消除**：
   - 第 86 轮停在 `clear_child_tid_on_exit`；
   - 第 87 轮推进到 robust-list 清理，但触发 lockdep panic；
   - 第 88 轮 `close-files / robust / ctid` 全部完成，不再 panic。

但是，第 88 轮暴露出一个新的、稳定复现的主阻塞：

> **BuildStorm 诊断 shell（PID 121）已经退出并完成全部清理，调度器打印 CPU 身份修正信息后，内核没有返回到 `verify_final_buildstorm_thread()` 的调用点。**

因此：

- 没有输出 `sudoos-diag: final-buildstorm: diagnostic exit=...`；
- 没有启动正式 `/tmp/buildstorm_testcode.official.sh`；
- 没有出现任何正式 BuildStorm 评分标志；
- 评测机持续停在同一个位置。

这不是评测机器偶发波动。多次刷新后停止位置一致，应该按**确定性的内核同步/调度问题**处理。

---

## 2. GitHub `final` 分支当前状态

### 2.1 分支与提交

GitHub 当前公开信息：

| 项目 | 当前状态 |
|---|---|
| 分支 | `final` |
| 提交数 | 163 |
| 最新提交 | `62facdd` |
| 提交说明 | `final first push` |
| 提交日期 | 2026-07-23 |
| 改动规模 | 48 files changed |
| 行数变化 | `+5,397 / -1,419` |

本次提交集中加入或修改了：

- Final CAgent / BuildStorm runner；
- RISC-V 8 GiB 启动与 direct-map 适配；
- 用户地址空间扩展；
- ELF / mmap / VFS / ext4 overlay；
- 网络与 socket 兼容；
- 进程、线程、futex、退出清理；
- 多核调度与任务回收；
- QEMU 日志等待工具；
- 决赛测试脚本。

### 2.2 当前 `final` 已包含的近期修正

当前 GitHub 源码中已经能看到以下实现，不应重复当作“尚未合入”：

#### A. `clear_child_tid` 使用退出线程自身 MM

退出清理不再依赖已经可能失效的隐式 `current_user_thread()`，而是使用：

```rust
let mm = thread.process().mm();
mm.copy_to_user(...);
get_futex_queue_for_mm(mm, ...);
```

#### B. robust-list 清理使用退出线程自身 MM

`cleanup_robust_list_on_exit()` 已改为显式使用：

```rust
let mm = thread.process().mm();
```

所有 robust-list 用户内存访问和 futex wake 均绑定该 MM。

#### C. lazy ext4 目录无条件递归

当前代码已经不是“目录存在就跳过递归”。对于真实 ext4 目录，即使对应 VFS 目录已经存在，仍然执行：

```rust
sdcard_install_ext4_dir_files(&sub_ext4);
```

因此，之前“`std` 缺失仅仅因为已存在目录跳过递归”的判断，**对当前 `final` 版本已经不成立**。目录递归补丁已经存在，但仍不足以让 rustc 找到 `std`。

#### D. 退出时 CPU 身份自动修正

当前调度器 `exit_current()` 会：

1. 读取架构层报告的 CPU；
2. 根据当前内核栈实际属于哪个 scheduler current task，寻找真实 CPU；
3. 两者不一致时调用 `set_current_cpu_id(actual_cpu)`；
4. 输出：

```text
scheduler: repaired stale CPU identity on exit
```

这说明源码作者已经知道“架构层 CPU 身份可能陈旧”，但当前修复只在退出临界点做补救，尚未证明整个切换与唤醒链闭环正确。

---

## 3. 三轮评测的推进过程

### 3.1 第 86 轮：CAgent 完成后在 `clear_child_tid` panic

日志末尾：

```text
process-cleanup: tid=13 close-files done
process-cleanup: tid=13 robust done

================ KERNEL PANIC ================
M9-B user-memory operation has no current user Thread
```

结论：

- CAgent 工作负载本身已执行；
- 主进程退出时，`clear_child_tid_on_exit()` 仍走隐式 current-thread 路径；
- runner 无法完成总结；
- 整组成绩作废。

### 3.2 第 87 轮：推进到 BuildStorm，robust-list 触发 lockdep panic

本轮已经出现：

```text
BUILDSTORM_DIAG_RUN_RC=127
process-exit: pid=121 tid=121 group=true status=0
process-cleanup: tid=121 close-files done

================ KERNEL PANIC ================
unlock with empty held-lock stack
```

结论：

- `clear_child_tid` 修复成功；
- RISC-V CAgent 已经闭环；
- BuildStorm 诊断编译已经执行；
- 新阻塞发生在 PID 121 的 robust-list 清理阶段。

### 3.3 第 88 轮：所有进程清理完成，但同步调用不返回

最新日志末尾：

```text
BUILDSTORM_DIAG_RUN_RC=127
process-exit: pid=121 tid=121 group=true status=0
process-cleanup: tid=121 close-files begin
process-cleanup: tid=121 close-files done
process-cleanup: tid=121 robust done
process-cleanup: tid=121 ctid done
scheduler: repaired stale CPU identity on exit
           reported=0 actual=2
```

之后没有任何输出。

结论：

- robust-list 修复生效；
- PID 121 的用户态逻辑和内核退出清理均结束；
- panic 已消失；
- 但等待该用户任务完成的上层同步调用没有被唤醒或没有恢复运行；
- 正式 BuildStorm 脚本尚未启动；
- 多次刷新评测机均停在同一点，属于确定性阻塞。

---

## 4. RISC-V CAgent 进度

### 4.1 已确认完成

最新日志明确出现：

```text
/tmp/cagent_testcode.official.sh : PASS

#### OS COMP SUMMARY ####
arch=riscv64
total=1
completed=1
pass=1
fail=0
skipped=0
timeout=0
signal11=0
signal14=0
score=1
```

同时，主进程 PID 13 完整执行：

```text
close-files done
robust done
ctid done
```

这说明以下链路均已真实成立：

- 动态 glibc Bash；
- 本地 LLM server；
- loopback TCP；
- 多个并发 agent；
- clone / wait / pipe / exec；
- 文件创建、读写、目录、搜索和容量检查；
- date / uname / nproc / grep / find / rm 等工具；
- 全部 agent 回收；
- server 关闭；
- 官方 CAgent 脚本退出；
- runner summary 输出。

### 4.2 当前判定

| 项目 | 状态 |
|---|---|
| CAgent 实际工作负载 | 已完成 |
| CAgent 官方脚本 | PASS |
| 退出清理 | 已完成 |
| summary | 已完成 |
| RISC-V CAgent 工程完成度 | 约 100% |
| 是否还应把 CAgent 作为当前主攻方向 | 否 |

---

## 5. RISC-V BuildStorm 实际执行到了哪里

### 5.1 已完成的阶段

BuildStorm 已经不是“尚未启动”，而是进入了真实编译：

```text
entering final-2026 BuildStorm runner
final-buildstorm: lazy ext4 overlay ready
final-buildstorm: diagnostic minibuild begin
```

随后真实执行：

- `/usr/bin/rm`
- `/root/.cargo/bin/cargo`
- `cargo new`
- `cargo build`
- rustc
- `librustc_driver-*.so`
- libc / libm / libpthread / libdl / libgcc_s / libatomic
- socketpair、pipe、clone 等编译器运行依赖

日志出现：

```text
Compiling minibuild-diag v0.1.0 (/tmp/minibuild-diag)
```

因此，Rust 工具链入口和动态装载主链是有效的。

### 5.2 诊断编译内部失败

诊断编译中同时出现两类文件系统症状。

#### 症状 A：Cargo 缓存元数据写入失败

```text
warning: failed to auto-clean cache data
warning: failed to save last-use data

disk I/O error

Caused by:
  Error code 3850: disk I/O error
```

这表明 overlay 的某些写入、重命名、同步、锁或 SQLite 文件操作并不符合 Cargo 预期。

#### 症状 B：rustc 找不到目标 `std`

```text
error[E0463]: can't find crate for `std`

the `riscv64gc-unknown-linux-gnu` target may not be installed
```

与此同时系统输出：

```text
.../lib/rustlib :
0 newly installed, 16 already available
```

这只能证明 `rustlib` 顶层存在 16 个 VFS 项，不能证明：

- `riscv64gc-unknown-linux-gnu/lib` 能被正确遍历；
- `libstd-*.rlib` 确实存在；
- rustc 能 stat/open/read 这些文件；
- symlink、inode type、目录项类型和路径解析全部正确；
- 文件内容及长度有效。

### 5.3 诊断 shell 最终行为

虽然 `cargo build` 返回 101，诊断脚本仍继续执行：

```text
BUILDSTORM_DIAG_BUILD_RC=101
```

随后尝试运行不存在的产物：

```text
/tmp/minibuild-diag/target/debug/minibuild: not found
BUILDSTORM_DIAG_RUN_RC=127
```

由于诊断命令最后执行的是：

```sh
echo BUILDSTORM_DIAG_RUN_RC=$?
```

shell 自身最终状态为 0。因此 PID 121 正常退出是合理的。

---

## 6. 真正的稳定卡点

### 6.1 源码预期控制流

当前 `verify_final_buildstorm_thread()` 的逻辑是：

```text
运行诊断 shell
    ↓
run_rootfs_program_with_cwd() 返回
    ↓
打印 final-buildstorm: diagnostic exit=<code>
    ↓
启动 /tmp/buildstorm_testcode.official.sh
    ↓
打印正式脚本退出状态
```

### 6.2 实际控制流

日志表现为：

```text
诊断 shell 完成
    ↓
PID 121 close-files / robust / ctid 全部完成
    ↓
exit_current() 发现 CPU 身份错误
    ↓
reported=0 actual=2，执行修正
    ↓
无后续输出
```

缺失的第一行应该是：

```text
sudoos-diag: final-buildstorm: diagnostic exit=0
```

这行没有出现，证明：

> **`run_rootfs_program_with_cwd()` 没有返回给 `verify_final_buildstorm_thread()`。**

因此当前第一主阻塞不应描述成“rustc 找不到 std”，而应描述为：

> **用户任务退出后的 task retirement / join completion / waiter wake / scheduler resume 链路没有完成，导致同步运行者永久等待。**

### 6.3 为什么 CPU 身份修正值得高度怀疑

最后一行明确显示：

```text
reported=0 actual=2
```

说明正在 CPU 2 上执行退出逻辑的任务，架构层 `current_cpu_id()` 却仍报告 CPU 0。

源码的补救逻辑只在 `exit_current()` 中：

- 根据栈反查真实 CPU；
- 临时覆盖 current CPU ID；
- 再执行 scheduler exit。

风险包括：

1. 该任务此前的调度、loaded MM、wait queue 或 per-CPU 计数可能已经按错误 CPU 记账；
2. join waiter 可能被挂在另一 CPU 的队列；
3. `finish_switch()` 使用修正后的 CPU，但此前 pending/current 状态可能已经分裂；
4. task reaper 虽被 wake，但可能没有在正确 CPU 上得到调度；
5. `Task::destroy_resources()` 没执行，导致 `user_join.complete_all()` 没发生；
6. 等待 `run_rootfs_program_with_cwd()` 的内核线程因此不被唤醒。

这里第 5 点尤其关键：当前源码只有在 retired task 被 reaper 真正销毁时才执行：

```rust
join.complete_all();
```

PID 121 打印完 `ctid done` 只表示用户清理完成，并不代表 scheduler task 已被回收，也不代表 join 已完成。

### 6.4 当前最合理的根因范围

**高置信度范围：**

- task exit；
- context switch tail；
- retired task 入队；
- task reaper 唤醒；
- `destroy_resources()`；
- `join.complete_all()`；
- 阻塞内核线程恢复。

**中等置信度关联：**

- stale CPU identity；
- 任务迁移后的 per-CPU 身份恢复；
- RISC-V hart-local CPU ID 保存方式；
- user TP / kernel TP / context switch 之间的身份污染。

**当前不能直接下结论：**

- 不能仅凭最后一行断言是 `set_current_cpu_id()` 本身；
- 不能断言只是 ext4；
- 不能断言只是 `std` 文件缺失；
- 不能断言评测机性能慢；
- 不能把刷新评测机当作解决方法。

---

## 7. 为什么多次刷新评测机仍然卡住

至少两轮修复后的日志都沿同一 BuildStorm 路径前进：

- Cargo 启动；
- rustc 启动；
- `std` 缺失；
- 诊断产物不存在；
- `BUILDSTORM_DIAG_RUN_RC=127`；
- 主诊断 shell PID 121 退出。

第 88 轮进一步稳定到：

- `close-files done`
- `robust done`
- `ctid done`
- stale CPU identity 修正
- 停止

这说明：

1. 失败由确定性的代码路径触发；
2. 与具体评测机器负载、缓存冷热或随机调度无明显关系；
3. 刷新机器只会重放同一个问题；
4. 下一步应该增加 exit/reaper/join 的精确日志，而不是继续刷新。

---

## 8. LoongArch 当前状态

本轮提供的新日志主要是 RISC-V；LoongArch 暂无新的通过证据。

已知最近状态仍是：

```text
/tmp/cagent_testcode.official.sh : FAIL (signal=14)
```

BuildStorm 同样在动态 glibc 程序早期返回：

```text
final-buildstorm: diagnostic exit=-14
final-buildstorm: script exit=-14
```

已确认：

- LA 内核启动、SMP、VirtIO block/net、基础用户态 gate 均可运行；
- 失败发生在动态 glibc loader / 动态程序初始阶段；
- 当前 `signal=14` 可能只是 runner 将 `-EFAULT` 当成信号编号展示；
- `final` 已包含有界 LoongArch 用户异常日志，但当前已有评测输出中尚未看到足够的新现场数据。

### LA 当前判定

| 项目 | 状态 |
|---|---|
| 内核启动与基础 gate | 已完成 |
| 动态 glibc CAgent | 未通过 |
| CAgent 实际用例 | 尚未进入 |
| BuildStorm diagnostic | 动态程序早期失败 |
| 正式 BuildStorm | 未进入 |
| 当前优先级 | 低于 RV 同步卡死，但仍是独立主线 |

---

## 9. 当前进度矩阵

| 模块 | 当前阶段 | 已验证 | 未完成 |
|---|---|---|---|
| RV CAgent | 完整闭环 | 官方 PASS、summary、退出清理 | 无当前阻塞 |
| RV BuildStorm 环境 | 已进入 | overlay、shell、Cargo、rustc、动态库 | 环境语义仍不完整 |
| RV diagnostic minibuild | 编译已开始 | `Compiling minibuild-diag` | Cargo I/O、目标 std |
| RV diagnostic 退出 | 用户清理完成 | PID 121 全部 cleanup | join/reaper/runner 恢复 |
| RV 正式 BuildStorm | 未开始 | 脚本已安装 | runner 未恢复，无法启动 |
| RV 完整 tgoskits 编译 | 未到达 | 无 | 正确性和性能均未测试 |
| LA CAgent | 动态启动失败 | loader 被装载 | 首个真实用例未运行 |
| LA BuildStorm | 动态启动失败 | runner 入口 | diagnostic 主体未运行 |

---

## 10. 工程完成度与评分边界

### 10.1 工程完成度估计

该百分比表示路径完成度，不等于平台分数：

| 方向 | 工程完成度估计 |
|---|---:|
| RV CAgent | 100% |
| RV BuildStorm runner/environment | 55%–65% |
| RV BuildStorm diagnostic minibuild | 35%–45% |
| RV BuildStorm 正式编译 | 0%–10% |
| RV BuildStorm 性能优化 | 0% |
| LA CAgent | 20%–30% |
| LA BuildStorm | 10%–20% |
| 总体决赛路径 | 约 35%–45% |

### 10.2 平台得分边界

目前日志只能证明：

- RV CAgent 在 guest 内完整 PASS；
- BuildStorm 尚未出现正式评分标志；
- LA 两组尚未通过。

不能因为 diagnostic 中 Cargo/rustc 已启动，就把 BuildStorm 工程进度换算成平台得分。

---

## 11. 下一步建议：只做定位，不先大改

### P0：确认 retired task 是否真正入队和销毁

建议增加有界日志：

```text
task-exit-prepare:
  task_id
  reported_cpu
  actual_cpu
  next_task

task-switch-tail:
  cpu
  previous
  next
  disposition
  retired_backlog
  retired_outstanding

task-reaper:
  wake
  take task_id
  destroy begin
  join complete
  destroy done
```

验收目标：

```text
PID 121 exit
→ retired task 入队
→ reaper 取出
→ destroy_resources
→ join.complete_all
→ blocked kernel runner 被唤醒
→ diagnostic exit=0
```

### P1：检查 CPU 身份为什么长期陈旧

必须确认 RISC-V 的 CPU ID 来源：

- 是 tp；
- 是 sscratch；
- 是 per-hart 内存；
- 还是上下文结构字段。

需要追踪：

```text
context switch:
  hardware hart id
  current_cpu_id before
  scheduler cpu
  outgoing task
  incoming task
  kernel sp
```

重点不是继续在 `exit_current()` 兜底，而是找出为什么任务迁移到 CPU 2 后仍报告 CPU 0。

### P2：runner 能恢复后，再处理 BuildStorm 文件系统

恢复 `diagnostic exit=0` 和正式脚本启动之后，再分别定位：

1. Cargo cache `disk I/O error`；
2. `libstd-*.rlib` 的目录项、stat、open、read；
3. rustc sysroot 解析；
4. overlay 中 rename/fsync/flock/SQLite 行为。

建议只对以下路径做有界 trace：

```text
/root/.cargo/
/root/.rustup/
/tmp/minibuild-diag/
/work/tgoskits/
```

### P3：最后再进入正式编译与性能

顺序必须是：

```text
同步退出闭环
→ diagnostic shell 返回
→ minibuild 编译成功
→ 官方 BuildStorm 环境项
→ tgoskits 完整编译
→ 8 核性能优化
```

---

## 12. 给队友的简短同步版本

可直接复制：

> 当前 GitHub `final` 基线为 `62facdd`。RV CAgent 已真实完整 PASS，之前的 `clear_child_tid` 和 robust-list 退出 panic 都已修掉。BuildStorm 已能启动 Cargo/rustc，并真实开始编译 `minibuild-diag`，但 diagnostic 内存在 Cargo cache `disk I/O error` 和 rustc `E0463 can't find crate for std`。更关键的是，多次刷新评测机都稳定卡在同一位置：诊断 shell PID 121 已输出 `BUILDSTORM_DIAG_RUN_RC=127`，并完成 `close-files / robust / ctid`，随后调度器报告 `reported CPU 0 / actual CPU 2` 并修正 CPU 身份，但上层 `run_rootfs_program_with_cwd()` 没返回，所以没有 `diagnostic exit=0`，正式 BuildStorm 脚本也没有启动。当前第一优先级不是继续补 std，而是定位 task exit → retired queue → reaper → `join.complete_all()` → runner wake 的同步链，以及任务迁移后 CPU ID 陈旧的根因。LA 仍停在动态 glibc 程序早期 `-14`，本轮无新突破。

---

## 13. 事实、推断与待验证项

### 已证实事实

- RV CAgent PASS；
- BuildStorm Cargo/rustc 已启动；
- diagnostic 编译失败；
- diagnostic shell PID 121 正常退出；
- PID 121 三项清理均完成；
- 最后一行是 stale CPU identity 修正；
- 没有 `diagnostic exit=...`；
- 没有正式 BuildStorm 脚本输出；
- 多次评测刷新停止位置一致。

### 强推断

- `run_rootfs_program_with_cwd()` 正在等待 user task join；
- join 依赖 retired task 被 reaper 销毁；
- reaper/join/wakeup/runner resume 中至少一环没有完成；
- stale CPU identity 与该问题高度相关。

### 仍待验证

- retired task 是否已加入队列；
- `finish_switch()` 是否在正确 CPU 上完成；
- `TASK_REAPER_QUEUE.wake_one()` 是否触发；
- reaper 是否被调度；
- `Task::destroy_resources()` 是否执行；
- `join.complete_all()` 是否执行；
- waiter 是否被加入正确 CPU run queue；
- IPI 是否发送到正确 CPU；
- CPU ID 陈旧的最初发生点。

---

## 14. 证据来源

### GitHub

- Branch: `https://github.com/Sudo-666/SudoOS-Plus/tree/final`
- Commit history: `https://github.com/Sudo-666/SudoOS-Plus/commits/final`
- Current commit: `https://github.com/Sudo-666/SudoOS-Plus/commit/62facdd134f76d2bc8943636e69553fa1080c8f8`
- User/runtime code: `kernel/src/user.rs`
- Scheduler/task code: `kernel/src/task/mod.rs`
- Final runner: `kernel/src/oscomp/final_2026.rs`
- Final QEMU targets: `Makefile`

### 评测日志

- `粘贴的文本 (1)(86).txt`
- `粘贴的文本 (1)(87).txt`
- `粘贴的文本 (1)(88).txt`
- `Riscv输出.txt`
- `LoongArch输出.txt`

---

## 15. 本报告结论

当前版本不是停在“Cargo 没启动”或“rustc 没启动”，也不能简单归纳为“缺少 std”。

最准确的状态是：

> **RV CAgent 已完成；RV BuildStorm diagnostic 已运行并暴露文件系统问题，但在 diagnostic shell 正常退出后，内核的 task retirement / join completion / runner resume 链路发生稳定卡死。最后可见异常信号是任务运行于 CPU 2、架构层却报告 CPU 0。**

下一步应先让：

```text
PID 121 exit
→ join complete
→ diagnostic exit=0
→ 正式 BuildStorm 脚本启动
```

形成闭环，再继续修 Cargo I/O 和 Rust sysroot。
