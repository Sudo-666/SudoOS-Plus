# M14 BusyBox initramfs artifact

This checkpoint makes the BusyBox artifact real instead of just a README-level goal.

## Scope

Implemented:

- rootless macOS-friendly `newc` initramfs builder
- static BusyBox sanity guard
- deterministic archive output
- `/init -> /bin/busybox`
- common BusyBox applet symlinks
- minimal `/etc`, `/dev`, `/proc`, `/sys`, `/tmp` layout
- artifact audit that can parse and validate the generated cpio

Not implemented here:

- dynamic BusyBox
- musl ldso handoff
- ext4 persistent rootfs
- mounting procfs/sysfs/devtmpfs semantics

## Usage

```bash
make busybox-initramfs BUSYBOX=/absolute/path/to/static/busybox
python3 scripts/m14-busybox-artifact-audit.py build/initramfs/busybox.cpio
```

The archive is suitable for the next smoke step once the QEMU runner has a stable `INITRAMFS=` or equivalent hook.
