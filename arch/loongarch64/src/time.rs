use core::arch::asm;

const CSR_ECFG: usize = 0x4;
const CSR_TCFG: usize = 0x41;
const CSR_TICLR: usize = 0x44;

const ECFG_TIMER_INTERRUPT: usize = 1 << 11;
const TCFG_ENABLE: usize = 1 << 0;
const TCFG_VALUE_MASK: u64 = !0b11;
const TICLR_CLEAR: usize = 1;

const CPUCFG_FEATURE_WORD: usize = 2;
const CPUCFG_CONSTANT_TIMER: usize = 1 << 14;
const CPUCFG_FREQUENCY_WORD: usize = 4;
const CPUCFG_SCALE_WORD: usize = 5;

const MINIMUM_TIMER_DELTA: u64 = 4;

unsafe extern "C" {
    fn __loongarch_cpucfg_word(index: usize) -> usize;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    ConstantTimerUnavailable,
    InvalidFrequencyParameters {
        base: u32,
        multiplier: u16,
        divisor: u16,
    },
    FrequencyOverflow,
}

pub fn frequency_hz(_firmware_frequency: Option<u64>) -> Result<u64, TimerError> {
    let features = cpucfg_word(CPUCFG_FEATURE_WORD);

    if features & CPUCFG_CONSTANT_TIMER == 0 {
        return Err(TimerError::ConstantTimerUnavailable);
    }

    let base = cpucfg_word(CPUCFG_FREQUENCY_WORD) as u32;
    let scale = cpucfg_word(CPUCFG_SCALE_WORD) as u32;
    let multiplier = (scale & 0xffff) as u16;
    let divisor = (scale >> 16) as u16;

    if base == 0 || multiplier == 0 || divisor == 0 {
        return Err(TimerError::InvalidFrequencyParameters {
            base,
            multiplier,
            divisor,
        });
    }

    let scaled = u64::from(base)
        .checked_mul(u64::from(multiplier))
        .ok_or(TimerError::FrequencyOverflow)?;
    let frequency = scaled / u64::from(divisor);

    if frequency == 0 {
        return Err(TimerError::InvalidFrequencyParameters {
            base,
            multiplier,
            divisor,
        });
    }

    Ok(frequency)
}

#[inline]
pub fn counter() -> u64 {
    let value: u64;

    // SAFETY: RDTIME.D only reads the stable counter.  The counter ID is
    // discarded by selecting the architectural zero register.
    unsafe {
        asm!(
            "rdtime.d {value}, $zero",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn program_deadline(deadline: u64) -> Result<(), TimerError> {
    let now = counter();
    let requested = deadline.saturating_sub(now).max(MINIMUM_TIMER_DELTA);
    let rounded = requested.saturating_add(3) & TCFG_VALUE_MASK;
    let timer_value = rounded.max(MINIMUM_TIMER_DELTA) as usize;

    // One-shot mode is selected by leaving TCFG.Periodic clear.  Reprogramming
    // on every interrupt proves that the interrupt path really rearms events.
    write_csr::<CSR_TCFG>(timer_value | TCFG_ENABLE);
    Ok(())
}

pub fn acknowledge() {
    write_csr::<CSR_TICLR>(TICLR_CLEAR);
}

pub fn shutdown() -> Result<(), TimerError> {
    disable_interrupt_source();
    write_csr::<CSR_TCFG>(0);
    acknowledge();
    Ok(())
}

pub fn enable_interrupt_source() {
    update_csr_bits::<CSR_ECFG>(ECFG_TIMER_INTERRUPT, ECFG_TIMER_INTERRUPT);
}

pub fn disable_interrupt_source() {
    update_csr_bits::<CSR_ECFG>(0, ECFG_TIMER_INTERRUPT);
}

pub fn interrupt_source_enabled() -> bool {
    read_csr::<CSR_ECFG>() & ECFG_TIMER_INTERRUPT != 0
}

fn cpucfg_word(index: usize) -> usize {
    // SAFETY: CPUCFG is a side-effect-free architecture query and the helper
    // follows the LoongArch C ABI.
    unsafe { __loongarch_cpucfg_word(index) }
}

fn read_csr<const CSR: usize>() -> usize {
    let value: usize;

    // SAFETY: callers instantiate this helper only with readable CSRs.
    unsafe {
        asm!(
            "csrrd {value}, {csr}",
            value = out(reg) value,
            csr = const CSR,
            options(nomem, nostack),
        );
    }

    value
}

fn write_csr<const CSR: usize>(value: usize) {
    let scratch = value;

    // SAFETY: callers instantiate this helper only with writable timer CSRs.
    unsafe {
        asm!(
            "csrwr {scratch}, {csr}",
            scratch = inout(reg) scratch => _,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
}

fn update_csr_bits<const CSR: usize>(value: usize, mask: usize) {
    let scratch = value;

    // SAFETY: CSRXCHG changes only the selected CSR bits.  r12 is reserved as
    // the mask operand for the duration of this instruction.
    unsafe {
        asm!(
            "csrxchg {scratch}, $r12, {csr}",
            scratch = inout(reg) scratch => _,
            in("$r12") mask,
            csr = const CSR,
            options(nomem, nostack),
        );
    }
}
