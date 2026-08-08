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
        "riscv64" => {
            // 根据 Cargo 的 Feature 环境变量,动态选择对应的链接脚本
            if env::var("CARGO_FEATURE_PLATFORM_VISIONFIVE2").is_ok() {
                project_root.join("arch/riscv64/src/platform/visionfive2/linker.ld")
            } else {
                project_root.join("arch/riscv64/src/platform/qemu_virt/linker.ld")
            }
        }

        "loongarch64" => {
            // 根据 Cargo 的 Feature 环境变量，动态选择对应的链接脚本
            if env::var("CARGO_FEATURE_PLATFORM_LS2K1000").is_ok() {
                project_root.join("arch/loongarch64/src/platform/ls2k1000/linker.ld")
            } else if env::var("CARGO_FEATURE_PLATFORM_QEMU_VIRT").is_ok() {
                project_root.join("arch/loongarch64/src/platform/qemu_virt/linker.ld")
            } else {
                panic!("For loongarch64, a platform feature must be enabled (e.g., 'platform-ls2k1000').");
            }
        }

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

    // Optional LoongArch vendor busybox for shell fallback.
    let vendor_la_busybox = project_root.join("vendor/userland/loongarch64/busybox");
    if target_arch == "loongarch64" && vendor_la_busybox.is_file() {
        println!("cargo:rustc-cfg=vendor_la_busybox");
        println!(
            "cargo:rustc-env=MYOS_VENDOR_LA_BUSYBOX={}",
            vendor_la_busybox.display()
        );
    }
    println!("cargo:rerun-if-changed={}", vendor_la_busybox.display());
}

fn require_file(path: &Path) {
    assert!(
        path.is_file(),
        "required linker script does not exist: {}",
        path.display()
    );
}