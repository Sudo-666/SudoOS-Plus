fn path_exists(path: &str) -> bool {
    crate::fs::stat(path).is_ok()
}

pub fn looks_like_final_image() -> bool {
    path_exists("/mnt/sdcard/glibc/cagent_testcode.sh")
        || path_exists("/mnt/sdcard/musl/cagent_testcode.sh")
        || path_exists("/mnt/sdcard/glibc/buildstorm_testcode.sh")
        || path_exists("/mnt/sdcard/musl/buildstorm_testcode.sh")
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

pub fn run_cow_stress() -> bool {
    crate::println!("sudoos-diag: entering COW stress runner");
    crate::user::verify_cow_stress()
}

// CLOUD_FINAL_IMAGE_CONTRACT_V1
fn report_final_image_contract() {
    let paths = [
        "/mnt/sdcard/glibc/cagent_testcode.sh",
        "/mnt/sdcard/musl/cagent_testcode.sh",
        "/mnt/sdcard/glibc/buildstorm_testcode.sh",
        "/mnt/sdcard/musl/buildstorm_testcode.sh",
        "/mnt/sdcard/work/tgoskits/Cargo.toml",
    ];

    crate::println!("final-image-contract:");
    for path in paths {
        crate::println!(
            "  {} = {}",
            path,
            if path_exists(path) { "present" } else { "missing" },
        );
    }

    let scripts = crate::SCANNED_TEST_SCRIPTS.lock();
    crate::println!("  scanned-test-scripts = {}", scripts.len());
    for script in scripts.iter().take(12) {
        crate::println!("  scanned-script = {}", script);
    }
}

pub fn run_all() -> bool {
    // FINAL_PLATFORM_ALL_SCORING_POINTS_V1
    //
    // The platform boots the submitted kernel without our local Makefile's
    // `sudoos.oscomp=...` argument.  Run CAgent first so its short judge can
    // finish exactly as before, then continue with the real BuildStorm script
    // for a BuildStorm-scoring invocation of the same kernel.  The explicit
    // modes remain available for isolated local regression runs.
    crate::println!("sudoos-diag: entering final-2026 all scoring runners");
    report_final_image_contract();
    let cagent_ran = run_cagent();
    let buildstorm_ran = run_buildstorm();
    cagent_ran || buildstorm_ran
}
