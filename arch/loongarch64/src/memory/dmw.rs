use core::arch::asm;

use myos_mm::VirtAddr;

pub const UNCACHED_DMW_VALUE: usize = 0x8000_0000_0000_0001;

pub const CACHED_DMW_VALUE: usize = 0x9000_0000_0000_0011;

const CRMD_DA: usize = 1 << 3;
const CRMD_PG: usize = 1 << 4;

pub fn current_mode() -> usize {
    let value: usize;

    // SAFETY:
    // 只读取当前模式 CSR。
    unsafe {
        asm!(
            "csrrd {value}, 0x0",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn dmw0() -> usize {
    let value: usize;

    // SAFETY: 只读取 DMW0。
    unsafe {
        asm!(
            "csrrd {value}, 0x180",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn dmw1() -> usize {
    let value: usize;

    // SAFETY: 只读取 DMW1。
    unsafe {
        asm!(
            "csrrd {value}, 0x181",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn dmw2() -> usize {
    let value: usize;

    // SAFETY: 只读取 DMW2。
    unsafe {
        asm!(
            "csrrd {value}, 0x182",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn dmw3() -> usize {
    let value: usize;

    // SAFETY: 只读取 DMW3。
    unsafe {
        asm!(
            "csrrd {value}, 0x183",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn current_pc() -> VirtAddr {
    let value: usize;

    // SAFETY:
    // pcaddi 只计算当前 PC，不访问内存。
    unsafe {
        asm!(
            "pcaddi {value}, 0",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    VirtAddr::new(value)
}

pub fn assert_configured() {
    let mode = current_mode();

    assert!(mode & CRMD_PG != 0, "LoongArch mapped mode is disabled",);

    assert!(
        mode & CRMD_DA == 0,
        "LoongArch direct-address mode is still enabled",
    );

    assert_eq!(dmw0(), UNCACHED_DMW_VALUE, "unexpected LoongArch DMW0",);

    assert_eq!(dmw1(), CACHED_DMW_VALUE, "unexpected LoongArch DMW1",);

    assert_eq!(
        dmw2(),
        0,
        "unused LoongArch DMW2 must be disabled",
    );

    assert_eq!(dmw3(), 0, "unused LoongArch DMW3 must be disabled",);
}
