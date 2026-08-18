# kernel/csrc — 内核 C 源

本目录是纯 Rust 内核里唯一的 C 源码树：LS2K1000 USB（CherryUSB）平台
胶水与 OSAL 映射。

## 构建路径

`kernel/build.rs` 在 **loongarch64 + platform-ls2k1000** 下交叉编译本目录
`usb/*.c` **以及** `vendor/cherryusb/` 的裁剪子集（`core/usbh_core.c`、
`osal/usb_workq.c`、`class/hub/usbh_hub.c`、`class/msc/usbh_msc.c`、
`port/ehci/usb_ehci.c`）为 `libsudoos_usb.a` 静态库并链接进内核；其余
目标不产生任何 C 依赖。工具链默认取 PATH 中的 `loongarch64-linux-gnu-gcc`
/ `loongarch64-linux-gnu-ar`，可用环境变量 `LS2K1000_CC` / `LS2K1000_AR`
覆盖。

C 侧 ABI 与 Rust 目标 `loongarch64-unknown-none-softfloat` 保持一致：
`-mabi=lp64s -march=loongarch64`，全部 freestanding，不依赖 libc。

`usb/usb_config.h` 通过 include 路径优先于 `vendor/cherryusb` 根模板生效
（`kernel/csrc/usb` 是首个 `-I` 目录）。

## 里程碑

- M0：C 构建路径 + 探针 `sudoos_usb_glue_probe`。
- M1：CherryUSB 子集编译链接 + OSAL（内存/临界区/时钟真实，
  线程/信号量为桩）+ EHCI 平台胶水（uncached DMW 基址 + LoongArch
  dcache 维护）。
- M2+：线程/信号量接 SudoOS 调度器，EHCI 枚举识别 `0951:1666`。

设计决策见 `docs/decisions/ADR-001`。vendored CherryUSB 固定快照：
`vendor/cherryusb/`（Apache-2.0，见其 LICENSE；HEAD 3db0d15f）。
