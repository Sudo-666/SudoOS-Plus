use core::arch::asm;

const SIE_STIE: usize = 1 << 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    MissingTimebaseFrequency,
    InvalidTimebaseFrequency,
    TimeExtensionUnavailable,
    SbiCallFailed { code: isize },
}

impl From<crate::sbi::SbiError> for TimerError {
    fn from(error: crate::sbi::SbiError) -> Self {
        Self::SbiCallFailed { code: error.0 }
    }
}

pub fn frequency_hz(firmware_frequency: Option<u64>) -> Result<u64, TimerError> {
    if !crate::sbi::time_extension_available()? {
        return Err(TimerError::TimeExtensionUnavailable);
    }

    match firmware_frequency {
        Some(frequency) if frequency != 0 => Ok(frequency),
        Some(_) => Err(TimerError::InvalidTimebaseFrequency),
        None => Err(TimerError::MissingTimebaseFrequency),
    }
}

#[inline]
pub fn counter() -> u64 {
    let value: u64;

    // SAFETY: `rdtime` only reads the supervisor-visible monotonic counter.
    unsafe {
        asm!(
            "rdtime {value}",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value
}

pub fn program_deadline(deadline: u64) -> Result<(), TimerError> {
    crate::sbi::set_timer(deadline)?;
    Ok(())
}

pub const fn acknowledge() {
    // SBI set_timer() clears a pending supervisor timer interrupt when the new
    // deadline is in the future.  The common handler programs that deadline
    // immediately after this hook.
}

pub fn shutdown() -> Result<(), TimerError> {
    disable_interrupt_source();
    crate::sbi::set_timer(u64::MAX)?;
    Ok(())
}

pub fn enable_interrupt_source() {
    // SAFETY: only the current hart's supervisor timer interrupt mask changes.
    unsafe {
        asm!(
            "csrs sie, {mask}",
            mask = in(reg) SIE_STIE,
            options(nostack),
        );
    }
}

pub fn disable_interrupt_source() {
    // SAFETY: only the current hart's supervisor timer interrupt mask changes.
    unsafe {
        asm!(
            "csrc sie, {mask}",
            mask = in(reg) SIE_STIE,
            options(nostack),
        );
    }
}

pub fn interrupt_source_enabled() -> bool {
    let value: usize;

    // SAFETY: this only reads the current hart's `sie` CSR.
    unsafe {
        asm!(
            "csrr {value}, sie",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }

    value & SIE_STIE != 0
}
