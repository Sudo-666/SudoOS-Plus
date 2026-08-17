//! 启动参数(bootargs)解析。
//!
//! bootargs 由 U-Boot 写入 `/chosen/bootargs`,以空格分隔的
//! `key=value` 对构成。本模块只解析内核自身的 `sudoos.*` 参数。

use core::fmt;

/// 从 bootargs 中解析 `sudoos.maxcpus=<n>`。
///
/// - 未给出该键:返回 `Ok(None)`,内核使用全部可用 CPU;
/// - 给出正整数:返回 `Ok(Some(n))`;
/// - 给出 `0` 或非数字:返回 `Err`,启动时应明确报错并停止。
pub fn max_cpus(bootargs: Option<&str>) -> Result<Option<usize>, MaxCpusError> {
    let Some(bootargs) = bootargs else {
        return Ok(None);
    };

    for token in bootargs.split_whitespace() {
        if let Some(value) = token.strip_prefix("sudoos.maxcpus=") {
            if value.is_empty() {
                return Err(MaxCpusError::Malformed);
            }

            let parsed = value
                .parse::<usize>()
                .map_err(|_| MaxCpusError::Malformed)?;
            if parsed == 0 {
                return Err(MaxCpusError::Zero);
            }

            return Ok(Some(parsed));
        }
    }

    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxCpusError {
    /// `sudoos.maxcpus=0`:内核无法在零个 CPU 上运行。
    Zero,
    /// `sudoos.maxcpus` 值非正整数。
    Malformed,
}

impl fmt::Display for MaxCpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => write!(
                f,
                "sudoos.maxcpus=0 is invalid: at least the boot CPU is required"
            ),
            Self::Malformed => {
                write!(f, "sudoos.maxcpus value must be a positive integer")
            }
        }
    }
}
