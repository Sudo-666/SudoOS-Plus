use core::arch::asm;

const BASE_EXTENSION_ID: usize = 0x10;
const BASE_PROBE_EXTENSION_FID: usize = 3;
const TIME_EXTENSION_ID: usize = 0x5449_4d45;
const TIME_SET_TIMER_FID: usize = 0;
const IPI_EXTENSION_ID: usize = 0x0073_5049;
const IPI_SEND_FID: usize = 0;
const HSM_EXTENSION_ID: usize = 0x0048_534d;
const HSM_HART_START_FID: usize = 0;

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
    probe_extension(TIME_EXTENSION_ID)
}

pub fn ipi_extension_available() -> Result<bool, SbiError> {
    probe_extension(IPI_EXTENSION_ID)
}

pub fn hsm_extension_available() -> Result<bool, SbiError> {
    probe_extension(HSM_EXTENSION_ID)
}

fn probe_extension(extension_id: usize) -> Result<bool, SbiError> {
    let value = call1(BASE_EXTENSION_ID, BASE_PROBE_EXTENSION_FID, extension_id).into_result()?;

    Ok(value != 0)
}

pub fn set_timer(deadline: u64) -> Result<(), SbiError> {
    call1(TIME_EXTENSION_ID, TIME_SET_TIMER_FID, deadline as usize)
        .into_result()
        .map(|_| ())
}

pub fn hart_start(hart_id: usize, start_address: usize, opaque: usize) -> Result<(), SbiError> {
    call3(
        HSM_EXTENSION_ID,
        HSM_HART_START_FID,
        hart_id,
        start_address,
        opaque,
    )
    .into_result()
    .map(|_| ())
}

pub fn send_ipi(hart_id: usize) -> Result<(), SbiError> {
    /*
     * A one-bit mask with hart_mask_base equal to the destination hart avoids
     * assuming that firmware hart IDs are small or densely numbered.
     */
    call2(IPI_EXTENSION_ID, IPI_SEND_FID, 1, hart_id)
        .into_result()
        .map(|_| ())
}

fn call1(extension_id: usize, function_id: usize, argument0: usize) -> SbiRet {
    call3(extension_id, function_id, argument0, 0, 0)
}

fn call2(extension_id: usize, function_id: usize, argument0: usize, argument1: usize) -> SbiRet {
    call3(extension_id, function_id, argument0, argument1, 0)
}

fn call3(
    extension_id: usize,
    function_id: usize,
    argument0: usize,
    argument1: usize,
    argument2: usize,
) -> SbiRet {
    let error: usize;
    let value: usize;

    // SAFETY: this follows the SBI v0.2+ calling convention. The ecall may
    // modify a0/a1 and firmware state, all of which are declared here.
    unsafe {
        asm!(
            "ecall",
            inlateout("a0") argument0 => error,
            inlateout("a1") argument1 => value,
            in("a2") argument2,
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
