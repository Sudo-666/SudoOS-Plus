//! Linux 风格的 /proc 文件系统。
//!
//! 每个 proc 文件是一个实现 `ProcFileGenerator` trait 的对象，
//! 在 read 时动态生成内容。目录条目在挂载时预先填充。

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
/// 返回的字节被缓存在文件位置中（简化实现）。
pub trait ProcFileGenerator: Send + Sync + 'static {
    /// 生成文件内容。
    fn generate(&self) -> Result<Vec<u8>, Errno>;
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

/// /proc/self — 指向当前 PID 的符号链接目标
struct SelfSymlink;

impl ProcFileGenerator for SelfSymlink {
    fn generate(&self) -> Result<Vec<u8>, Errno> {
        let pid = crate::task::current_user_thread()
            .map(|t| t.process_arc().id().get())
            .unwrap_or(1);
        Ok(pid.to_string().into_bytes())
    }
}

// ---------------------------------------------------------------------------
// /proc 根目录构建
// ---------------------------------------------------------------------------

/// 构建 /proc 根目录的条目列表。
/// 返回 (name, generator) 列表，由调用者创建 VFS 节点并插入到目录中。
pub fn root_entries() -> Vec<(&'static str, Arc<dyn ProcFileGenerator>)> {
    vec![
        ("version", Arc::new(VersionFile)),
        ("cpuinfo", Arc::new(CpuInfoFile)),
        ("meminfo", Arc::new(MemInfoFile)),
        ("uptime", Arc::new(UptimeFile)),
        ("mounts", Arc::new(MountsFile)),
    ]
}
