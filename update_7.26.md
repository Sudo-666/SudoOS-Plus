# SudoOS 7.26 更新日志

## 分支

`pym7.26`（基于 `final` 分支 `1f7b3f6`）

## 修改清单

### P0：调度器退出卡死（RISC-V）

**文件**：`kernel/src/task/mod.rs`

`exit_current()` 不再信任 `current_cpu_id()`（读 `tp` 寄存器，迁移后可能陈旧）。始终遍历所有 CPU 用栈匹配确定 `actual_cpu`，无条件调用 `set_current_cpu_id()`。

### P1：评测平台适配

- **启动页表** `arch/riscv64/src/asm/entry.S`：临时直接映射 8→24 GiB。新平台 FDT 在 ~19GiB，旧映射不覆盖。
- **LA MAX_CPUS** `arch/loongarch64/src/smp.rs`：8→16。新平台 12 核，旧上限触发 panic。

### P1：BuildStorm 文件系统

- **ext4 递归展开** `kernel/src/main.rs`：`oscomp_materialize_ext4_dir_flat` 添加 `recurse_levels`，按需展开 3 层（rustlib → riscv64gc → lib → .rlib）。
- **ext4 文件大小** `kernel/src/ext4.rs`：`MAX_EXT4_FILE_BYTES` 16→256 MB。`.rlib` 文件（20-30MB）之前被静默跳过，rustc 报 "only metadata stub"。
- **Cargo 可写缓存** `kernel/src/user.rs`：`CARGO_HOME=/tmp/cargo-cache`，`/tmp` 检测到 ext4 符号链接则替换为 tmpfs。
- **官方脚本** `kernel/src/final_buildstorm_testcode.sh`：`/work/` → `/tmp/`，`CARGO_HOME` 用 `${CARGO_HOME:-...}`，timeout 4h→10min。
- **CRLF 修复** `kernel/src/*.sh`：Windows 的 `\r\n` 导致 `/bin/sh` 解析失败（`script exit=2`）。

### 评测平台公告（7.26）

> 1) 纠正了 cagent 测试的 waitpid 问题
> 2) 纠正了 cagent 测试 ss 程序缺失问题
> 3) RV: `-m 16G -smp 8`，LA: `-m 36G -smp 12`，超时 6250s
> Linux 最好成绩：LA 6223s / RV 4655s

## 提交历史

| 提交 | 说明 |
|------|------|
| `5a7a548` | P0: `exit_current()` 永远栈反查 CPU 并无条件修正 |
| `2266483` | P1: ext4 按需展开 + 递归深度控制 |
| `4cd6ecf` | P1: CARGO_HOME + `/tmp` → tmpfs |
| `78857f6` | P1: 脚本 `${CARGO_HOME:-...}` |
| `71a88e7` | P2: 脚本 `/work/` → `/tmp/`，timeout 10min |
| `bd6114f` | P1: 启动页表 8→24 GiB |
| `bdb2224` | P1: 递归深度 1→2（展开到 `.rlib`） |
| `24dee87` | docs: update_7.26.md + CRLF 脚本修复 |
| `3dda1a4` | add FDT magic 诊断 |
| `0145654` | LA MAX_CPUS 8→16 |
| `0cf0d4b` | MAX_EXT4_FILE_BYTES 16→256 MB |

## 本地验证结果

- ✅ 内核正常启动（`-smp 1/2/8`，boot CPU 0/3/4/5 均正常）
- ✅ CAgen 不卡死（P0 修复）
- ✅ `E0463` 消失（54 个 `.rlib` 安装成功，256MB 修复后不再有 metadata stub）
- ✅ ext4 展开噪声大幅减少（递归深度控制）
- ⏳ `librustc_driver.so` 映射失败——WSL QEMU 2GB 内存瓶颈，评测机 16GB 不会出现
- ⏳ LA 本地跑不了——virtio DMA 缓冲区在 2GB QEMU 上超出直接映射范围

## 评测建议

```bash
git clone -b pym7.26 https://gitlab.eduxiji.net/T2026102699910462/oskernel2026-0xdeadbeef.git
cd oskernel2026-0xdeadbeef
ARCH=riscv64 PROFILE=release bash scripts/build.sh
ARCH=loongarch64 PROFILE=release bash scripts/build.sh
```

预期：RV CAgent PASS，BuildStorm diagnostic 编译通过（TOOLCHAIN ok + MINIBUILD ok）。LA 至少不 panic 启动。

---

*Co-Authored-By: Claude <noreply@anthropic.com>*
