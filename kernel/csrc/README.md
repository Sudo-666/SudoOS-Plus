# kernel/csrc — 内核 C 源

本目录是纯 Rust 内核里唯一的 C 源码树，目前只有 LS2K1000 USB
（CherryUSB）平台胶水。

## 构建路径

`kernel/build.rs` 在 **loongarch64 + platform-ls2k1000** 下交叉编译
`usb/*.c` 为 `libsudoos_usb.a` 静态库并链接进内核；其余目标不产生
任何 C 依赖。工具链默认取 PATH 中的 `loongarch64-linux-gnu-gcc` /
`loongarch64-linux-gnu-ar`，可用环境变量 `LS2K1000_CC` / `LS2K1000_AR`
覆盖。

C 侧 ABI 与 Rust 目标 `loongarch64-unknown-none-softfloat` 保持一致：
`-mabi=lp64s -march=loongarch64`，全部 freestanding，不依赖 libc。

设计决策见 `docs/decisions/ADR-001`。
