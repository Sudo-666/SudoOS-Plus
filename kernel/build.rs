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

/// 交叉编译 LS2K1000 USB C 源（CherryUSB 裁剪 + 平台胶水）为静态库并链接。
///
/// 这是纯 Rust 内核里第一条 C 构建路径（M0，见 docs/decisions/ADR-001）。
/// C 侧使用与 Rust 目标 `loongarch64-unknown-none-softfloat` 一致的
/// `lp64s` ABI，全部 freestanding，不依赖 libc。loongarch64 交叉工具链
/// 可用 `LS2K1000_CC` / `LS2K1000_AR` 覆盖，默认取 PATH 中的
/// `loongarch64-linux-gnu-gcc` / `loongarch64-linux-gnu-ar`。
/// 查询交叉 gcc 自带的 freestanding 头目录（stdint/stddef/stdarg/stdbool 等）。
/// `-nostdinc` 后需要显式加回这些目录，才能同时避开交叉 libc 头。
fn gcc_freestanding_includes(cc: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for kind in ["include", "include-fixed"] {
        let output = std::process::Command::new(cc)
            .arg(format!("-print-file-name={kind}"))
            .output()
            .expect("failed to query gcc freestanding include dir");
        let dir = String::from_utf8(output.stdout)
            .expect("gcc -print-file-name output is not UTF-8")
            .trim()
            .to_owned();
        if !dir.is_empty() && dir != kind {
            dirs.push(PathBuf::from(dir));
        }
    }
    dirs
}

fn compile_ls2k1000_glue(project_root: &Path) {
    let cc = env::var("LS2K1000_CC").unwrap_or_else(|_| "loongarch64-linux-gnu-gcc".to_owned());
    let ar = env::var("LS2K1000_AR").unwrap_or_else(|_| "loongarch64-linux-gnu-ar".to_owned());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));

    // 我们自己的平台胶水 + 固定版本的 CherryUSB 裁剪源码（只编译需要子集）。
    let glue_dir = project_root.join("kernel/csrc/usb");
    const VENDORED_CHERRYUSB: &[&str] = &[
        "vendor/cherryusb/core/usbh_core.c",
        "vendor/cherryusb/osal/usb_workq.c",
        "vendor/cherryusb/class/hub/usbh_hub.c",
        "vendor/cherryusb/class/msc/usbh_msc.c",
        "vendor/cherryusb/port/ehci/usb_ehci.c",
    ];
    // 首个目录是我们的 usb_config.h，优先于 vendor 根模板。
    const CHERRYUSB_INCLUDES: &[&str] = &[
        "kernel/csrc/usb",
        "vendor/cherryusb",
        "vendor/cherryusb/core",
        "vendor/cherryusb/common",
        "vendor/cherryusb/osal",
        "vendor/cherryusb/class/hub",
        "vendor/cherryusb/class/msc",
        "vendor/cherryusb/class/cdc",
        "vendor/cherryusb/class/hid",
        "vendor/cherryusb/port/ehci",
    ];

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&glue_dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", glue_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "c"))
        .collect();
    sources.extend(VENDORED_CHERRYUSB.iter().map(|path| project_root.join(path)));
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
        let is_vendored = source.starts_with(project_root.join("vendor/cherryusb"));
        let mut command = std::process::Command::new(&cc);
        command.args([
            "-mabi=lp64s",
            "-march=loongarch64",
            "-ffreestanding",
            "-nostdlib",
            "-nostdinc",
            "-fno-builtin",
            "-fno-stack-protector",
            "-fno-pic",
            "-fno-pie",
            "-ffunction-sections",
            "-fdata-sections",
            "-fno-asynchronous-unwind-tables",
            "-O2",
            // CherryUSB 日志走 libc printf：全部关掉（usb_config.h 在
            // usb_util.h 之后被包含，无法覆盖，故用编译期宏）。
            // bring-up 临时开 2（error/warning/info）：端口复位/枚举掉线诊断，
            // 稳定后须降回 -1 或 0。
            "-DUSB_DBG_LEVEL=2",
        ]);
        // -nostdinc 后显式补回：我们的 freestanding shim 头最优先，然后是
        // GCC 自有头（stdint/stddef/stdarg/stdbool 等），再是 CherryUSB 目录。
        command.arg("-I").arg(project_root.join("kernel/csrc/usb/include"));
        for dir in gcc_freestanding_includes(&cc) {
            command.arg("-I").arg(dir);
        }
        for include in CHERRYUSB_INCLUDES {
            command.arg("-I").arg(project_root.join(include));
        }
        // vendor 代码用 -w 压制（非我们维护），胶水保持 -Wall。
        command.arg(if is_vendored { "-w" } else { "-Wall" });
        command.arg("-c").arg(&source).arg("-o").arg(&object);
        let status = command.status().unwrap_or_else(|error| {
            panic!("failed to run loongarch64 C compiler ({cc}): {error}")
        });
        assert!(
            status.success(),
            "C compile failed for {}",
            source.display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        objects.push(object);
    }
    for include in CHERRYUSB_INCLUDES {
        println!("cargo:rerun-if-changed={}", project_root.join(include).display());
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
