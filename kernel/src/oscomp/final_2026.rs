fn path_exists(path: &str) -> bool {
    crate::fs::stat(path).is_ok()
}

pub fn looks_like_final_image() -> bool {
    path_exists("/mnt/sdcard/glibc/cagent_testcode.sh")
        || path_exists("/mnt/sdcard/musl/cagent_testcode.sh")
        || path_exists("/mnt/sdcard/work/tgoskits/Cargo.toml")
}

pub fn run_cagent() -> bool {
    crate::println!("sudoos-diag: entering final-2026 CAgent runner");
    crate::user::verify_final_cagent()
}

pub fn run_buildstorm() -> bool {
    crate::println!("sudoos-diag: entering final-2026 BuildStorm runner");
    crate::user::verify_final_buildstorm()
}

pub fn run_buildstorm_diag() -> bool {
    crate::println!("sudoos-diag: entering final-2026 BuildStorm diagnostic runner");
    crate::user::verify_final_buildstorm_diag()
}

pub fn run_lifecycle_stress() -> bool {
    crate::println!("sudoos-diag: entering task lifecycle stress runner");
    crate::user::verify_task_lifecycle_stress()
}

pub fn run_all() -> bool {
    crate::println!("sudoos-diag: entering final-2026 all runner");
    let cagent_ran = run_cagent();
    let buildstorm_ran = run_buildstorm();
    cagent_ran || buildstorm_ran
}
