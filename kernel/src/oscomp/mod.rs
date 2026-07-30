pub mod final_2026;
pub mod preliminary;

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
    if crate::block::open_device("vda").is_some() {
        crate::println!(
            "sudoos-diag: oscomp mode selected: final-all (contest block fallback)"
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
