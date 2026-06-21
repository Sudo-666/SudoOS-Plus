use std::{
    env,
    path::{Path, PathBuf},
};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));

    let project_root = manifest_dir
        .parent()
        .expect("kernel crate must be inside the project root");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH is not set");

    let linker_script = match target_arch.as_str() {
        "riscv64" => project_root.join("arch/riscv64/linker.ld"),

        "loongarch64" => project_root.join("arch/loongarch64/linker.ld"),

        unsupported => {
            panic!("unsupported target architecture: {unsupported}");
        }
    };

    require_file(&linker_script);

    println!("cargo:rerun-if-changed={}", linker_script.display());

    println!(
        "cargo:rustc-link-arg-bin=myos-kernel=-T{}",
        linker_script.display()
    );

    println!("cargo:rustc-link-arg-bin=myos-kernel=--gc-sections");
}

fn require_file(path: &Path) {
    assert!(
        path.is_file(),
        "required linker script does not exist: {}",
        path.display()
    );
}
