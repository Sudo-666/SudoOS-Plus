pub mod final_2026;
pub mod preliminary;

use crate::irq_lock::IrqSpinLock;
use crate::lockdep::{LockClass, LockRank};

const MODE_LOCK: LockClass = LockClass::new("oscomp.mode", LockRank::Vfs, 93);

/// 当前运行的用例模式（`final-cagent`/`final-buildstorm`/`preliminary`），
/// 看门狗超时用它打印 `CONTEST_RESULT ... timeout`。
static ACTIVE_MODE: IrqSpinLock<Option<&'static str>> =
    IrqSpinLock::new_with_class(None, MODE_LOCK);

/// 用例结果裁决。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContestVerdict {
    Passed,
    Failed,
    TimedOut,
}

/// 记录当前运行的用例模式。
pub fn set_active_mode(mode: &'static str) {
    *ACTIVE_MODE.lock() = Some(mode);
}

/// 返回当前运行的用例模式（未记录时为 `unknown`）。
pub fn active_mode() -> &'static str {
    ACTIVE_MODE.lock().unwrap_or("unknown")
}

/// 打印 `CONTEST_RESULT mode=<mode> <pass|fail|timeout>`。这是最终结果协议：
/// 只有镜像脚本退出码为 0 才打印 `pass`；日志检查器按模式匹配这些行判定
/// 用例成败（K2.1）。
pub fn report_contest_result(mode: &str, verdict: ContestVerdict) {
    let word = match verdict {
        ContestVerdict::Passed => "pass",
        ContestVerdict::Failed => "fail",
        ContestVerdict::TimedOut => "timeout",
    };
    crate::println!("CONTEST_RESULT mode={mode} {word}");
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    Preliminary,
    FinalCagent,
    FinalBuildstorm,
    FinalBuildstormDiag,
    LifecycleStress,
    FinalAll,
}

impl RunMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Preliminary => "preliminary",
            Self::FinalCagent => "final-cagent",
            Self::FinalBuildstorm => "final-buildstorm",
            Self::FinalBuildstormDiag => "final-buildstorm-diag",
            Self::LifecycleStress => "lifecycle-stress",
            Self::FinalAll => "final-all",
        }
    }
}

fn parse_mode(value: &str) -> Option<RunMode> {
    match value {
        "preliminary" => Some(RunMode::Preliminary),
        "final-cagent" => Some(RunMode::FinalCagent),
        "final-buildstorm" => Some(RunMode::FinalBuildstorm),
        "final-buildstorm-diag" => Some(RunMode::FinalBuildstormDiag),
        "lifecycle-stress" => Some(RunMode::LifecycleStress),
        "final-all" => Some(RunMode::FinalAll),
        _ => None,
    }
}

pub fn mode_from_bootargs(bootargs: Option<&str>) -> Option<RunMode> {
    let args = bootargs?;
    for word in args.split_whitespace() {
        if let Some(value) = word
            .strip_prefix("sudoos.oscomp=")
            .or_else(|| word.strip_prefix("oscomp.mode="))
        {
            return parse_mode(value);
        }
    }
    None
}

pub fn select_mode(explicit: Option<RunMode>) -> RunMode {
    if let Some(mode) = explicit {
        crate::println!(
            "sudoos-diag: oscomp mode selected: {} (bootargs)",
            mode.name()
        );
        return mode;
    }

    if final_2026::looks_like_final_image() {
        crate::println!("sudoos-diag: oscomp mode selected: final-all (final image discovery)");
        return RunMode::FinalAll;
    }

    // CLOUD_FINAL_MODE_FALLBACK_V1
    if crate::storage::contest_storage_mounted() {
        crate::println!(
            "sudoos-diag: oscomp mode selected: final-all (contest storage fallback)"
        );
        return RunMode::FinalAll;
    }
    crate::println!("sudoos-diag: oscomp mode selected: preliminary (no contest disk)");
    RunMode::Preliminary
}

pub fn run(mode: RunMode) -> bool {
    match mode {
        RunMode::Preliminary => preliminary::run(),
        RunMode::FinalCagent => final_2026::run_cagent(),
        RunMode::FinalBuildstorm => final_2026::run_buildstorm(),
        RunMode::FinalBuildstormDiag => final_2026::run_buildstorm_diag(),
        RunMode::LifecycleStress => final_2026::run_lifecycle_stress(),
        RunMode::FinalAll => final_2026::run_all(),
    }
}
