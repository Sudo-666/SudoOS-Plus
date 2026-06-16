# M11：VFS 虚拟文件系统

## 完成边界

M11 实现 Linux 2.6 风格的 VFS 抽象层和两个内存文件系统：

- 5 个核心 trait：`FileOperations`、`InodeOperations`、`SuperBlockOperations`、`FileSystemType`、`DentryOperations`
- 4 个核心结构：`File`/`ArcFile`、`Inode`/`ArcInode`、`Dentry`/`DentryRef`、`SuperBlock`/`ArcSuperBlock`
- 原子引用计数实现（`AtomicU32`），支持 `Send + Sync`
- `FileTable<const MAX_FDS>` per-process 文件描述符表
- **tmpfs**：完整的内存文件系统（文件、目录、符号链接、原子 rename）
- **devfs**：设备文件系统（`/dev/null`、`/dev/zero`、`/dev/console`）
- Linux 兼容的类型和常量：`Stat`、`DirEntry`、`Errno`、`FileMode`、`OpenFlags`、`SeekWhence`

### 暂不包含

- VFS mount table 和路径解析（M9 后接入）
- dentry cache（当前为直接引用计数）
- page cache / `address_space`
- `poll`/`select`/`epoll` 完整实现
- ext4（通过 lwext4，M15）
- 网络文件系统
- 文件锁（`flock`）

## Crate 架构

```
myos-vfs (vfs/)
├── lib.rs              # #![no_std] + extern crate alloc, 扁平 pub use
├── errno.rs            # 34 个 Linux errno
├── file_type.rs        # FileType 枚举
├── file_mode.rs        # FileMode (S_IFMT/S_IRWXU)
├── open_flags.rs       # OpenFlags + AccessMode
├── seek.rs             # SeekWhence
├── inode_id.rs         # InodeId(u64)
├── dev_t.rs            # Dev { major, minor }
├── fcntl.rs            # FcntlCmd
├── rename.rs           # RenameFlags
├── dirent.rs           # DirEntry (Linux dirent64)
├── stat.rs             # Stat + StatFs + 魔数
├── buffer.rs           # IoBuffer / MutableIoBuffer
├── file_ops.rs         # FileOperations trait (9 methods)
├── inode_ops.rs        # InodeOperations trait (13 methods)
├── super_ops.rs        # SuperBlockOperations trait (5 methods)
├── dentry_ops.rs       # DentryOperations trait (3 methods)
├── fs_type.rs          # FileSystemType trait + MS_* flags
├── super_block.rs      # SuperBlock + ArcSuperBlock
├── inode.rs            # Inode + ArcInode
├── dentry.rs           # Dentry + DentryRef
├── file.rs             # File + ArcFile
├── file_table.rs       # FileTable<const MAX_FDS>
└── pathname.rs         # PathComponents 路径解析

kernel/src/fs/
├── mod.rs              # make_test_dentry() helper
├── tmpfs.rs            # tmpfs 内存文件系统
└── devfs.rs            # devfs 设备文件系统
```

## trait 设计

### FileOperations

```rust
pub trait FileOperations: Send + 'static {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno>;
    fn write(&mut self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno>;
    fn seek(&mut self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno>;
    fn release(&mut self, file: &File);
    fn ioctl(&mut self, file: &File, cmd: u64, arg: usize) -> Result<usize, Errno>;
    fn fstat(&self, file: &File) -> Result<Stat, Errno>;
    fn mmap(&self, file: &File, offset: u64) -> Result<u64, Errno>;
    fn fsync(&mut self, file: &File) -> Result<(), Errno>;
    fn readdir(&self, file: &File, entries: &mut ReadDirEntries<'_>) -> Result<usize, Errno>;
    fn poll(&self, file: &File) -> Result<PollStatus, Errno>;
}
```

### InodeOperations

```rust
pub trait InodeOperations: Send + 'static {
    fn create(&self, dir: &Inode, name: &str, mode: FileMode) -> Result<InodeId, Errno>;
    fn lookup(&self, dir: &Inode, name: &str) -> Result<InodeId, Errno>;
    fn link(&self, old: &Inode, dir: &Inode, name: &str) -> Result<(), Errno>;
    fn unlink(&self, dir: &Inode, name: &str) -> Result<(), Errno>;
    fn mkdir(&self, dir: &Inode, name: &str, mode: FileMode) -> Result<InodeId, Errno>;
    fn rmdir(&self, dir: &Inode, name: &str) -> Result<(), Errno>;
    fn rename(...) -> Result<(), Errno>;
    fn symlink(&self, dir: &Inode, name: &str, target: &str) -> Result<InodeId, Errno>;
    fn readlink(&self, inode: &Inode, buffer: &mut [u8]) -> Result<usize, Errno>;
    fn mknod(&self, dir: &Inode, name: &str, mode: FileMode, dev: Dev) -> Result<InodeId, Errno>;
    fn getattr(&self, inode: &Inode) -> Result<Stat, Errno>;
    fn setattr(&self, inode: &Inode, stat: &Stat) -> Result<(), Errno>;
    fn open(&self, inode: &Inode) -> Result<Box<dyn FileOperations>, Errno>;
}
```

### SuperBlockOperations

```rust
pub trait SuperBlockOperations: Send + 'static {
    fn alloc_inode(&self, sb: &SuperBlock, mode: FileMode) -> Result<ArcInode, Errno>;
    fn destroy_inode(&self, inode: &Inode);
    fn write_inode(&self, inode: &Inode) -> Result<(), Errno>;
    fn statfs(&self, sb: &SuperBlock) -> Result<StatFs, Errno>;
    fn put_super(&self, sb: &SuperBlock);
}
```

## 关键设计决策

| 决策 | 理由 |
|------|------|
| `#![no_std]` + `extern crate alloc` | 与 mm crate 一致，独立于内核 |
| 内部无锁 | 同步由 `IrqSpinLock` 在 kernel crate 层负责 |
| `const generics` 定容 | `FileTable<128>` 等与 `VmAreaSet<N>` 一致 |
| `AtomicU32` 引用计数 | `ArcFile`/`ArcInode`/`DentryRef`/`ArcSuperBlock` |
| `*const T` 内部指针 | 与 Rust 标准库 `Arc` 实现方式一致 |
| `i_private` + `s_fs_info` 裸指针 | 匹配 Linux 内核模式，文件系统扩展数据 |
| trait 默认方法返回 `ENOSYS`/`ENOTTY` | 按需覆盖，减少样板代码 |
| `ReadDirEntries` push-based API | 避免分配，直接写入用户态 buffer |

## tmpfs 实现

tmpfs 是纯内存文件系统，使用 `BTreeMap` 存储目录项，`Vec<u8>` 存储文件内容：

```
TmpfsSbData (SuperBlock.s_fs_info)
├── inodes: BTreeMap<InodeId, ArcInode>
└── next_ino: AtomicU64

TmpfsInodeOps → 目录操作 (create/lookup/mkdir/unlink/rename/symlink)
TmpfsRegularFile → Vec<u8> 读写/seek
TmpfsDirFile → BTreeMap 遍历 → readdir
```

## devfs 实现

devfs 是只读设备文件系统，预定义 3 个设备节点：

| 设备 | major | minor | 行为 |
|------|-------|-------|------|
| `/dev/null` | 1 | 3 | read→EOF, write→丢弃 |
| `/dev/zero` | 1 | 5 | read→填充零, write→丢弃 |
| `/dev/console` | 5 | 1 | read→EOF(暂), write→UART |

设备通过 `Device` trait 抽象，`DeviceFile` 适配器桥接 `Device → FileOperations`。

## 测试覆盖

### 单元测试 (cargo test -p myos-vfs)

| 模块 | 测试数 | 覆盖内容 |
|------|-------|---------|
| buffer | 5 | IoBuffer/MutableIoBuffer fill/push/remaining |
| dentry | 3 | 名称、引用计数、negative dentry |
| dev_t | 4 | 编码、预设常量值 |
| dirent | 2 | 构造、write_to 序列化 |
| errno | 2 | to_isize/from_isize 往返 |
| fcntl | 7 | 全部 6 种 cmd + unknown |
| file | 6 | 读写、seek、引用计数、EOF、稀疏写 |
| file_mode | 3 | S_IFMT 分类、权限位 |
| file_table | 15 | 分配/关闭/dup/cloexec/容量耗尽/重用 |
| file_type | 1 | from_mode_bits |
| inode | 3 | 字段、nlink、引用计数 |
| open_flags | 2 | access_mode、标志组合 |
| pathname | 11 | 绝对/相对/根/迭代/.. /长组件/空 |
| rename | 3 | NOREPLACE/EXCHANGE/NONE |
| seek | 2 | 有效值/无效值 |
| stat | 5 | 零值、blocks计算、StatFs |
| super_block | 4 | 引用计数、root inode、s_fs_info |
| **总计** | **78** | |

### 集成测试 (QEMU smoke)

| 测试 | 状态 |
|------|------|
| tmpfs create/lookup | ✅ |
| tmpfs read/write | ✅ |
| tmpfs mkdir/rmdir | ✅ |
| tmpfs rename | ✅ |
| tmpfs symlink/readlink | ✅ |
| tmpfs unlink | ✅ |
| tmpfs readdir | ✅ |
| devfs /dev/null | ✅ |
| devfs /dev/zero | ✅ |
| devfs /dev/console | ✅ |

## 后续 (M9 集成)

1. **FileTable 挂入 Process**
   ```rust
   pub struct Process {
       pub file_table: IrqSpinLock<Option<FileTable<128>>>,
       pub root: IrqSpinLock<Option<DentryRef>>,
       pub cwd: IrqSpinLock<Option<DentryRef>>,
   }
   ```

2. **路径解析 + 挂载表**
   - `resolve_path(dir_fd, path, cwd, file_table) → DentryRef`
   - `MOUNT_TABLE: IrqSpinLock<Option<MountTable>>`

3. **15 个 syscall 接入**
   | Syscall | 编号 | 功能 |
   |---------|------|------|
   | openat | 56 | 打开/创建文件 |
   | close | 57 | 关闭 fd |
   | read | 63 | 读取文件 |
   | write | 64 | 写入文件 |
   | lseek | 62 | 移动文件偏移 |
   | fstat | 80 | 获取已打开文件信息 |
   | newfstatat | 79 | 按路径获取文件信息 |
   | getdents64 | 61 | 读取目录项 |
   | mkdirat | 34 | 创建目录 |
   | unlinkat | 35 | 删除文件 |
   | renameat | 38 | 重命名 |
   | dup | 23 | 复制 fd |
   | dup3 | 24 | 复制 fd 到指定编号 |
   | fcntl | 25 | fd 控制 |
   | ioctl | 29 | 设备控制 |
