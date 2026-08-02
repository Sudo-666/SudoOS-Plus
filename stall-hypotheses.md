# BuildStorm 编译停滞问题 —— 猜想清单

> 目标：编译在 8 核 25 分钟只推进 7 个 crate（第 8 个 shlex 后停滞）；平台两次停在 smallvec。
> 本文列出基于所有诊断证据的可能根因，按可信度排序，供下一步定向排查。

## 已确认的事实（诊断证据）

1. **停滞结构**（watchdog dump，120s 间隔 3 次相同）：
   - `cargo(22) Blocked wait=104` —— 等子进程
   - `rustc(106) Runnable queued=Cpu0` —— **排队 120s+ 从未运行**
   - `cpu0 need_resched=false runnable=1` —— **队列有任务但调度器认为无需切换**
   - `cpu1-7: Idle 空转` —— 7 个 CPU 全空，不偷任务
2. **480s 后"恢复"**：switches 暴涨（513→2423），所有 CPU 活跃，但编译**仍停在 7 个 crate**（任务在切换但不干活）
3. **用户态 PC**（monitor 采样）：
   - `0x0d7081c2` = pid=27 的 **ecall 循环**（反复 openat/fstat/mmap/close/read，pc 在 0x2001xxxx）
   - `0x01121308` = pid=65 写 "Compiling" 进度（write fd=2）
   - pid=13 的 0x01121308 = **"buffer overflow " 字符串**（glibc fortify 消息）
4. **内核 PC**：sys_read 区域反复出现；`ram_virtual_address`（3 CPU）；`IrqSpinLock::lock`（1 CPU）
5. **sys_read 返回**：pid=13/14 反复 read fd=3 返回 832 字节（管道流正常——"cargo 不读管道"假设**不成立**）
6. **本地速度异常**：2 核 15 分钟只编译 2 个 crate ≈ 慢 450 倍（TCG 正常 10-30 倍）——**异常慢，不是纯 TCG**

---

## 猜想 A：调度器 Runnable 任务不被运行（最硬证据）★

**证据**：`runnable=1 + need_resched=false` 持续 120s+；Idle 不捡任务；7 CPU 空转。

**可能子因**：
- A1. **wake_waiters 的 need_resched 被清零后任务留队列**：唤醒时设置 need_resched=true → 切换发生时清零 → 但切换选的不是该任务（dequeue_next 顺序问题）→ 任务留在队列，need_resched=false → 直到下一次 wake/tick。**tick 对 Idle 直接 return**（`if current.kind.is_idle() { return }`），所以 Idle 时 tick 不设置 need_resched → **空转 CPU 永远不知道队列有任务**。
- A2. **Idle 循环的 work_available 检查竞态**：Idle 在关中断下检查队列为空 → WFI → 期间任务被唤醒入队（同 CPU）→ wake 路径只设 need_resched 不发 IPI（同 CPU 不需要）→ WFI 等待中断 → tick 10ms 后到 → Idle tick 直接 return → **任务永远不被捡起**。**这完全符合观察！**
- A3. 迁移缺失：排队后不迁移，CPU0 忙则任务饿死（文档称"迁移"但只在唤醒时选核）。

**验证方法**：dump 加"任务入队时间戳"；或 Idle 的 WFI 后无条件重查队列。

**修复方向**：Idle 的 tick 不 return（改为检查队列）；或 WFI 唤醒后必查队列；或 wake 同 CPU 时发本地 resched。

---

## 猜想 B：管道写阻塞唤醒丢失 ★★

**证据**：pid=27 反复 syscall（openat/mmap/read）；rustc 写管道满阻塞 → cargo 读 → wake_writers。

**可能子因**：
- B1. pipe 的 `read_epoch`/`write_epoch` 竞态：read 和 write 并发时，epoch 检查在锁外 → 唤醒丢失（wake 在等待者入队前执行，epoch 已变但等待者用旧 epoch 入队 → 永远等）。
- B2. `wake_writers` 在 read 后调用，但**阻塞的写者可能已切换**（wake_after_switch 竞态）。

**验证**：dump 里看 rustc 主线程的 waitaddr 是否 pipe write_wait；加 pipe 唤醒计数。

---

## 猜想 C：glibc fortify "buffer overflow" 触发 ★★

**证据**：pid=13 映射 "buffer overflow " 字符串；多个 CPU 在消息相关代码。

**可能子因**：
- C1. **某 syscall 返回错误长度**导致 glibc `__*_chk` 检测到溢出：候选 readv/writev 长度计算、ioctl 结构大小、fcntl、readlink、poll fd_set 越界写。
- C2. fortify 失败后 abort → **SIGABRT 信号处理卡住**（默认动作未终止进程）→ 进程卡在 abort → 其他进程等它 → 死锁。
- C3. fortify 消息写 stderr 管道阻塞（没人读 stderr）→ 卡在消息输出。

**验证**：SIGABRT 发送时打印（已加过但没跑完）；或 syscall 层对比 glibc 预期长度。

---

## 猜想 D：串口/console 同步写慢 ★★★

**证据**：文档自述"串口输出在 QEMU 中是同步慢路径"；pid=65 反复写 "Compiling" 进度；cargo 输出量大。

**可能子因**：
- D1. UART busy-wait 每字节等待 FIFO → TCG 下每字节极慢 → 输出成为编译瓶颈。
- D2. 评测平台用串口管道 → QEMU 后端满 → guest write 阻塞 → 死锁（Linux 有中断驱动 UART，我们没有）。

**验证**：对比同输出量的 Linux 行为；测试关闭进度输出是否推进。

**修复方向**：console 输出加缓冲/批量；或 UART 中断驱动。

---

## 猜想 E：多核并发 virtio-blk 读挂起 ★★

**证据**：早期 dbg2 monitor 采样 CPU0 稳定在 `read_blocks`；本次采样 `ram_virtual_address` 3 CPU。

**可能子因**：
- E1. virtio 队列多核并发（两个 CPU 同时操作 queue）→ 状态错乱 → 设备不完成请求。
- E2. DMA 缓冲生命周期问题（请求缓冲被提前释放/覆盖）。

**验证**：挂起时读 virtio used ring 索引（monitor xp）确认设备是否完成。

---

## 猜想 F：fork/vfork 竞态 ★★★

**证据**：cargo 反复 fork rustc；vfork 有 completion 机制。

**可能子因**：
- F1. CLONE_VFORK completion 丢失（exec 失败路径不唤醒父进程）→ 父进程永远等。
- F2. fork 的 eager copy 在多核并发下慢（页表复制）。

**验证**：dump 里 cargo 的 wait=104 是否为 vfork completion。

---

## 猜想 G：TCG 本地环境异常 ★★★

**证据**：2 核 15 分钟 2 crate ≈ 慢 450 倍（远超 TCG 正常）。

**可能子因**：
- G1. 本地 WSL + 8GB 配置的 TCG 效率低于平台。
- G2. 本地测试干扰（monitor/串口文件写）。

**影响**：本地复现的"停滞"可能是慢的放大；平台行为可能不同（平台 51 秒 8 crate 是本地速度的 ~10 倍）。

---

## 排查优先级建议

1. **猜想 A（调度器 Idle tick）**——证据最硬（runnable=1 + need_resched=false + Idle 空转），修复成本低（Idle tick 检查队列），且 480s"恢复"可解释（某次 tick 恰好触发）。
2. **猜想 C（fortify）**——SIGABRT 诊断已加，跑一轮即可确认。
3. **猜想 B/E（管道/ virtio）**——dump 加 waitaddr 类型区分。
4. **猜想 D（串口）**——平台对比实验。

## 已验证排除的

- ❌ epoll 桩（已修复，cargo 确认调用 epoll_pwait）
- ❌ futex 50ms 忙等（已修复）
- ❌ fcntl 锁（已修复，3850 消失）
- ❌ VMA 栈溢出（已修复）
- ❌ "cargo 不读管道"（read 返回 832 字节正常）
- ❌ 单一死锁点（480s 后 CPU 恢复活跃，但编译仍不推进——**任务在跑但不干活**，指向用户态忙等/输出瓶颈）
