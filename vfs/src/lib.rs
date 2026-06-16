#![no_std]

extern crate alloc;

mod buffer;
mod dentry;
mod dentry_ops;
mod dev_t;
mod dirent;
mod errno;
mod fcntl;
mod file;
mod file_ops;
mod file_mode;
mod file_table;
mod file_type;
mod fs_type;
mod inode;
mod inode_id;
mod inode_ops;
mod open_flags;
mod pathname;
mod rename;
mod seek;
mod stat;
mod super_block;
mod super_ops;

// --- Base types ---
pub use buffer::{IoBuffer, MutableIoBuffer};
pub use dev_t::{Dev, DEV_CONSOLE, DEV_NULL, DEV_ZERO, DEV_MAJOR_MEM};
pub use dirent::DirEntry;
pub use errno::Errno;
pub use fcntl::FcntlCmd;
pub use file_mode::FileMode;
pub use file_type::FileType;
pub use inode_id::InodeId;
pub use open_flags::{AccessMode, OpenFlags};
pub use rename::RenameFlags;
pub use seek::SeekWhence;
pub use stat::{Stat, StatFs, DEVFS_MAGIC, EXT4_SUPER_MAGIC, TMPFS_MAGIC};

// --- Pathname parsing ---
pub use pathname::{PathComponents, PathError, MAX_NAME_LEN, MAX_PATH_DEPTH};

// --- VFS traits ---
pub use dentry_ops::DentryOperations;
pub use file_ops::{FileOperations, PollStatus, ReadDirEntries};
pub use fs_type::{FileSystemType, MS_BIND, MS_NOATIME, MS_NODEV, MS_NOEXEC, MS_NOSUID, MS_RDONLY, MS_REMOUNT, MS_SYNCHRONOUS};
pub use inode_ops::InodeOperations;
pub use super_ops::SuperBlockOperations;

// --- Core structures ---
pub use dentry::{Dentry, DentryRef};
pub use file::{ArcFile, File};
pub use file_table::{FileTable, FileTableError};
pub use inode::{ArcInode, Inode};
pub use super_block::{ArcSuperBlock, SuperBlock};
