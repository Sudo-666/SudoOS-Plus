# SudoOS (MyOS)

一个使用 **Rust** 编写的业余操作系统内核，目标平台为 **RISC-V 64** 和 **LoongArch 64**。

启动期内存管理已成形，双架构均运行于高半内核且通过 MMU 硬件验证。LoongArch TLB refill 已接入，vmalloc/ioremap 均可硬件访问。SMP bring-up、抢占式内核调度、IPI 协作和 kernel-wide TLB shootdown 已在 RISC-V/LoongArch QEMU smoke 中验证。

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
                 ├─ trap vector 安装
                 ├─ IRQ dispatch 初始化
                 ├─ time/tick 计数初始化
                 └─ wfi / idle 0
```

## 单元测试

```bash
cargo test -p myos-mm    # 24 tests
```

## 构建 & 运行

```bash
make build ARCH=riscv64    # 构建
make run ARCH=riscv64      # 构建 + 运行
make run-riscv64           # 快捷命令
make run-loongarch64

make debug ARCH=riscv64    # GDB (端口 1234)
cargo test -p myos-mm      # 单元测试 (24 tests)
make check                 # source tree + fmt + host tests + 双架构 build/clippy
make smoke-all             # 双架构 QEMU 串口 smoke
make smoke-smp-all         # 双架构 SMP QEMU smoke
make stress-smp            # 可配置 SMP smoke 矩阵
make verify                # check + smoke-all + smoke-smp-all
make clean
```

常用 stress 示例：

```bash
make stress-smp STRESS_ARCHES="riscv64 loongarch64" STRESS_SMPS="1 2 4 8"
make stress-smp STRESS_ARCHES="riscv64 loongarch64" STRESS_PROFILES="debug release" STRESS_MEMS="64M 256M 1G"
```

stress 日志会写入 `build/stress-smp/`，每个 case 保存配置和串口输出，便于定位偶发失败。

## 工程文档

- [`docs/boot-order.md`](docs/boot-order.md)：当前启动顺序和初始化依赖。
- [`docs/context-rules.md`](docs/context-rules.md)：early/task/idle/hardirq/panic 上下文规则。
- [`docs/locking.md`](docs/locking.md)：当前锁、锁顺序草案和 M5 lockdep-lite 前置约束。
- [`docs/cpu-lifecycle.md`](docs/cpu-lifecycle.md)：CPU discovered/online/active/IPI-ready 生命周期。
- [`docs/scheduler-state-machine.md`](docs/scheduler-state-machine.md)：任务状态机、M4C/M4C2 verifier 证明边界。

## 当前进度

| 子系统 | 状态 | 说明 |
|--------|------|------|
| 构建系统 | ✅ | Cargo workspace（8 crates + 5 vendor），双架构 build/clippy/smoke 全绿 |
| RISC-V 两阶段启动 | ✅ | 低物理 entry.S → 静态 Sv39 → 高半内核 |
| LoongArch DMW 启动 | ✅ | DMW0/1 窗口 → cached high execution |
| FDT 设备枚举 | ✅ | model/compatible/cpu/memory/virtio-mmio |
| 物理内存映射 | ✅ | 6 步排除管线, MemoryMap 事务语义 |
| 虚拟内存布局 | ✅ | Sv39 + LA64 双策略 |
| 页表策略 | ✅ | W^X 强制, MappingOptions 校验 |
| 早期帧分配器 | ✅ | checkpoint/restore |
| 启动页表构造 | ✅ | BootPageTable + map_page + translate |
| 内核镜像映射 | ✅ | 高半 text/rodata/data |
| RAM direct-map | ✅ | Sv39 页表 / DMW 窗口 |
| MMU 启用 | ✅ | RISC-V SATP/Sv39 + LoongArch TLB refill 均已硬件验证 |
| 低地址映射撤销 | ✅ | RISC-V: removed / LoongArch: DMW2=0 |
| trap 分发 | ✅ | 双架构 TrapFrame、frame guard、连续异常 + 寄存器恢复自检、breakpoint 验证 |
| IRQ 子系统 | ✅ | 统一 InterruptSource dispatch，未处理 IRQ fail-fast |
| time/tick | ✅ | monotonic tick 计数 + 周期 timer armed + timer IRQ 验证 |
| 物理页分配器 | ✅ | buddy allocator, DMA32/Normal zone, page refcount, early handoff 校验 |
| 内核堆 | ✅ | global allocator (alloc crate), slab 小对象, buddy-backed large allocation |
| VMA/address space | ✅ | VmAreaSet、AddressSpace、brk metadata、mmap gap search |
| page fault policy | ✅ | anonymous/file/device/COW/protection/segv 分类模型 |
| page fault handler | ✅ | 双架构 fault trap 解码、统一 PageFault pipeline、kernel fail-fast; user demand paging 待 P4b |
| runtime page table | ✅ | 双架构 buddy-backed table pages、map/protect/unmap/translate，RISC-V 写入活动页表 |
| kernel vmalloc | ✅ | vmalloc/vfree/ioremap/iounmap API，guard page，双架构硬件生命周期验证 |
| TLB 模型 | ✅ | RISC-V sfence.vma + LoongArch TLB refill/invtlb；kernel-wide SMP shootdown + generation/ack |
| IRQ-safe 锁 | ✅ | IrqSpinLock 保存/恢复本地中断状态，嵌套锁自检 |
| SMP bring-up | ✅ | 双架构 secondary CPU 启动、online/ready mask、per-CPU trap/timer/stack、IPI delivery |
| 内核调度器 | ✅ | 抢占式 per-CPU FIFO round-robin、idle task、wait queue、work stealing、任务迁移、资源回收 |
| 系统调用 | ⬜ | syscall 表, U-mode |
| 设备驱动 | ⬜ | virtio-blk, virtio-net |
| 文件系统 | ⬜ | VFS, tmpfs/ext4（lwext4 适配层） |

## 下一步

1. **SMP/M5 收尾** — release smoke、LoongArch SMP=8、README/路线图同步、提交前 verify
2. **最小用户态入口** — UserThread/Process、用户页表、user text/stack、U-mode trap return
3. **最小 syscall 闭环** — ecall/syscall trap、sys_exit/sys_write 调试输出、用户任务退出与回收
4. **用户态缺页执行路径** — per-process AddressSpace、anonymous/heap/stack demand paging、copy_from_user/copy_to_user
5. **fork + COW** — AddressSpace 复制、write-protect、COW fault、refcount 回收
6. **设备与 VFS** — virtio-mmio transport、virtio-blk、tmpfs/devfs、page cache、file-backed mmap

## MM 完成路线图

本节用于跨 agent 交接。继续开发时请优先更新这里，不要只把进度留在聊天记录里。

### 当前 MM 边界

已完成：
- boot memory layout：FDT RAM + reserved-memory + kernel image + FDT 排除。
- early frame allocator：checkpoint/restore，启动期页表用。
- boot page table：RISC-V Sv39 / LoongArch LA64 map_page + translate。
- physical page allocator：buddy，DMA32/Normal zone，poisoning，自检。
- page refcount：物理页引用计数 inc/dec/get，refcount 归零后单页释放接口。
- kernel heap：slab 小对象 + buddy-backed large allocation，global allocator。
- MM object model：VMA、AddressSpace、brk metadata、mmap gap search。
- fault policy model：anonymous、file-backed、device、COW、protection、segv 分类。
- kernel vmalloc：`vmalloc/vfree/ioremap/iounmap` API，虚拟地址保留，guard page，runtime page table map/unmap。
- runtime page table：从 boot page table handoff，buddy-backed table pages，map/protect/unmap/translate。
- page fault handler：RISC-V instruction/load/store page fault 与 LoongArch page invalid/modified/protection 异常进入统一 `PageFault` pipeline。
- TLB model：RISC-V local `sfence.vma` 已执行；LoongArch TLB refill/invtlb 已接入，vmalloc/ioremap 硬件生命周期已验证。
- SMP kernel TLB shootdown：kernel-wide flush-all primitive，IPI reason 分发，generation/ack，远端 stale TLB 验证。

未完成：
- user demand paging：需要 per-process AddressSpace 后才能执行 anonymous/heap/stack fault map。
- user page table：每进程根页表、内核高半共享映射、地址空间切换。
- brk/mmap/munmap/mprotect syscall 后端：需要用户态和 syscall 层接入。
- COW 执行路径：fork 时 write-protect，fault 时复制页，更新引用计数。
- file-backed mmap：需要 VFS/page cache。
- per-address-space/range TLB shootdown：当前只有 kernel-wide flush-all；用户地址空间后再做 active CPU mask、range/ASID batching。

### P1: Runtime Page Table

目标：把启动期 `BootPageTable` 后面的页表 walker 抽象成可复用的运行期页表对象。

状态：核心完成。

任务：
- [x] 从 `KernelMemoryState` handoff boot page table 到 runtime page table。
- [x] 中间页表由 buddy 分配，失败时回滚未发布页表页。
- [x] 提供 `map_page` / `protect_page` / `unmap_page` / `translate`。
- [x] RISC-V 对当前活动根页表执行写入，并在修改后 `sfence.vma`。
- [x] LoongArch 保留 DMW 边界，完成软件页表 map/protect/unmap/translate 自检。
- [x] LoongArch 接入硬件页表寄存器、TLB refill 与 `invtlb`。
- [x] 空页表回收：unmap 后延迟回收空中间页表，释放前执行同步 TLB shootdown。

验收：
- [x] `cargo test -p myos-mm`
- [x] `make clippy`
- [x] `make build ARCH=riscv64`
- [x] `make build ARCH=loongarch64`
- [x] RISC-V/LoongArch QEMU 启动到 `kernel_main: initialization completed`
- [x] kernel vm 自检打印 `runtime map/protect/unmap verified`

### P2: Kernel Vmalloc / Ioremap

目标：让 `kernel/src/vm.rs` 从 reservation 变成真实内核虚拟映射管理器。

状态：核心完成。RISC-V 当前活动页表可用；LoongArch 硬件页表/TLB refill 已接入，vmalloc/ioremap 已通过硬件访问验证。

任务：
- [x] `vmalloc(size, align)`：reserve VA，分配物理页，写入 runtime kernel page table，返回可用虚拟范围。
- [x] `vfree()`：unmap，flush，本地释放物理页，释放 VA reservation。
- [x] `ioremap(phys, size)`：reserve VA，映射 device/uncached memory，不拥有物理页。
- [x] `iounmap()`：unmap device mapping，释放 VA reservation。
- [x] 保留前后 guard page。
- [x] 多步操作失败回滚：释放已分配物理页、已映射 PTE、VA reservation。
- [ ] 越界访问触发 page fault 后进入统一 fault handler。
- 优先把 RISC-V early UART identity mapping 逐步迁移到 ioremap/fixmap 策略。

验收：
- [x] kernel vm 自检打印 `vmalloc/vfree` verified。
- [x] kernel vm 自检打印 `ioremap/iounmap` verified。
- [x] kernel vm 自检打印 `protect/unmap` verified。
- [x] QEMU 两架构启动通过。
- [x] `make clippy` 无 warning。

### P3: Page Fault Handler

目标：缺页不再只是 panic，而是进入统一 fault pipeline。

状态：内核入口完成。用户态 demand paging 待 P4b per-process AddressSpace。

任务：
- [x] 在 RISC-V trap 中识别 instruction/load/store page fault。
- [x] 在 LoongArch trap 中识别 page invalid/page modified/page protection 类异常。
- [x] 构造 `PageFault { address, access, source, present }`。
- [x] kernel fault handler 记录 counters 并 fail-fast。
- [x] 启动日志打印 page fault subsystem。
- [x] debug 自检确认 fault counters 初始为 0。
- [ ] anonymous/heap/stack：分配 zero page 并 map。依赖 P4b user AddressSpace。
- [ ] COW：复制页并恢复 writable。依赖 P5 fork/COW。
- [ ] file-backed：接 page cache 后读取。依赖 P6 VFS/page cache。
- [ ] protection/segv：用户态 kill。依赖 task/signal 或进程退出机制。

依赖：
- P1 runtime page table。
- P2 vmalloc 可用于 kernel fault 验证。

验收：
- [x] `make clippy`
- [x] `cargo test -p myos-mm`
- [x] `make build ARCH=riscv64`
- [x] `make build ARCH=loongarch64`
- [x] RISC-V/LoongArch QEMU 启动到 `kernel_main: initialization completed`
- [x] 启动日志出现 `page fault subsystem` 与 `page fault test`

### P4a: Minimal User Mode

目标：先跑通一个用户任务进入 U-mode/PLV3、触发 syscall、返回内核并退出。

任务：
- 定义 `UserThread` / `Process` 最小结构，绑定用户入口、用户栈和地址空间。
- 为测试程序映射一页 user text 和一页 user stack。
- RISC-V 实现 `sret` 返回用户态；LoongArch 实现 `ertn` 返回 PLV3。
- trap 入口识别用户态 syscall，至少实现 debug write/exit。
- 用户任务退出后释放栈、页表和任务资源。

依赖：
- task/process 基础结构。
- arch trap return ABI。

### P4b: User Address Space + Demand Paging

目标：每个进程拥有独立用户页表，同时共享内核高半映射，并能处理用户缺页。

任务：
- `AddressSpace` 绑定 arch runtime page table root。
- clone kernel mappings 到用户页表或共享高半根项。
- activate/switch address space，预留 ASID/PCID 接口。
- 实现 brk/mmap/munmap/mprotect 的内核后端。
- 增加 copy_from_user/copy_to_user，必须处理跨页和 fault。
- anonymous/heap/stack fault 分配 zero page 并 map。

依赖：
- syscall ABI。
- P4a minimal user mode。

### P5: Copy-On-Write

目标：fork 后共享物理页，写时复制。

任务：
- [x] buddy metadata 增加 page refcount 操作接口：inc/dec/get。
- [x] 增加 `free_unreferenced_frame`，支持 refcount 归零后释放单页。
- [x] 全局 page allocator 包装 refcount API，启动自检打印 `refcount: verified`。
- fork AddressSpace 时复制 VMA，PTE 清 writable，设置 COW 标志。
- COW fault 分配新页、复制旧页、替换 PTE、flush。
- refcount 归零才释放物理页。

依赖：
- P4b user address space。
- scheduler/task fork 语义。

### P6: File-Backed Mmap

目标：支持文件页映射和页缓存。

任务：
- 设计 page cache key：inode + file offset。
- fault 时从 page cache 命中或发起读盘。
- dirty/writeback 策略先保守实现。
- 支持 private file mapping 与 shared file mapping 的不同语义。

依赖：
- VFS。
- block device driver。

### P7: Address-Space TLB Shootdown

目标：在现有 kernel-wide SMP shootdown 基础上，支持用户地址空间级别的精准 TLB 一致性。

任务：
- 每个 AddressSpace 记录 active CPU mask。
- 修改 PTE 后生成 `TlbShootdown`。
- 本地 flush 立即执行，远端通过 IPI 执行。
- 加 generation/ack，避免释放页早于远端 flush。
- 支持 range/ASID batching，避免所有变更都 full flush。

依赖：
- P4b user address space。
- 当前 SMP bring-up、IPI、kernel-wide shootdown。

### 推荐施工顺序

1. SMP/M5 封版：release smoke、LoongArch SMP=8、`make verify`、文档同步。
2. P4a minimal user mode：用户 text/stack、trap return、syscall exit/write。
3. P4b user address space + syscall memory API + user demand paging。
4. P5 COW。
5. P7 address-space/range TLB shootdown 优化。
6. VFS/page cache 后做 P6。

### 交接注意

- `build/` 已加入 `.gitignore`，之前清理缓存时有大量 tracked build artifacts staged for deletion；不要把这些当成源码删除事故。
- 当前 `myos-mm` 单测数是 24。
- `kernel vmalloc` 已有稳定 token API：`vmalloc/vfree/ioremap/iounmap`。释放函数消费 token，避免双重释放。
- RISC-V vmalloc 映射写入当前活动页表；LoongArch runtime page table/TLB refill 已接入，vmalloc VA 已可硬件解引用。
- page fault trap 入口已经接入；当前 kernel fault fail-fast，user demand paging 不能在 P4b 前假装完成。
- 现在的 `FaultOutcome` 是策略分类；anonymous/COW/file-backed 的执行路径分别依赖 P4b/P5/P6。
- LoongArch runtime page table 已接入硬件页表/TLB refill；vmalloc/ioremap 硬件访问已验证。
- SMP bring-up、抢占调度、wait queue、work stealing、任务迁移和 kernel-wide TLB shootdown 已完成；后续 P7 指的是用户地址空间级别的 range/ASID 优化。
- 不要为了快速完成把用户态、VFS、SMP 依赖伪造成 stub 成品；这些必须按依赖顺序接入。

## License

MIT OR Apache-2.0

## 作者

Mingyang Chen
