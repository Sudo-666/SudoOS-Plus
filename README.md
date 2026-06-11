# SudoOS (MyOS)

一个使用 **Rust** 编写的业余操作系统内核，目标平台为 **RISC-V 64** 和 **LoongArch 64**。

启动期内存管理已完成，两个架构均已启用 MMU 并运行于高半内核地址空间。

## 运行效果

**RISC-V 64**

```
make run ARCH=riscv64
```

```
MyOS
  architecture : riscv64    boot cpu: 0    device tree: 0x8fe00000

fdt:             model: riscv-virtio,qemu   compatible: riscv-virtio
  cpu: 1    memory: 256 MiB    virtio-mmio: 8 devices

physical memory:         253 MiB usable
virtual memory:          Sv39, kernel @ 0xffffffff80000000

kernel image mapping:
  text             phys 0x80400000 → virt 0xffffffff80000000
  rodata           phys 0x80415000 → virt 0xffffffff80015000
  data/bss/stack   phys 0x8041f000 → virt 0xffffffff8001f000

RISC-V final address space:
  current PC      : 0xffffffff800035b0
  high text       : 0xffffffff80000000
  direct map      : verified
  low boot mapping: removed
```

**LoongArch 64**

```
make run ARCH=loongarch64
```

```
MyOS
  architecture : loongarch64    device tree: 0x100000    system table: 0x200

LoongArch DMW:
  DMW0 (uncached): 0x8000000000000001   DMW1 (cached): 0x9000000000000011
  high execution: verified

fdt:             compatible: linux,dummy-loongson3   cpu: 1   memory: 256 MiB

physical memory:         251 MiB usable
virtual memory:          LA64 DMW + 4-level paging, kernel @ 0x9000000000400000

kernel image mapping:
  text             phys 0x400000 → virt 0x9000000000400000
  rodata           phys 0x41b000 → virt 0x900000000041b000
  data/bss/stack   phys 0x425000 → virt 0x9000000000425000
```

## ELF 布局验证

RISC-V ELF program headers（`llvm-readelf -lW`）：

```
Entry point: 0x80200000

Type  VirtAddr            PhysAddr            Flg
LOAD  0x80200000          0x80200000          R E   (.boot.text)
LOAD  0x80201000          0x80201000          R     (.boot.rodata)
LOAD  0x80202000          0x80202000          RW    (.boot.bss, NOBITS)
LOAD  0xffffffff80000000  0x80400000          R E   (.text)
LOAD  0xffffffff80015000  0x80415000          R     (.rodata)
LOAD  0xffffffff8001f000  0x8041f000          RW    (.bss + .boot_stack)
```

关键不变量：
- boot 段 `VirtAddr == PhysAddr`，全部位于 `0x80200000` 附近
- kernel 段 `VirtAddr` 位于 `0xffffffff80000000`，`PhysAddr` 位于 `0x80400000`
- ELF 无运行时重定位

## 目标架构

| 架构 | 页表 | 内核链接 | 内核物理 | Boot 物理 | MMU | 状态 |
|------|------|---------|---------|----------|-----|------|
| RISC-V 64 | Sv39 (3级) | `0xffffffff80000000` | `0x80400000` | `0x80200000` | SATP | ✅ |
| LoongArch 64 | LA64 (4级) + DMW | `0x9000000000400000` | `0x400000` | `0x200000` | DMW | ✅ |

## Crate 架构

```
kernel ← boot, fdt, mm, runtime, arch-riscv64, arch-loongarch64
```

| Crate | 路径 | 职责 |
|-------|------|------|
| `myos-kernel` | `kernel/` | 入口 → 设备发现 → 内存初始化 → 页表构造 → MMU 切换 |
| `myos-boot` | `boot/` | `BootInfo` builder, `BootAddress` |
| `myos-fdt` | `firmware/fdt/` | FDT 验证 + parser 封装 + 设备枚举 |
| `myos-mm` | `mm/` | `PhysAddr`, `MemoryMap`, `PagePermissions`, `MappingOptions`, `EarlyFrameAllocator` |
| `myos-runtime` | `runtime/` | `ByteConsole` trait + `ConsoleWriter<C>` |
| `arch-riscv64` | `arch/riscv64/` | 两阶段启动汇编、Sv39 页表、direct-map、UART |
| `arch-loongarch64` | `arch/loongarch64/` | DMW 窗口启动、EFI system table、LA64 页表 |

## 启动流程

```
RISC-V:                               LoongArch:
  OpenSBI → _start (0x80200000)          QEMU → _start (0x200000)
    ├─ 临时 Sv39 页表构造                  ├─ DMW 窗口配置
    ├─ satp 写入                           ├─ CRMD.PG=1
    ├─ jr KERNEL_VIRT_BASE                ├─ jirl → cached DMW
    └─ __riscv_high_entry                  └─ rust_entry (高地址)
      ├─ BSS 清零 / gp / sp
      └─ rust_entry (高地址)
           └─ kernel_main
                 ├─ FDT 解析
                 ├─ 物理内存映射 (6 步排除)
                 ├─ 虚拟布局 + 页表策略
                 ├─ EarlyFrameAllocator + BootPageTable
                 ├─ 内核镜像映射 (高半)
                 ├─ direct-map 构造
                 ├─ switch_sv39_root / DMW 验证
                 └─ wfi / idle 0
```

## 单元测试

```bash
cargo test -p myos-mm    # 14 tests
```

## 构建 & 运行

```bash
make build ARCH=riscv64    # 构建
make run ARCH=riscv64      # 构建 + 运行
make run-riscv64           # 快捷命令
make run-loongarch64

make debug ARCH=riscv64    # GDB (端口 1234)
cargo test -p myos-mm      # 单元测试
make fmt && make clippy && make clean
```

## 当前进度

| 子系统 | 状态 | 说明 |
|--------|------|------|
| 构建系统 | ✅ | Cargo workspace (7 crates + 2 vendor) |
| RISC-V 两阶段启动 | ✅ | 低物理 entry.S → 临时 Sv39 → 高半内核 |
| LoongArch DMW 启动 | ✅ | DMW0/1 窗口 → cached high execution |
| FDT 设备枚举 | ✅ | model/compatible/cpu/memory/virtio-mmio |
| 物理内存映射 | ✅ | 6 步排除管线, MemoryMap 事务语义 |
| 虚拟内存布局 | ✅ | Sv39 + LA64 双策略 |
| 页表策略 | ✅ | W^X 强制, MappingOptions 校验 |
| 早期帧分配器 | ✅ | checkpoint/restore |
| 启动页表构造 | ✅ | BootPageTable + map_page + translate |
| 内核镜像映射 | ✅ | 高半 text/rodata/data |
| RAM direct-map | ✅ | Sv39 页表 / DMW 窗口 |
| MMU 启用 | ✅ | SATP + DMW, 硬件 walk 验证 |
| 低地址映射撤销 | ✅ | RISC-V: removed / LoongArch: DMW2=0 |
| 中断/异常 | ⬜ | trap handler, PLIC / EIOINTC |
| 物理页分配器 | ⬜ | buddy allocator |
| 进程/调度 | ⬜ | task, fork, exec, scheduler |
| 系统调用 | ⬜ | syscall 表, U-mode |
| 设备驱动 | ⬜ | virtio-blk, virtio-net |
| 文件系统 | ⬜ | VFS, ext2, tmpfs |

## 下一步

1. **中断处理** — `stvec` / `eentry`，trap handler，时钟中断
2. **物理页分配器** — buddy allocator
3. **进程管理** — task_struct, 上下文切换, 调度器
4. **系统调用** — syscall 表, U-mode 切换

## License

MIT OR Apache-2.0

## 作者

Mingyang Chen
