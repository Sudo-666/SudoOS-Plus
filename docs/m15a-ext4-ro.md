# M15-A read-only ext4 VFS snapshot

This patch adds a conservative native ext4 read-only path:

```text
block device -> ext4 super/group/inode/extent parser -> VFS node snapshot
```

It deliberately does **not** claim full Linux ext4 parity. The M15-A boundary is:

- verify ext4 superblock and incompatible feature bits;
- read block group descriptor tables;
- read ext4 inodes;
- support extent-backed regular files and directories;
- support fast and extent-backed symlinks;
- mount the ext4 root over an existing VFS directory as a read-only snapshot;
- prevent writes, truncates, creates, unlinks, links, symlinks, and renames inside the read-only subtree from silently becoming tmpfs mutations.

The Linux-like design intent is to avoid the dangerous halfway state where
`mount("ext4")` succeeds but `open/read/getdents/stat` still operate on tmpfs.

Still incomplete after M15-A:

- persistent writeback;
- journal replay / JBD2;
- orphan list handling;
- htree indexed directories;
- xattrs/ACLs;
- full extent-tree and sparse-file stress coverage;
- real-device DMA/cache-coherency hardening.
