# OSKernel2026 初赛提交检查表

评测机只执行 `make all`。该目标必须只负责构建，并在仓库根目录生成：

- `kernel-rv`
- `kernel-la`

## 必跑命令

```bash
make oscomp-vendor      # Cargo.lock 有外部依赖时必须提交 vendor/cargo
make all                # 生成 kernel-rv 和 kernel-la
make oscomp-audit       # 检查 0 分高危项
file kernel-rv kernel-la
```

## 0 分高危项

- `rust-toolchain` 使用浮动 `nightly`，导致评测机下载最新 nightly。
- 依赖隐藏目录 `.cargo`；评测 clone 会过滤隐藏目录，必须提交 `cargo-dot` 并在构建时恢复 `.cargo`。
- `make all` 运行 QEMU、smoke、stress、soak，导致评测时间过长。
- 根目录没有 `kernel-rv` 或 `kernel-la`。
- 内核没有扫描评测盘根目录的 `*_testcode.sh`，或没有输出 `#### OS COMP TEST GROUP START ... ####` 这类 marker。
- 测试跑完没有主动 shutdown/poweroff，评测机等到超时。

## 分支策略

`newtry` 是最新开发版；若不稳定，建议从 `backup-final-test` 切出提交分支：

```bash
git checkout backup-final-test
git checkout -b submit-oscomp
python3 ~/Downloads/install_oscomp_newtry_patch.py
```
