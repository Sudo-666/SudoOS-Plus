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
            // 根据 Cargo 的 Feature 环境变量,动态选择对应的链接脚本。
            // 平台 feature 严格互斥(见 arch/riscv64/platform/mod.rs),这里不设
            // "VF2 优先"回退,避免在混编时静默选错链接脚本。
            if env::var("CARGO_FEATURE_PLATFORM_VISIONFIVE2").is_ok() {
                project_root.join("arch/riscv64/src/platform/visionfive2/linker.ld")
            } else if env::var("CARGO_FEATURE_PLATFORM_QEMU_VIRT").is_ok() {
                project_root.join("arch/riscv64/src/platform/qemu_virt/linker.ld")
            } else {
                panic!(
                    "For riscv64, a platform feature must be enabled (e.g., 'platform-qemu-virt')."
                );
            }
        }

        "loongarch64" => {
            // 根据 Cargo 的 Feature 环境变量，动态选择对应的链接脚本
            if env::var("CARGO_FEATURE_PLATFORM_LS2K1000").is_ok() {
                project_root.join("arch/loongarch64/src/platform/ls2k1000/linker.ld")
            } else if env::var("CARGO_FEATURE_PLATFORM_QEMU_VIRT").is_ok() {
                project_root.join("arch/loongarch64/src/platform/qemu_virt/linker.ld")
            } else {
                panic!(
                    "For loongarch64, a platform feature must be enabled (e.g., 'platform-ls2k1000')."
                );
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

    // LS2K1000 C 胶水（CherryUSB 宿主）：仅在 loongarch64 +
    // platform-ls2k1000 下交叉编译 kernel/csrc/usb/*.c 为静态库并链接。
    // 其余目标零 C 依赖。见 docs/decisions/ADR-001。
    if env::var("CARGO_FEATURE_PLATFORM_LS2K1000").is_ok() {
        compile_ls2k1000_glue(project_root);
    }
}

fn require_file(path: &Path) {
    assert!(
        path.is_file(),
        "required linker script does not exist: {}",
        path.display()
    );
}

/// 交叉编译 LS2K1000 USB 平台胶水 C 源为静态库并链接进内核。
///
/// 这是纯 Rust 内核里第一条 C 构建路径（M0，见 docs/decisions/ADR-001）。
/// C 侧使用与 Rust 目标 `loongarch64-unknown-none-softfloat` 一致的
/// `lp64s` ABI，全部 freestanding，不依赖 libc。loongarch64 交叉工具链
/// 可用 `LS2K1000_CC` / `LS2K1000_AR` 覆盖，默认取 PATH 中的
/// `loongarch64-linux-gnu-gcc` / `loongarch64-linux-gnu-ar`。
fn compile_ls2k1000_glue(project_root: &Path) {
    let cc = env::var("LS2K1000_CC").unwrap_or_else(|_| "loongarch64-linux-gnu-gcc".to_owned());
    let ar = env::var("LS2K1000_AR").unwrap_or_else(|_| "loongarch64-linux-gnu-ar".to_owned());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));

    let src_dir = project_root.join("kernel/csrc/usb");
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&src_dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", src_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    sources.sort();
    assert!(!sources.is_empty(), "no C sources under kernel/csrc/usb");

    let mut objects = Vec::new();
    for source in &sources {
        let object = out_dir.join(format!(
            "{}.o",
            source
                .file_stem()
                .expect("C source has a file name")
                .to_string_lossy()
        ));
        let status = std::process::Command::new(&cc)
            .args([
                "-mabi=lp64s",
                "-march=loongarch64",
                "-ffreestanding",
                "-nostdlib",
                "-fno-builtin",
                "-fno-stack-protector",
                "-fno-pic",
                "-fno-pie",
                "-ffunction-sections",
                "-fdata-sections",
                "-fno-asynchronous-unwind-tables",
                "-O2",
                "-Wall",
                "-c",
            ])
            .arg(source)
            .arg("-o")
            .arg(&object)
            .status()
            .unwrap_or_else(|error| panic!("failed to run loongarch64 C compiler ({cc}): {error}"));
        assert!(
            status.success(),
            "C compile failed for {}",
            source.display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        objects.push(object);
    }

    let archive = out_dir.join("libsudoos_usb.a");
    let status = std::process::Command::new(&ar)
        .arg("rcs")
        .arg(&archive)
        .args(&objects)
        .status()
        .unwrap_or_else(|error| panic!("failed to run loongarch64 ar ({ar}): {error}"));
    assert!(status.success(), "ar failed for {}", archive.display());

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=sudoos_usb");
}
