# M14 dual-arch vendor BusyBox artifact

M14 的 BusyBox 不是内核代码的一部分，而是用户态验收样本。仓库约定路径：

```text
vendor/userland/riscv64/busybox-static
vendor/userland/loongarch64/busybox-static
```

默认 `make m14-vendor-userland-audit` 要求 riscv64 存在且是 ELF64 static；loongarch64 缺失时先 WARN，避免在 LoongArch rootfs/virtio 路径未完全收敛前卡住内核主线。

最终双架构封版时使用：

```bash
make m14-vendor-userland-audit-strict
make busybox-initramfs-vendor-all
```

`busybox-initramfs-vendor-all` 会为已存在的架构产物生成：

```text
build/initramfs/busybox-riscv64.cpio
build/initramfs/busybox-loongarch64.cpio
```
