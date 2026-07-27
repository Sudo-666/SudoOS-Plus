# SudoOS 7.27 更新日志

## 分支

`pym7.26`（基于 `final` 分支）

## 评测平台适配

- 启动页表 8→24 GiB（`arch/riscv64/src/asm/entry.S`）：新平台 FDT ~19GiB，旧映射不覆盖
- LA MAX_CPUS 8→16（`arch/loongarch64/src/smp.rs`）：新平台 12 核

## P0：调度器退出卡死

`kernel/src/task/mod.rs`：`exit_current()` 不信任 `current_cpu_id()`（RISC-V tp 寄存器迁移后可能陈旧），始终用栈反查确定 CPU。

## ext4 文件系统

- `MAX_EXT4_FILE_BYTES` 16→256 MB：`.rlib` 文件 >16MB 不再被静默跳过
- `MAX_EXT4_NODES` 8192→65536：深层工具链目录快照不超限
- 两个版本待评测机验证：
  - `59ec753`：`recurse_levels=2` + `mkdir`（接近 online 输出版本）
  - `5a1d5ac`：目录全量快照（已验证本地编译通过）

## BuildStorm 脚本适配

- `CARGO_HOME=/tmp/cargo-cache`（tmpfs 可写）
- `/work/` 写路径→`/tmp/`
- timeout 4h→10min
- CRLF→LF 修复

## 已知问题

- LA：动态链接器 PIF 崩溃（R_LARCH_64 + DT_RELR 修复已写好但未合并）
- 本地 2GB WSL QEMU 无法完整跑 BuildStorm，需评测机验证

---

*Co-Authored-By: Claude <noreply@anthropic.com>*
