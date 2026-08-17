//! Linux 风格的 /proc 文件系统。
//!
//! 每个 proc 文件是一个实现 `ProcFileGenerator` trait 的对象，
//! 在 read 时动态生成内容。目录条目在挂载时预先填充。
//!
//! 动态 PID 目录:procfs 无法在持有 Vfs 锁时访问进程注册表(lockdep rank:
//! Vfs 36 > Process 35),因此 `/proc` 的 `readdir`/`lookup` 在进入 Vfs 临界区
//! 之前先通过 [`live_process_metas`] 采集一份只读快照,再把每 PID 的
//! `stat/status/cmdline/comm` 生成器绑定到该快照。生成器自身不访问进程
//! 注册表,只在 read 时格式化快照字节。

use alloc::{
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
    vec::Vec,
};

use myos_vfs::Errno;

/// Proc 文件内容生成器。
///
/// 每次 read 时调用 `generate()` 动态生成完整文件内容。
/// 返回的字节被缓存在文件位置中(简化实现)。
pub trait ProcFileGenerator: Send + Sync + 'static {
    /// 生成文件内容。
    fn generate(&self) -> Result<Vec<u8>, Errno>;
}

// ---------------------------------------------------------------------------
// 进程元数据快照
// ---------------------------------------------------------------------------

/// `/proc/<pid>` 目录内容的一份只读快照。
///
/// 在**不持有 Vfs 锁**时通过 [`live_process_metas`] 采集(lockdep rank 允许
/// Process 35 < Vfs 36 的顺序),随后绑定到各 ProcFileGenerator。生成器只读
/// 此快照,绝不再访问 PROCESS_REGISTRY。
#[derive(Clone, Debug)]
pub struct ProcMeta {
    pub pid: usize,
    pub ppid: Option<usize>,
    pub pgrp: isize,
    pub session: isize,
    pub uid: u32,
    pub gid: u32,
    /// Linux 状态字母:R 运行、S 睡眠、Z 僵尸。
    pub state: u8,
    pub thread_count: usize,
    /// `/proc/<pid>/comm` 内容(argv[0] basename, ≤ TASK_COMM_LEN-1)。
    pub comm: Vec<u8>,
    /// `/proc/<pid>/cmdline` 内容(argv NUL 分隔)。
    pub cmdline: Vec<u8>,
}

/// 采集当前存活进程的只读快照。
///
/// 只能在**未持有 Vfs 锁**时调用:内部经 `for_each_process` 短暂持有
/// PROCESS_REGISTRY(rank Process),返回后由调用方在 Vfs 临界区外把结果
/// 挂到 `/proc` 的 children 上。
pub fn live_process_metas() -> Vec<Arc<ProcMeta>> {
    let mut metas = Vec::new();
    let current_pid = crate::task::current_user_thread()
        .map(|t| t.process().id().get())
        .unwrap_or(0);
    crate::process::for_each_process(|process| {
        let pid = process.id().get();
        let state = if process.thread_count() == 0 {
            b'Z'
        } else if pid == current_pid {
            b'R'
        } else {
            b'S'
        };
        let credentials = process.credentials();
        metas.push(Arc::new(ProcMeta {
            pid,
            ppid: process.parent_id().map(|id| id.get()),
            pgrp: process.process_group(),
            session: process.session(),
            uid: credentials.real_uid(),
            gid: credentials.real_gid(),
            state,
            thread_count: process.thread_count(),
            comm: process.comm(),
            cmdline: process.cmdline(),
        }));
    });
    metas
}

// ---------------------------------------------------------------------------
// 具体 proc 文件
// ---------------------------------------------------------------------------

/// /proc/version — 内核版本字符串
struct VersionFile;

impl ProcFileGenerator for VersionFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(b"SudoOS 0.16 (M16)\n".to_vec())
    }
}

/// /proc/cpuinfo — CPU 信息
struct CpuInfoFile;

impl ProcFileGenerator for CpuInfoFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let cpu_count = crate::smp::discovered_cpu_count();
        let arch = crate::arch::ARCH_NAME;
        let mut output = String::new();
        output.push_str("processor\t: 0\n");
        output.push_str(&format!("cpu architecture\t: {arch}\n"));
        output.push_str(&format!("cpu count\t: {cpu_count}\n"));
        output.push_str("BogoMIPS\t: 100.00\n");
        // 为每个 CPU 输出信息
        for cpu in 0..cpu_count {
            output.push_str(&format!("\nprocessor\t: {cpu}\n"));
            output.push_str(&format!("hart\t: {cpu}\n"));
        }
        output.push('\n');
        Ok(output.into_bytes())
    }
}

/// /proc/meminfo — 内存统计
struct MemInfoFile;

impl ProcFileGenerator for MemInfoFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let page_size_kb = myos_mm::PAGE_SIZE / 1024;
        let free_pages = crate::page_alloc::total_free_pages().unwrap_or(0);
        let free = free_pages * page_size_kb;
        // 估算总内存 — 基于 free + allocated 的粗略估计
        let total = free + 16384; // 假设至少 16 MiB 的已分配内存
        let mut output = String::new();
        output.push_str(&format!("MemTotal:       {total:>8} kB\n"));
        output.push_str(&format!("MemFree:        {free:>8} kB\n"));
        output.push_str(&format!("MemAvailable:   {free:>8} kB\n"));
        output.push_str(&format!("PageSize:       {page_size_kb:>8} kB\n"));
        Ok(output.into_bytes())
    }
}

/// /proc/uptime — 系统运行时间
struct UptimeFile;

impl ProcFileGenerator for UptimeFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let now = crate::time::now().cycles();
        let freq = crate::time::clock_frequency_hz();
        let seconds = if freq != 0 { now / freq } else { 0 };
        let output = format!("{seconds}.00 0.00\n");
        Ok(output.into_bytes())
    }
}

/// /proc/mounts — 挂载表
struct MountsFile;

impl ProcFileGenerator for MountsFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        crate::fs::format_mounts()
    }
}

// ---------------------------------------------------------------------------
// /proc/<pid>/ 每进程文件
// ---------------------------------------------------------------------------

/// /proc/<pid>/comm — 进程名
struct PidCommFile(Arc<ProcMeta>);

impl ProcFileGenerator for PidCommFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let mut output = self.0.comm.clone();
        output.push(b'\n');
        Ok(output)
    }
}

/// /proc/<pid>/cmdline — argv, NUL 分隔
struct PidCmdlineFile(Arc<ProcMeta>);

impl ProcFileGenerator for PidCmdlineFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        Ok(self.0.cmdline.clone())
    }
}

/// /proc/<pid>/status — 单行关键字段的简化 status
struct PidStatusFile(Arc<ProcMeta>);

impl ProcFileGenerator for PidStatusFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let meta = &self.0;
        let state = meta.state as char;
        let ppid = meta.ppid.unwrap_or(0);
        let output = format!(
            "Name:\t{}\nState:\t{}\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nUid:\t{}\t{}\t{}\t{}\nGid:\t{}\t{}\t{}\t{}\nThreads:\t{}\n",
            String::from_utf8_lossy(&meta.comm),
            state,
            meta.pid,
            meta.pid,
            ppid,
            meta.uid,
            meta.uid,
            meta.uid,
            meta.uid,
            meta.gid,
            meta.gid,
            meta.gid,
            meta.gid,
            meta.thread_count,
        );
        Ok(output.into_bytes())
    }
}

/// /proc/<pid>/stat — Linux 兼容的 52 字段 stat 行。
///
/// 关键字段对齐内核行为:comm 带括号、state 字母、ppid/pgrp/session。
/// 未跟踪的统计字段(tty/flags/time/rss…)填 0;BusyBox ps 主要解析
/// 1-3(pid/comm/state)与 4-6(ppid/pgrp/session)。
struct PidStatFile(Arc<ProcMeta>);

impl ProcFileGenerator for PidStatFile {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let meta = &self.0;
        let comm = String::from_utf8_lossy(&meta.comm);
        let state = meta.state as char;
        let ppid = meta.ppid.unwrap_or(0);
        let fields = [
            meta.pid.to_string(),          // 1  pid
            format!("({comm})"),           // 2  comm(带括号)
            state.to_string(),             // 3  state
            ppid.to_string(),              // 4  ppid
            meta.pgrp.to_string(),         // 5  pgrp
            meta.session.to_string(),      // 6  session
            "0".to_string(),               // 7  tty_nr
            "0".to_string(),               // 8  tpgid
            "0".to_string(),               // 9  flags
            "0".to_string(),               // 10 minflt
            "0".to_string(),               // 11 cminflt
            "0".to_string(),               // 12 majflt
            "0".to_string(),               // 13 cmajflt
            "0".to_string(),               // 14 utime
            "0".to_string(),               // 15 stime
            "0".to_string(),               // 16 cutime
            "0".to_string(),               // 17 cstime
            "0".to_string(),               // 18 priority
            "0".to_string(),               // 19 nice
            meta.thread_count.to_string(), // 20 num_threads
            "0".to_string(),               // 21 itrealvalue
            "0".to_string(),               // 22 starttime
            "0".to_string(),               // 23 vsize
            "0".to_string(),               // 24 rss
            "0".to_string(),               // 25 rsslim
            "0".to_string(),               // 26 startcode
            "0".to_string(),               // 27 endcode
            "0".to_string(),               // 28 startstack
            "0".to_string(),               // 29 kstkesp
            "0".to_string(),               // 30 kstkeip
            "0".to_string(),               // 31 signal
            "0".to_string(),               // 32 blocked
            "0".to_string(),               // 33 sigignore
            "0".to_string(),               // 34 sigcatch
            "0".to_string(),               // 35 wchan
            "0".to_string(),               // 36 nswap
            "0".to_string(),               // 37 cnswap
            "0".to_string(),               // 38 exit_signal
            "0".to_string(),               // 39 processor
            "0".to_string(),               // 40 rt_priority
            "0".to_string(),               // 41 policy
            "0".to_string(),               // 42 delayacct_blkio_ticks
            "0".to_string(),               // 43 guest_time
            "0".to_string(),               // 44 cguest_time
            "0".to_string(),               // 45 start_data
            "0".to_string(),               // 46 end_data
            "0".to_string(),               // 47 start_brk
            "0".to_string(),               // 48 arg_start
            "0".to_string(),               // 49 arg_end
            "0".to_string(),               // 50 env_start
            "0".to_string(),               // 51 env_end
            "0".to_string(),               // 52 exit_code
        ];
        let mut output = fields.join(" ");
        output.push('\n');
        Ok(output.into_bytes())
    }
}

/// 为 `/proc/<pid>` 目录提供四个文件生成器。
/// 调用方负责把生成器包成 `NodeState::ProcFile` 节点并作为目录 children。
pub fn pid_dir_entries(meta: Arc<ProcMeta>) -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("comm", Arc::new(PidCommFile(Arc::clone(&meta)))),
        ("cmdline", Arc::new(PidCmdlineFile(Arc::clone(&meta)))),
        ("status", Arc::new(PidStatusFile(Arc::clone(&meta)))),
        ("stat", Arc::new(PidStatFile(meta))),
    ]
}

// ---------------------------------------------------------------------------
// /proc 根目录构建
// ---------------------------------------------------------------------------

/// 构建 /proc 根目录的静态条目列表。
/// 返回 (name, generator) 列表,由调用者创建 VFS 节点并插入到目录中。
/// `/proc/self` 与各 `/proc/<pid>` 目录由 fs::populate_proc_root 动态维护。
pub fn root_entries() -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("version", Arc::new(VersionFile)),
        ("cpuinfo", Arc::new(CpuInfoFile)),
        ("meminfo", Arc::new(MemInfoFile)),
        ("uptime", Arc::new(UptimeFile)),
        ("mounts", Arc::new(MountsFile)),
    ]
}
