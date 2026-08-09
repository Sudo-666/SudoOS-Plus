# LS2K1000 稳定基线测试协议(ls2k-core-v0.1)

分支 `ls2k-stabilize`,基线构建链产物:

| 产物 | 说明 |
|---|---|
| `kernel-ls2k1000.elf` | ELF(调试/符号) |
| `kernel-ls2k1000.bin` | raw payload |
| `kernel-ls2k1000.uImage` | 上板烧写文件(vendor mkimage,IH_ARCH_LA=27) |
| `kernel-ls2k1000.buildinfo` | 构建溯源(git commit / dirty / hashes) |

## 烧写与启动(bootm)

```text
sf probe
fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
fatload usb 0:1 0x900000000a000000 ls2k1000-minimal.dtb
iminfo  0x9000000002000000
bootm   0x9000000002000000 - 0x900000000a000000
```

## 启动判读表(每轮必看)

```text
TRAP-VECTOR expected=0x...1000 installed=0x...1000 vs=0 PASS   ← 异常向量页对齐(永久检查)
HEAP-STATE[...] initialized=true stats=...                      ← 类型安全堆状态
CPU-CNTR cpu=0 timer=N ipi_recv=M ipi_send=K                    ← per-CPU 中断计数
CPU-CNTR cpu=1 timer=0 ipi_recv=M ipi_send=0                    ← 副核 idle 无 timer=预期;M>0 证明中断路径
CPU-COUNTERS PASS                                               ← boot 核 timer + 副核 IPI 往返均正常
BOOT11 all-ap-online
BOOT14 user-entry
SMOKE_TEST: PASS                                                ← 冒烟通过
```

**判读要点**:
- `cpu=1 timer=0` 是**预期行为**而非故障:副核进入 tickless NO_HZ idle 后,
  `time::enter_idle` 在本地无软件定时器时会把硬件定时器 shutdown(TCFG=0),
  因此 idle 副核收不到 timer IRQ。这正是设计如此;副核一旦跑普通任务即恢复 tick。
- 副核中断路径由启动检查发送的**真实 reschedule IPI 往返**证明:
  `cpu=1 ipi_recv=M>0` 说明 唤醒 idle → trap entry → ECODE_INTERRUPT → IPI 分发
  → acknowledge 整条硬件路径可用(timer IRQ 走同一条 trap 路径)。

任何 `KERNEL PANIC` / `OOM-HANDLER` / `HEAP_FATAL` / 锁超时即失败。

## 测试矩阵

| # | 测试 | 操作 | 验收 |
|---|---|---|---|
| 1 | 连续冷启动 | U-Boot `reset` 后重新 bootm,连续 20 次 | 20/20 到 `BOOT14 user-entry` + `SMOKE_TEST: PASS` |
| 2 | 双核 60 分钟 | 启动后停在 shell,持续运行 ≥60 min,全程观察 | 无 panic/OOM/锁超时;`CPU-COUNTERS PASS`;跑负载时两个 CPU 都有 timer 增长 |
| 3 | SMP 调度 | 观察 `CPU-CNTR`(boot 核 timer>0,副核跑负载后 timer>0);跑双线程负载 | 两个 CPU 都执行过普通任务 |
| 4 | IPI 双向 | `CPU-COUNTERS` 的 ipi_recv/ipi_send | boot 核 ipi_send>0 且副核 ipi_recv>0;运行中计数持续增长 |
| 5 | 进程压力 | `sudoos.oscomp=lifecycle-stress` 或用户态循环 clone/execve/wait4(需 initramfs 含 `/bin/sh`) | 1000 次全部成功,无 OOM |
| 6 | VM 压力 | 用户态循环 mmap/munmap/mprotect | 无页错误崩溃 |
| 7 | IPC 压力 | 用户态并发 pipe + signal | 无死锁/丢失 |
| 8 | 内存回收 | 压力测试后对比 `HEAP-STATE` / 空闲页计数 | 空闲页回到稳定基线 |
| 9 | 错误检查 | 全程日志 | 无 panic、OOM、页错误、锁超时 |

### 说明

- **tickless NO_HZ 与副核 timer 计数的关系**(run-15 真机 panic 根因):副核
  `idle_until_interrupt` 走 `time::enter_idle`,本地无软件定时器时
  `reprogram_local(None)` 调用 `shutdown()` 停掉 TCFG,故 idle 副核
  `timer` 恒为 0。启动检查因此改为:**boot 核验证 timer IRQ、副核验证真实
  IPI 往返**——二者覆盖同一条 trap 路径。详见 `kernel/src/main.rs` 的
  CPU-COUNTERS 检查注释。
- 测试 1/2/3/4/8/9 依赖当前基线即可完成(无需存储/initramfs)。
- 测试 5/6/7 需要真实用户环境(initramfs/BusyBox 或竞赛磁盘)——见阶段四;
  在到达阶段四前,可用内核自带 `lifecycle-stress`(`sudoos.oscomp=lifecycle-stress`
  bootargs)先行覆盖进程 fork/exec/wait 压力,前提是 initramfs 提供 `/bin/sh` 与 `/bin/true`。
- 冷启动计数的判读:串口日志每轮包含 `SMOKE_TEST: PASS` 即为该轮通过。

## 通过后打标签

```bash
git tag -a ls2k-core-v0.1 -m "LS2K1000 core platform stable"
git push origin ls2k-stabilize --tags
```

标签语义:CPU、内存、异常、中断、SMP、调度器、用户态核心适配完成,且通过
20 次冷启动 + 60 分钟双核稳定性测试。
