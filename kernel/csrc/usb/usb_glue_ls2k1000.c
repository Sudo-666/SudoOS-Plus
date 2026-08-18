/*
 * kernel/csrc/usb/usb_glue_ls2k1000.c
 *
 * SudoOS 第一个 C 源文件：LS2K1000 USB（CherryUSB）平台胶水宿主。
 *
 * M0 阶段只放一个构建路径探针，证明 loongarch64 C 对象能经
 * kernel/build.rs 交叉编译并链进纯 Rust 内核。M1 起在此填充
 * 真实的 EHCI 初始化、virt_to_dma 与延时实现（见
 * docs/decisions/ADR-001）。
 *
 * 编译约束（见 kernel/build.rs）：
 *   -mabi=lp64s        匹配 Rust 目标 loongarch64-unknown-none-softfloat
 *   -march=loongarch64 LA264 基础 ISA（2K1000 v1.0），不启用 SIMD
 *   -ffreestanding -nostdlib -fno-builtin -fno-stack-protector
 */

/* 探针哨兵：0x2A4A0001 = "LSU-B1"。真机串口应打印此值。 */
unsigned int sudoos_usb_glue_probe(void) {
    return 0x2A4A0001u;
}
