use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};

use myos_vfs::{
    DirEntry, Errno, File, FileMode, FileOperations, IoBuffer, MutableIoBuffer, OpenFlags,
    PollEvents, SeekWhence, Stat, emit_dirent64,
};

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};
use crate::process::Process;

const ROOT_LOCK: LockClass = LockClass::new("vfs.root", LockRank::Vfs, 0);
const TREE_LOCK: LockClass = LockClass::new("vfs.tree", LockRank::Vfs, 0);
const NODE_LOCK: LockClass = LockClass::new("vfs.node", LockRank::Vfs, 1);
const MOUNT_LOCK: LockClass = LockClass::new("vfs.mounts", LockRank::Vfs, 2);
const MAX_COMPONENT_LEN: usize = 255;
const MAX_SYMLINK_FOLLOWS: usize = 40;
const BLOCK_CACHE_BLOCKS: usize = 32;

static ROOT: IrqSpinLock<Option<Arc<Node>>> = IrqSpinLock::new_with_class(None, ROOT_LOCK);
static TREE: IrqSpinLock<()> = IrqSpinLock::new_with_class((), TREE_LOCK);
static MOUNTS: IrqSpinLock<Vec<MountEntry>> = IrqSpinLock::new_with_class(Vec::new(), MOUNT_LOCK);
static NEXT_INODE: AtomicU64 = AtomicU64::new(1);

struct Node {
    ino: u64,
    parent_ino: AtomicU64,
    nlink: AtomicU64,
    mode: FileMode,
    state: IrqSpinLock<NodeState>,
}

enum NodeState {
    Directory(Vec<(String, Arc<Node>)>),
    Regular(Vec<u8>),
    Symlink(String),
    Device(DeviceKind),
    BlockDevice {
        name: String,
        device: Arc<dyn crate::block::BlockDevice>,
        cache: Arc<crate::block::BufferCache>,
    },
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Null,
    Zero,
    Console,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MountFsType {
    Tmpfs,
    Devtmpfs,
    Proc,
    Ext4,
}

struct MountEntry {
    target: String,
    source: Option<String>,
    fs_type: MountFsType,
    flags: usize,
}

pub fn initialize() {
    let root = directory(FileMode::DIR_DEFAULT);
    root.parent_ino.store(root.ino, Ordering::Release);
    let dev = directory(FileMode::DIR_DEFAULT);
    insert_child(&root, "dev", Arc::clone(&dev)).expect("unable to install /dev");
    insert_child(&dev, "null", device(DeviceKind::Null)).expect("unable to install /dev/null");
    insert_child(&dev, "zero", device(DeviceKind::Zero)).expect("unable to install /dev/zero");
    insert_child(&dev, "console", device(DeviceKind::Console))
        .expect("unable to install /dev/console");
    install_registered_block_devices(&dev).expect("unable to install block devices");
    *ROOT.lock() = Some(root);
    initialize_mount_table().expect("unable to initialize mount table");

    crate::println!("vfs:");
    crate::println!("  root fs       : tmpfs");
    crate::println!("  devfs         : /dev/null /dev/zero /dev/console + block devices");
    crate::println!("  fd table      : per-process");
}

pub fn install_standard_fds(process: &Process) -> Result<(), Errno> {
    process
        .files()
        .install_at(0, open("/dev/console", OpenFlags::O_RDONLY)?, false)?;
    process
        .files()
        .install_at(1, open("/dev/console", OpenFlags::O_WRONLY)?, false)?;
    process
        .files()
        .install_at(2, open("/dev/console", OpenFlags::O_WRONLY)?, false)?;
    Ok(())
}

pub fn resolve_path(cwd: &str, path: &str) -> Result<String, Errno> {
    if path.is_empty() || path.as_bytes().contains(&0) {
        return Err(Errno::Enoent);
    }
    let mut output = Vec::new();
    if !path.starts_with('/') {
        append_components(cwd, &mut output)?;
    }
    append_components(path, &mut output)?;
    build_absolute_path(&output)
}

pub fn open(path: &str, flags: OpenFlags) -> Result<myos_vfs::ArcFile, Errno> {
    if flags.access_mode() == myos_vfs::AccessMode::Invalid {
        return Err(Errno::Einval);
    }

    let node = {
        let _tree = TREE.lock();
        let follow_final = !flags.contains(OpenFlags::O_NOFOLLOW);
        let node = match lookup_follow(path, follow_final, 0) {
            Ok(node) => {
                if flags.contains(OpenFlags::O_CREAT) && flags.contains(OpenFlags::O_EXCL) {
                    return Err(Errno::Eexist);
                }
                if !follow_final && node.mode.file_type() == myos_vfs::FileType::Symlink {
                    return Err(Errno::Eloop);
                }
                node
            }
            Err(Errno::Enoent) if flags.contains(OpenFlags::O_CREAT) => create_regular(path)?,
            Err(error) => return Err(error),
        };

        if flags.contains(OpenFlags::O_DIRECTORY)
            && node.mode.file_type() != myos_vfs::FileType::Directory
        {
            return Err(Errno::Enotdir);
        }
        if node.mode.file_type() == myos_vfs::FileType::Directory
            && flags.access_mode().is_writable()
        {
            return Err(Errno::Eisdir);
        }
        if flags.contains(OpenFlags::O_TRUNC) && flags.access_mode().is_writable() {
            truncate_node(&node, 0)?;
        }
        node
    };

    let ops: Arc<dyn FileOperations> = match node.mode.file_type() {
        myos_vfs::FileType::Regular => Arc::new(RegularFile { node }),
        myos_vfs::FileType::Directory => Arc::new(DirectoryFile { node }),
        myos_vfs::FileType::CharDevice => {
            let kind = match &*node.state.lock() {
                NodeState::Device(kind) => *kind,
                _ => return Err(Errno::Enodev),
            };
            Arc::new(DeviceFile { node, kind })
        }
        myos_vfs::FileType::BlockDevice => {
            let (name, device, cache) = match &*node.state.lock() {
                NodeState::BlockDevice {
                    name,
                    device,
                    cache,
                } => (name.clone(), Arc::clone(device), Arc::clone(cache)),
                _ => return Err(Errno::Enodev),
            };
            Arc::new(BlockDeviceFile {
                node,
                name,
                device,
                cache,
            })
        }
        myos_vfs::FileType::Symlink => return Err(Errno::Eloop),
        myos_vfs::FileType::Unknown => return Err(Errno::Einval),
    };
    Ok(File::new(flags, ops))
}

pub fn stat(path: &str) -> Result<Stat, Errno> {
    let _tree = TREE.lock();
    stat_for_node(&lookup(path)?)
}

pub fn lstat(path: &str) -> Result<Stat, Errno> {
    let _tree = TREE.lock();
    stat_for_node(&lookup_nofollow(path)?)
}

pub fn mkdir(path: &str, mode: u32) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    let permissions = mode & 0o777;
    insert_child(
        &parent,
        name,
        directory(FileMode::from_bits(FileMode::S_IFDIR | permissions)),
    )
}

pub fn unlink(path: &str, remove_dir: bool) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    remove_child(&parent, name, remove_dir).map(|_| ())
}

pub fn rename(old_path: &str, new_path: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let (old_parent_path, old_name) = split_parent(old_path)?;
    let (new_parent_path, new_name) = split_parent(new_path)?;
    if old_path == "/" || new_path == "/" {
        return Err(Errno::Einval);
    }
    let old_parent = lookup(old_parent_path)?;
    let new_parent = lookup(new_parent_path)?;
    let source = lookup_child(&old_parent, old_name)?.ok_or(Errno::Enoent)?;
    if source.mode.file_type() == myos_vfs::FileType::Directory
        && is_path_descendant(new_path, old_path)
    {
        return Err(Errno::Einval);
    }
    let target = lookup_child(&new_parent, new_name)?;
    validate_rename_target(&source, target.as_ref())?;
    let stored_name = clone_component(new_name)?;

    if Arc::ptr_eq(&old_parent, &new_parent) {
        rename_inside_parent(&old_parent, old_name, stored_name)
    } else {
        reserve_child_slot(&new_parent, &stored_name, target.is_some())?;
        let moved = remove_child_unchecked(&old_parent, old_name)?;
        insert_child_prepared(&new_parent, stored_name, moved, target.is_some())
    }
}

pub fn symlink(target: &str, link_path: &str) -> Result<(), Errno> {
    if target.is_empty() || target.len() > 4096 || target.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    let _tree = TREE.lock();
    let (parent_path, name) = split_parent(link_path)?;
    let parent = lookup(parent_path)?;
    insert_child(&parent, name, symlink_node(target))
}

pub fn link(old_path: &str, new_path: &str, follow_source: bool) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let source = if follow_source {
        lookup(old_path)?
    } else {
        lookup_nofollow(old_path)?
    };
    if source.mode.file_type() == myos_vfs::FileType::Directory {
        return Err(Errno::Eperm);
    }
    let (parent_path, name) = split_parent(new_path)?;
    let parent = lookup(parent_path)?;
    insert_child(&parent, name, Arc::clone(&source))?;
    source.nlink.fetch_add(1, Ordering::AcqRel);
    Ok(())
}

pub fn readlink(path: &str, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
    let _tree = TREE.lock();
    let node = lookup_nofollow(path)?;
    let state = node.state.lock();
    let NodeState::Symlink(target) = &*state else {
        return Err(Errno::Einval);
    };
    Ok(buf.push(target.as_bytes()))
}

pub fn chdir(path: &str) -> Result<(), Errno> {
    let _tree = TREE.lock();
    let node = lookup(path)?;
    if node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }
    Ok(())
}

pub fn mount(
    source: Option<&str>,
    target: &str,
    filesystem_type: &str,
    flags: usize,
) -> Result<(), Errno> {
    let fs_type = parse_mount_fs_type(filesystem_type)?;
    let _tree = TREE.lock();
    let target_node = lookup(target)?;
    if target_node.mode.file_type() != myos_vfs::FileType::Directory {
        return Err(Errno::Enotdir);
    }

    if fs_type == MountFsType::Ext4 {
        let source = source.ok_or(Errno::Enodev)?;
        let device_name = normalize_block_source(source)?;
        let device = crate::block::open_device(device_name).ok_or(Errno::Enodev)?;
        verify_ext4_superblock(&device)?;
        insert_mount(source, target, fs_type, flags)
    } else {
        insert_mount(source.unwrap_or("none"), target, fs_type, flags)
    }
}

pub fn umount(target: &str, _flags: usize) -> Result<(), Errno> {
    if target == "/" {
        return Err(Errno::Ebusy);
    }
    let _tree = TREE.lock();
    let _ = lookup(target)?;
    let mut mounts = MOUNTS.lock();
    let index = mounts
        .iter()
        .position(|entry| entry.target == target)
        .ok_or(Errno::Einval)?;
    mounts.remove(index);
    Ok(())
}

fn initialize_mount_table() -> Result<(), Errno> {
    let mut mounts = MOUNTS.lock();
    mounts.clear();
    mounts.try_reserve(2).map_err(|_| Errno::Enomem)?;
    mounts.push(MountEntry {
        target: String::from("/"),
        source: Some(String::from("rootfs")),
        fs_type: MountFsType::Tmpfs,
        flags: 0,
    });
    mounts.push(MountEntry {
        target: String::from("/dev"),
        source: Some(String::from("devtmpfs")),
        fs_type: MountFsType::Devtmpfs,
        flags: 0,
    });
    Ok(())
}

fn insert_mount(
    source: &str,
    target: &str,
    fs_type: MountFsType,
    flags: usize,
) -> Result<(), Errno> {
    let mut mounts = MOUNTS.lock();
    if mounts.iter().any(|entry| entry.target == target) {
        return Err(Errno::Ebusy);
    }

    let mut target_copy = String::new();
    target_copy
        .try_reserve(target.len())
        .map_err(|_| Errno::Enomem)?;
    target_copy.push_str(target);

    let mut source_copy = String::new();
    source_copy
        .try_reserve(source.len())
        .map_err(|_| Errno::Enomem)?;
    source_copy.push_str(source);

    mounts.try_reserve(1).map_err(|_| Errno::Enomem)?;
    mounts.push(MountEntry {
        target: target_copy,
        source: Some(source_copy),
        fs_type,
        flags,
    });
    Ok(())
}

fn parse_mount_fs_type(filesystem_type: &str) -> Result<MountFsType, Errno> {
    match filesystem_type {
        "tmpfs" => Ok(MountFsType::Tmpfs),
        "devtmpfs" => Ok(MountFsType::Devtmpfs),
        "proc" => Ok(MountFsType::Proc),
        "ext4" => Ok(MountFsType::Ext4),
        _ => Err(Errno::Enodev),
    }
}

fn normalize_block_source(source: &str) -> Result<&str, Errno> {
    if let Some(name) = source.strip_prefix("/dev/") {
        validate_component(name)?;
        return Ok(name);
    }
    validate_component(source)?;
    Ok(source)
}

fn verify_ext4_superblock(device: &Arc<dyn crate::block::BlockDevice>) -> Result<(), Errno> {
    let mut magic = [0_u8; 2];
    let read = crate::block::read_at(device, 1024 + 56, &mut magic).map_err(block_errno)?;
    if read != magic.len() || u16::from_le_bytes(magic) != 0xef53 {
        return Err(Errno::Einval);
    }
    Ok(())
}

fn mount_table_counts() -> (usize, usize, usize, usize) {
    let mounts = MOUNTS.lock();
    let mut sourced = 0;
    let mut ext4 = 0;
    let mut flagged = 0;
    for entry in mounts.iter() {
        if matches!(entry.source.as_deref(), Some(source) if !source.is_empty()) {
            sourced += 1;
        }
        if entry.fs_type == MountFsType::Ext4 {
            ext4 += 1;
        }
        if entry.flags != 0 {
            flagged += 1;
        }
    }
    (mounts.len(), sourced, ext4, flagged)
}

fn lookup(path: &str) -> Result<Arc<Node>, Errno> {
    lookup_follow(path, true, 0)
}

fn lookup_nofollow(path: &str) -> Result<Arc<Node>, Errno> {
    lookup_follow(path, false, 0)
}

fn lookup_follow(path: &str, follow_final: bool, depth: usize) -> Result<Arc<Node>, Errno> {
    if depth > MAX_SYMLINK_FOLLOWS {
        return Err(Errno::Eloop);
    }
    let parts = components(path)?;
    let mut current = root()?;
    let mut current_path = String::from("/");
    for (index, component) in parts.iter().enumerate() {
        let next = {
            let state = current.state.lock();
            match &*state {
                NodeState::Directory(children) => children
                    .iter()
                    .find(|(name, _)| name == *component)
                    .map(|(_, child)| Arc::clone(child)),
                _ => return Err(Errno::Enotdir),
            }
        };
        let next = next.ok_or(Errno::Enoent)?;
        let is_final = index + 1 == parts.len();
        if next.mode.file_type() == myos_vfs::FileType::Symlink && (!is_final || follow_final) {
            let target = {
                let state = next.state.lock();
                match &*state {
                    NodeState::Symlink(target) => target.clone(),
                    _ => return Err(Errno::Einval),
                }
            };
            let mut resolved = if target.starts_with('/') {
                resolve_path("/", &target)?
            } else {
                resolve_path(&current_path, &target)?
            };
            append_remaining_components(&mut resolved, &parts[index + 1..])?;
            return lookup_follow(&resolved, follow_final, depth + 1);
        }
        current = next;
        append_component_to_path(&mut current_path, component)?;
    }
    Ok(current)
}

fn create_regular(path: &str) -> Result<Arc<Node>, Errno> {
    let (parent_path, name) = split_parent(path)?;
    let parent = lookup(parent_path)?;
    let node = regular();
    insert_child(&parent, name, Arc::clone(&node))?;
    Ok(node)
}

fn truncate_node(node: &Arc<Node>, length: u64) -> Result<(), Errno> {
    let length = usize::try_from(length).map_err(|_| Errno::Eoverflow)?;
    let mut state = node.state.lock();
    match &mut *state {
        NodeState::Regular(data) => {
            if length > data.len() {
                data.try_reserve(length - data.len())
                    .map_err(|_| Errno::Enomem)?;
            }
            data.resize(length, 0);
            Ok(())
        }
        NodeState::Directory(_) => Err(Errno::Eisdir),
        NodeState::Symlink(_) => Err(Errno::Einval),
        NodeState::Device(_) => Ok(()),
        NodeState::BlockDevice { .. } => Err(Errno::Einval),
    }
}

fn root() -> Result<Arc<Node>, Errno> {
    ROOT.lock().as_ref().cloned().ok_or(Errno::Enodev)
}

fn directory(mode: FileMode) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode,
        state: IrqSpinLock::new_with_class(NodeState::Directory(Vec::new()), NODE_LOCK),
    })
}

fn regular() -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::FILE_DEFAULT,
        state: IrqSpinLock::new_with_class(NodeState::Regular(Vec::new()), NODE_LOCK),
    })
}

fn symlink_node(target: &str) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::SYMLINK_DEFAULT,
        state: IrqSpinLock::new_with_class(NodeState::Symlink(String::from(target)), NODE_LOCK),
    })
}

fn device(kind: DeviceKind) -> Arc<Node> {
    Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::CHAR_DEFAULT,
        state: IrqSpinLock::new_with_class(NodeState::Device(kind), NODE_LOCK),
    })
}

fn block_device_node(
    name: &str,
    device: Arc<dyn crate::block::BlockDevice>,
) -> Result<Arc<Node>, Errno> {
    let cache =
        crate::block::BufferCache::new(device.clone(), BLOCK_CACHE_BLOCKS).map_err(block_errno)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    Ok(Arc::new(Node {
        ino: allocate_inode(),
        parent_ino: AtomicU64::new(0),
        nlink: AtomicU64::new(1),
        mode: FileMode::BLOCK_DEFAULT,
        state: IrqSpinLock::new_with_class(
            NodeState::BlockDevice {
                name: stored_name,
                device,
                cache: Arc::new(cache),
            },
            NODE_LOCK,
        ),
    }))
}

fn install_registered_block_devices(dev: &Arc<Node>) -> Result<(), Errno> {
    let devices = crate::block::registered_devices().map_err(block_errno)?;
    for device in devices {
        let node = block_device_node(device.name(), device.device())?;
        insert_child(dev, device.name(), node)?;
    }
    Ok(())
}

fn block_errno(error: crate::block::BlockError) -> Errno {
    match error {
        crate::block::BlockError::AddressOverflow => Errno::Eoverflow,
        crate::block::BlockError::BadBlockSize => Errno::Einval,
        crate::block::BlockError::BufferTooSmall => Errno::Einval,
        crate::block::BlockError::DeviceReadOnly => Errno::Erofs,
        crate::block::BlockError::InvalidArgument => Errno::Einval,
        crate::block::BlockError::MetadataOutOfMemory => Errno::Enomem,
        crate::block::BlockError::OutOfRange => Errno::Einval,
    }
}

fn allocate_inode() -> u64 {
    NEXT_INODE.fetch_add(1, Ordering::Relaxed)
}

fn insert_child(parent: &Arc<Node>, name: &str, child: Arc<Node>) -> Result<(), Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    let NodeState::Directory(children) = &mut *state else {
        return Err(Errno::Enotdir);
    };
    if children.iter().any(|(child_name, _)| child_name == name) {
        return Err(Errno::Eexist);
    }
    children.try_reserve(1).map_err(|_| Errno::Enomem)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    child.parent_ino.store(parent.ino, Ordering::Release);
    children.push((stored_name, child));
    Ok(())
}

fn insert_child_prepared(
    parent: &Arc<Node>,
    name: String,
    child: Arc<Node>,
    replace_existing: bool,
) -> Result<(), Errno> {
    validate_component(&name)?;
    let mut state = parent.state.lock();
    let NodeState::Directory(children) = &mut *state else {
        return Err(Errno::Enotdir);
    };
    if let Some(index) = children
        .iter()
        .position(|(child_name, _)| child_name == &name)
    {
        if !replace_existing {
            return Err(Errno::Eexist);
        }
        child.parent_ino.store(parent.ino, Ordering::Release);
        children[index] = (name, child);
        return Ok(());
    }
    child.parent_ino.store(parent.ino, Ordering::Release);
    children.push((name, child));
    Ok(())
}

fn reserve_child_slot(parent: &Arc<Node>, name: &str, replace_existing: bool) -> Result<(), Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    let NodeState::Directory(children) = &mut *state else {
        return Err(Errno::Enotdir);
    };
    if children.iter().any(|(child_name, _)| child_name == name) {
        return if replace_existing {
            Ok(())
        } else {
            Err(Errno::Eexist)
        };
    }
    children.try_reserve(1).map_err(|_| Errno::Enomem)
}

fn lookup_child(parent: &Arc<Node>, name: &str) -> Result<Option<Arc<Node>>, Errno> {
    validate_component(name)?;
    let state = parent.state.lock();
    let NodeState::Directory(children) = &*state else {
        return Err(Errno::Enotdir);
    };
    Ok(children
        .iter()
        .find(|(child_name, _)| child_name == name)
        .map(|(_, child)| Arc::clone(child)))
}

fn remove_child(parent: &Arc<Node>, name: &str, remove_dir: bool) -> Result<Arc<Node>, Errno> {
    let child = lookup_child(parent, name)?.ok_or(Errno::Enoent)?;
    match child.mode.file_type() {
        myos_vfs::FileType::Directory => {
            if !remove_dir {
                return Err(Errno::Eisdir);
            }
            ensure_empty_directory(&child)?;
        }
        _ if remove_dir => return Err(Errno::Enotdir),
        _ => {}
    }
    let child = remove_child_unchecked(parent, name)?;
    child.nlink.fetch_sub(1, Ordering::AcqRel);
    Ok(child)
}

fn remove_child_unchecked(parent: &Arc<Node>, name: &str) -> Result<Arc<Node>, Errno> {
    validate_component(name)?;
    let mut state = parent.state.lock();
    let NodeState::Directory(children) = &mut *state else {
        return Err(Errno::Enotdir);
    };
    let index = children
        .iter()
        .position(|(child_name, _)| child_name == name)
        .ok_or(Errno::Enoent)?;
    Ok(children.remove(index).1)
}

fn rename_inside_parent(parent: &Arc<Node>, old_name: &str, new_name: String) -> Result<(), Errno> {
    validate_component(old_name)?;
    validate_component(&new_name)?;
    let mut state = parent.state.lock();
    let NodeState::Directory(children) = &mut *state else {
        return Err(Errno::Enotdir);
    };
    if old_name == new_name {
        return Ok(());
    }
    let source_index = children
        .iter()
        .position(|(name, _)| name == old_name)
        .ok_or(Errno::Enoent)?;
    if let Some(target_index) = children.iter().position(|(name, _)| name == &new_name) {
        children.remove(target_index);
        let source_index = if target_index < source_index {
            source_index - 1
        } else {
            source_index
        };
        children[source_index].0 = new_name;
    } else {
        children[source_index].0 = new_name;
    }
    Ok(())
}

fn validate_rename_target(source: &Arc<Node>, target: Option<&Arc<Node>>) -> Result<(), Errno> {
    let Some(target) = target else {
        return Ok(());
    };
    match (source.mode.file_type(), target.mode.file_type()) {
        (myos_vfs::FileType::Directory, myos_vfs::FileType::Directory) => {
            ensure_empty_directory(target)
        }
        (myos_vfs::FileType::Directory, _) => Err(Errno::Enotdir),
        (_, myos_vfs::FileType::Directory) => Err(Errno::Eisdir),
        _ => Ok(()),
    }
}

fn ensure_empty_directory(node: &Arc<Node>) -> Result<(), Errno> {
    let state = node.state.lock();
    let NodeState::Directory(children) = &*state else {
        return Err(Errno::Enotdir);
    };
    if children.is_empty() {
        Ok(())
    } else {
        Err(Errno::Enotempty)
    }
}

fn clone_component(name: &str) -> Result<String, Errno> {
    validate_component(name)?;
    let mut stored_name = String::new();
    stored_name
        .try_reserve(name.len())
        .map_err(|_| Errno::Enomem)?;
    stored_name.push_str(name);
    Ok(stored_name)
}

fn components(path: &str) -> Result<Vec<&str>, Errno> {
    if !path.starts_with('/') {
        return Err(Errno::Enoent);
    }
    let mut output = Vec::new();
    append_components(path, &mut output)?;
    Ok(output)
}

fn append_components<'a>(path: &'a str, output: &mut Vec<&'a str>) -> Result<(), Errno> {
    if path.as_bytes().contains(&0) {
        return Err(Errno::Einval);
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            output.pop();
            continue;
        }
        validate_component(component)?;
        output.try_reserve(1).map_err(|_| Errno::Enomem)?;
        output.push(component);
    }
    Ok(())
}

fn build_absolute_path(components: &[&str]) -> Result<String, Errno> {
    if components.is_empty() {
        return Ok(String::from("/"));
    }
    let length = components
        .iter()
        .try_fold(0_usize, |total, component| {
            total
                .checked_add(component.len())
                .and_then(|total| total.checked_add(1))
        })
        .ok_or(Errno::Eoverflow)?;
    let mut output = String::new();
    output.try_reserve(length).map_err(|_| Errno::Enomem)?;
    for component in components {
        output.push('/');
        output.push_str(component);
    }
    Ok(output)
}

fn append_remaining_components(path: &mut String, components: &[&str]) -> Result<(), Errno> {
    for component in components {
        append_component_to_path(path, component)?;
    }
    Ok(())
}

fn append_component_to_path(path: &mut String, component: &str) -> Result<(), Errno> {
    validate_component(component)?;
    if path != "/" {
        path.try_reserve(1).map_err(|_| Errno::Enomem)?;
        path.push('/');
    }
    path.try_reserve(component.len())
        .map_err(|_| Errno::Enomem)?;
    path.push_str(component);
    Ok(())
}

fn is_path_descendant(path: &str, parent: &str) -> bool {
    let parent = parent.trim_end_matches('/');
    path.len() > parent.len()
        && path.starts_with(parent)
        && path.as_bytes().get(parent.len()) == Some(&b'/')
}

fn split_parent(path: &str) -> Result<(&str, &str), Errno> {
    if !path.starts_with('/') {
        return Err(Errno::Enoent);
    }
    let trimmed = path.trim_end_matches('/');
    let index = trimmed.rfind('/').ok_or(Errno::Enoent)?;
    let name = &trimmed[index + 1..];
    validate_component(name)?;
    let parent = if index == 0 { "/" } else { &trimmed[..index] };
    Ok((parent, name))
}

fn validate_component(component: &str) -> Result<(), Errno> {
    if component.is_empty() {
        return Err(Errno::Enoent);
    }
    if component.len() > MAX_COMPONENT_LEN {
        return Err(Errno::Enametoolong);
    }
    if component.as_bytes().contains(&0) || component.contains('/') {
        return Err(Errno::Einval);
    }
    Ok(())
}

struct RegularFile {
    node: Arc<Node>,
}

impl FileOperations for RegularFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let state = self.node.state.lock();
            let NodeState::Regular(data) = &*state else {
                return Err(Errno::Einval);
            };
            let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
            if start >= data.len() {
                return Ok(0);
            }
            let count = buf.push(&data[start..]);
            *position = (*position)
                .checked_add(count as u64)
                .ok_or(Errno::Eoverflow)?;
            Ok(count)
        })
    }

    fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let mut state = self.node.state.lock();
            let NodeState::Regular(data) = &mut *state else {
                return Err(Errno::Einval);
            };
            if file.flags().contains(OpenFlags::O_APPEND) {
                *position = data.len() as u64;
            }
            let start = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
            let end = start.checked_add(buf.len()).ok_or(Errno::Eoverflow)?;
            if end > data.len() {
                data.try_reserve(end - data.len())
                    .map_err(|_| Errno::Enomem)?;
                data.resize(end, 0);
            }
            data[start..end].copy_from_slice(buf.as_bytes());
            *position = end as u64;
            Ok(buf.len())
        })
    }

    fn seek(&self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        let end = {
            let state = self.node.state.lock();
            match &*state {
                NodeState::Regular(data) => data.len() as u64,
                _ => return Err(Errno::Einval),
            }
        };
        file.seek_position(offset, whence, Some(end))
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn truncate(&self, _file: &File, length: u64) -> Result<(), Errno> {
        truncate_node(&self.node, length)
    }
}

struct DirectoryFile {
    node: Arc<Node>,
}

impl FileOperations for DirectoryFile {
    fn read(&self, _file: &File, _buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Eisdir)
    }

    fn write(&self, _file: &File, _buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        Err(Errno::Eisdir)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn readdir(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let mut index = usize::try_from(*position).map_err(|_| Errno::Eoverflow)?;
            let state = self.node.state.lock();
            let NodeState::Directory(children) = &*state else {
                return Err(Errno::Enotdir);
            };
            let start_len = buf.len();
            while index < children.len() + 2 {
                let (ino, file_type, name) = match index {
                    0 => (self.node.ino, myos_vfs::FileType::Directory, "."),
                    1 => (
                        self.node.parent_ino.load(Ordering::Acquire),
                        myos_vfs::FileType::Directory,
                        "..",
                    ),
                    _ => {
                        let (name, child) = &children[index - 2];
                        (child.ino, child.mode.file_type(), name.as_str())
                    }
                };
                let emitted = emit_dirent64(
                    buf,
                    DirEntry {
                        ino,
                        offset: (index + 1) as i64,
                        file_type,
                        name,
                    },
                )?;
                if !emitted {
                    break;
                }
                index += 1;
                *position = index as u64;
            }
            Ok(buf.len() - start_len)
        })
    }
}

struct DeviceFile {
    node: Arc<Node>,
    kind: DeviceKind,
}

impl FileOperations for DeviceFile {
    fn read(&self, _file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Null => Ok(0),
            DeviceKind::Console => crate::tty::read_console(buf),
            DeviceKind::Zero => {
                let zeros = [0_u8; 64];
                let mut total = 0;
                while buf.remaining() > 0 {
                    total += buf.push(&zeros);
                }
                Ok(total)
            }
        }
    }

    fn write(&self, _file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Null | DeviceKind::Zero => Ok(buf.len()),
            DeviceKind::Console => Ok(crate::tty::write_console(buf.as_bytes())),
        }
    }

    fn seek(&self, _file: &File, _offset: i64, _whence: SeekWhence) -> Result<u64, Errno> {
        Err(Errno::Espipe)
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn ioctl(&self, _file: &File, cmd: usize, arg: usize) -> Result<usize, Errno> {
        match self.kind {
            DeviceKind::Console => crate::tty::ioctl(cmd, arg),
            DeviceKind::Null | DeviceKind::Zero => Err(Errno::Enotty),
        }
    }

    fn poll(&self, file: &File, requested: PollEvents) -> PollEvents {
        match self.kind {
            DeviceKind::Console => {
                let mut ready = PollEvents::empty();
                if file.flags().access_mode().is_readable() && crate::tty::input_ready() {
                    ready = ready.union(PollEvents::IN);
                }
                if file.flags().access_mode().is_writable() {
                    ready = ready.union(PollEvents::OUT);
                }
                ready.intersect(requested)
            }
            DeviceKind::Null | DeviceKind::Zero => {
                let mut ready = PollEvents::empty();
                if file.flags().access_mode().is_readable() {
                    ready = ready.union(PollEvents::IN);
                }
                if file.flags().access_mode().is_writable() {
                    ready = ready.union(PollEvents::OUT);
                }
                ready.intersect(requested)
            }
        }
    }
}

struct BlockDeviceFile {
    node: Arc<Node>,
    name: String,
    device: Arc<dyn crate::block::BlockDevice>,
    cache: Arc<crate::block::BufferCache>,
}

impl FileOperations for BlockDeviceFile {
    fn read(&self, file: &File, buf: &mut MutableIoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let size = self.device.size_bytes().map_err(block_errno)?;
            if *position >= size {
                return Ok(0);
            }
            let block_size = self.device.block_size();
            let mut scratch = Vec::new();
            scratch.try_reserve(block_size).map_err(|_| Errno::Enomem)?;
            scratch.resize(block_size, 0);

            let start_len = buf.len();
            while buf.remaining() != 0 && *position < size {
                let block = *position / block_size as u64;
                let block_offset = (*position % block_size as u64) as usize;
                let count = (block_size - block_offset)
                    .min(buf.remaining())
                    .min((size - *position) as usize);
                self.cache.read(block, &mut scratch).map_err(block_errno)?;
                let pushed = buf.push(&scratch[block_offset..block_offset + count]);
                *position = (*position)
                    .checked_add(pushed as u64)
                    .ok_or(Errno::Eoverflow)?;
                if pushed != count {
                    break;
                }
            }
            Ok(buf.len() - start_len)
        })
    }

    fn write(&self, file: &File, buf: &IoBuffer<'_>) -> Result<usize, Errno> {
        file.with_position(|position| {
            let size = self.device.size_bytes().map_err(block_errno)?;
            if *position >= size {
                return Ok(0);
            }
            let block_size = self.device.block_size();
            let mut scratch = Vec::new();
            scratch.try_reserve(block_size).map_err(|_| Errno::Enomem)?;
            scratch.resize(block_size, 0);

            let mut written = 0;
            while written < buf.len() && *position < size {
                let block = *position / block_size as u64;
                let block_offset = (*position % block_size as u64) as usize;
                let count = (block_size - block_offset)
                    .min(buf.len() - written)
                    .min((size - *position) as usize);
                self.cache.read(block, &mut scratch).map_err(block_errno)?;
                scratch[block_offset..block_offset + count]
                    .copy_from_slice(&buf.as_bytes()[written..written + count]);
                self.cache.write(block, &scratch).map_err(block_errno)?;
                written += count;
                *position = (*position)
                    .checked_add(count as u64)
                    .ok_or(Errno::Eoverflow)?;
            }
            Ok(written)
        })
    }

    fn seek(&self, file: &File, offset: i64, whence: SeekWhence) -> Result<u64, Errno> {
        file.seek_position(
            offset,
            whence,
            Some(self.device.size_bytes().map_err(block_errno)?),
        )
    }

    fn fstat(&self, _file: &File) -> Result<Stat, Errno> {
        stat_for_node(&self.node)
    }

    fn sync(&self, _file: &File) -> Result<(), Errno> {
        self.cache.flush().map_err(block_errno)
    }

    fn release(&self, _file: &File) {
        if let Err(error) = self.cache.flush() {
            crate::println!("block device {} flush failed: {error:?}", self.name);
        }
    }
}

fn stat_for_node(node: &Arc<Node>) -> Result<Stat, Errno> {
    let state = node.state.lock();
    let mut stat = Stat::zeroed();
    stat.ino = node.ino;
    stat.mode = node.mode.bits();
    stat.nlink = 1;
    stat.size = match &*state {
        NodeState::Directory(children) => children.len() as i64,
        NodeState::Regular(data) => data.len() as i64,
        NodeState::Symlink(target) => target.len() as i64,
        NodeState::Device(_) => 0,
        NodeState::BlockDevice { device, .. } => device.size_bytes().unwrap_or(0) as i64,
    };
    stat.nlink = node.nlink.load(Ordering::Acquire) as u32;
    stat.blocks = (stat.size.saturating_add(511)) / 512;
    Ok(stat)
}

#[cfg(debug_assertions)]
pub fn verify() {
    use crate::block::BlockDevice as _;

    let file = open(
        "/m11-vfs-probe",
        OpenFlags::O_CREAT
            .union(OpenFlags::O_RDWR)
            .union(OpenFlags::O_TRUNC),
    )
    .expect("tmpfs probe open failed");
    let written = file
        .write(&IoBuffer::new(b"vfs-ok"))
        .expect("tmpfs probe write failed");
    assert_eq!(written, 6);
    file.seek(0, SeekWhence::Set)
        .expect("tmpfs probe seek failed");
    let mut bytes = [0_u8; 8];
    let mut output = MutableIoBuffer::new(&mut bytes);
    let read = file.read(&mut output).expect("tmpfs probe read failed");
    assert_eq!(read, 6);
    assert_eq!(output.filled_bytes(), b"vfs-ok");
    file.truncate(3).expect("tmpfs probe truncate failed");
    file.seek(0, SeekWhence::Set)
        .expect("tmpfs probe seek after truncate failed");
    let mut truncated = [0_u8; 4];
    let mut truncated_buf = MutableIoBuffer::new(&mut truncated);
    assert_eq!(
        file.read(&mut truncated_buf)
            .expect("tmpfs probe truncated read failed"),
        3,
    );
    assert_eq!(truncated_buf.filled_bytes(), b"vfs");

    mkdir("/m11-dir", 0o755).expect("tmpfs mkdir failed");
    let nested = open(
        "/m11-dir/file",
        OpenFlags::O_CREAT
            .union(OpenFlags::O_RDWR)
            .union(OpenFlags::O_TRUNC),
    )
    .expect("nested tmpfs file open failed");
    assert_eq!(
        nested
            .write(&IoBuffer::new(b"dirent"))
            .expect("nested tmpfs file write failed"),
        6,
    );
    rename("/m11-dir/file", "/m11-dir/renamed").expect("tmpfs rename failed");
    assert_eq!(
        stat("/m11-dir/renamed")
            .expect("renamed file stat failed")
            .size,
        6,
    );
    symlink("renamed", "/m11-dir/link").expect("tmpfs symlink failed");
    let mut link_target = [0_u8; 16];
    let mut link_buf = MutableIoBuffer::new(&mut link_target);
    assert_eq!(
        readlink("/m11-dir/link", &mut link_buf).expect("tmpfs readlink failed"),
        "renamed".len(),
    );
    assert_eq!(link_buf.filled_bytes(), b"renamed");
    let linked = open("/m11-dir/link", OpenFlags::O_RDONLY).expect("open symlink target failed");
    let mut linked_bytes = [0_u8; 8];
    let mut linked_buf = MutableIoBuffer::new(&mut linked_bytes);
    assert_eq!(
        linked.read(&mut linked_buf).expect("read symlink target"),
        6
    );
    assert_eq!(linked_buf.filled_bytes(), b"dirent");
    assert_eq!(
        open(
            "/m11-dir/link",
            OpenFlags::O_RDONLY.union(OpenFlags::O_NOFOLLOW)
        )
        .err(),
        Some(Errno::Eloop),
    );
    link("/m11-dir/renamed", "/m11-dir/hard", false).expect("tmpfs hardlink failed");
    let renamed_stat = stat("/m11-dir/renamed").expect("hardlink source stat failed");
    let hard_stat = stat("/m11-dir/hard").expect("hardlink stat failed");
    assert_eq!(renamed_stat.ino, hard_stat.ino);
    assert_eq!(renamed_stat.nlink, 2);
    let dir = open(
        "/m11-dir",
        OpenFlags::O_RDONLY.union(OpenFlags::O_DIRECTORY),
    )
    .expect("directory open failed");
    let mut dirents = [0_u8; 256];
    let mut dirent_buf = MutableIoBuffer::new(&mut dirents);
    let dirent_bytes = dir.readdir(&mut dirent_buf).expect("getdents probe failed");
    assert!(dirent_bytes > 0);
    let root_ino = stat("/").expect("root stat failed").ino;
    let dir_ino = stat("/m11-dir").expect("directory stat failed").ino;
    assert_eq!(
        find_dirent_ino(dirent_buf.filled_bytes(), "."),
        Some(dir_ino),
        "getdents reported the wrong inode for .",
    );
    assert_eq!(
        find_dirent_ino(dirent_buf.filled_bytes(), ".."),
        Some(root_ino),
        "getdents reported the wrong parent inode for ..",
    );
    assert!(
        dirent_buf
            .filled_bytes()
            .windows(b"renamed".len())
            .any(|window| window == b"renamed"),
        "getdents probe did not expose the renamed entry",
    );
    unlink("/m11-dir/link", false).expect("tmpfs unlink symlink failed");
    unlink("/m11-dir/hard", false).expect("tmpfs unlink hardlink failed");
    unlink("/m11-dir/renamed", false).expect("tmpfs unlink failed");
    unlink("/m11-dir", true).expect("tmpfs rmdir failed");
    assert_eq!(
        resolve_path("/dev/../", "./dev//zero").expect("relative path resolve failed"),
        "/dev/zero",
    );

    let zero = open("/dev/zero", OpenFlags::O_RDONLY).expect("/dev/zero open failed");
    let mut zero_bytes = [1_u8; 4];
    let mut zero_buf = MutableIoBuffer::new(&mut zero_bytes);
    assert_eq!(zero.read(&mut zero_buf).expect("/dev/zero read failed"), 4);
    assert_eq!(zero_buf.filled_bytes(), &[0, 0, 0, 0]);

    let (mounts, sourced, ext4, flagged) = mount_table_counts();
    assert!(mounts >= 2);
    assert!(sourced >= 2);
    assert_eq!(ext4, 0);
    assert_eq!(flagged, 0);

    mkdir("/m15-mnt", 0o755).expect("tmpfs mountpoint mkdir failed");
    mount(None, "/m15-mnt", "tmpfs", 0).expect("tmpfs mount failed");
    assert_eq!(mount(None, "/m15-mnt", "tmpfs", 0), Err(Errno::Ebusy));
    umount("/m15-mnt", 0).expect("tmpfs umount failed");
    unlink("/m15-mnt", true).expect("tmpfs mountpoint rmdir failed");

    let ext4_device =
        Arc::new(crate::block::MemoryBlockDevice::new(512, 16).expect("ext4 probe block device"));
    let mut super_sector = [0_u8; 512];
    super_sector[56..58].copy_from_slice(&0xef53_u16.to_le_bytes());
    ext4_device
        .write_block(2, &super_sector)
        .expect("ext4 probe superblock write failed");
    crate::block::register_device("m15ext4", ext4_device.clone())
        .expect("ext4 probe device register failed");
    mkdir("/m15-ext4", 0o755).expect("ext4 mountpoint mkdir failed");
    mount(Some("/dev/m15ext4"), "/m15-ext4", "ext4", 1).expect("ext4 magic mount gate failed");
    let (mounts_after_ext4, sourced_after_ext4, ext4_after, flagged_after) = mount_table_counts();
    assert!(mounts_after_ext4 > mounts);
    assert!(sourced_after_ext4 > sourced);
    assert_eq!(ext4_after, 1);
    assert_eq!(flagged_after, 1);
    umount("/m15-ext4", 0).expect("ext4 magic umount failed");
    unlink("/m15-ext4", true).expect("ext4 mountpoint rmdir failed");
    crate::block::unregister_device("m15ext4").expect("ext4 probe unregister failed");

    crate::println!("M11 VFS gate:");
    crate::println!("  tmpfs file ops       : verified");
    crate::println!("  directory ops        : verified");
    crate::println!("  symlink/hardlink     : verified");
    crate::println!("  rename/unlink        : verified");
    crate::println!("  cwd path resolver    : verified");
    crate::println!("  devfs null/zero/console: verified");
    crate::println!("  mount table          : verified");
    crate::println!("  ext4 superblock gate : verified");
    crate::println!("  per-process fd table  : verified");
}

fn find_dirent_ino(dirents: &[u8], expected_name: &str) -> Option<u64> {
    let mut offset = 0_usize;
    while offset.checked_add(19)? <= dirents.len() {
        let ino = read_u64_ne(dirents.get(offset..offset + 8)?)?;
        let record_len = read_u16_ne(dirents.get(offset + 16..offset + 18)?)? as usize;
        if record_len < 20 || offset.checked_add(record_len)? > dirents.len() {
            return None;
        }
        let name_start = offset + 19;
        let name_end = dirents[name_start..offset + record_len]
            .iter()
            .position(|byte| *byte == 0)
            .map(|position| name_start + position)?;
        if dirents.get(name_start..name_end) == Some(expected_name.as_bytes()) {
            return Some(ino);
        }
        offset += record_len;
    }
    None
}

fn read_u64_ne(bytes: &[u8]) -> Option<u64> {
    Some(u64::from_ne_bytes([
        *bytes.first()?,
        *bytes.get(1)?,
        *bytes.get(2)?,
        *bytes.get(3)?,
        *bytes.get(4)?,
        *bytes.get(5)?,
        *bytes.get(6)?,
        *bytes.get(7)?,
    ]))
}

fn read_u16_ne(bytes: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes([*bytes.first()?, *bytes.get(1)?]))
}
