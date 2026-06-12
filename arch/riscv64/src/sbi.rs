use core::arch::asm;

const BASE_EXTENSION_ID: usize = 0x10;
const BASE_PROBE_EXTENSION_FID: usize = 3;
const TIME_EXTENSION_ID: usize = 0x5449_4d45;
const TIME_SET_TIMER_FID: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SbiError(pub isize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SbiRet {
    error: isize,
    value: usize,
}

impl SbiRet {
    fn into_result(self) -> Result<usize, SbiError> {
        if self.error == 0 {
            Ok(self.value)
        } else {
            Err(SbiError(self.error))
        }
    }
}

pub fn time_extension_available() -> Result<bool, SbiError> {
    let value = call1(
        BASE_EXTENSION_ID,
        BASE_PROBE_EXTENSION_FID,
        TIME_EXTENSION_ID,
    )
    .into_result()?;

    Ok(value != 0)
}

pub fn set_timer(deadline: u64) -> Result<(), SbiError> {
    call1(TIME_EXTENSION_ID, TIME_SET_TIMER_FID, deadline as usize)
        .into_result()
        .map(|_| ())
}

fn call1(extension_id: usize, function_id: usize, argument0: usize) -> SbiRet {
    let error: usize;
    let value: usize;

    // SAFETY: this follows the SBI v0.2+ calling convention.  The ecall may
    // modify a0/a1 and firmware state, all of which are declared here.
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") argument0 => error,
            lateout("a1") value,
            in("a6") function_id,
            in("a7") extension_id,
            options(nostack),
        );
    }

    SbiRet {
        error: error as isize,
        value,
    }
}
