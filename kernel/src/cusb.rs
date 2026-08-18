//! C-USB 桥：LS2K1000 上 CherryUSB 宿主的 Rust 侧接口。
//!
//! C 实现位于 `kernel/csrc/usb/`，由 `kernel/build.rs` 交叉编译为
//! `libsudoos_usb.a` 链进内核（见 docs/decisions/ADR-001）。
//!
//! M0 阶段仅提供构建路径探针；M1 起在此暴露 `sudoos_usb_init` /
//! `sudoos_usb_capacity` / `sudoos_usb_read_blocks`，供块设备层
//! 包装成 `/dev/sda`。

/// M0 探针：打印 C 胶水返回的哨兵值，证明 loongarch64 C 对象确实
/// 链进内核且可经 FFI 调用。真机串口应看到 `USB-glue M0 probe=0x2a4a0001`。
pub fn probe_build_path() {
    // SAFETY: `sudoos_usb_glue_probe` 是 kernel/csrc/usb 交叉编译的 C
    // 函数，无参、返回普通整数，FFI 类型与 ABI（lp64s）匹配。
    let value = unsafe { sudoos_usb_glue_probe() };
    crate::println!("USB-glue M0 probe={value:#010x}");
}

unsafe extern "C" {
    /// `kernel/csrc/usb/usb_glue_ls2k1000.c` 提供的构建路径探针。
    fn sudoos_usb_glue_probe() -> u32;
}
