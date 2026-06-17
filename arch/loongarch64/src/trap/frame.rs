pub const TRAP_FRAME_GUARD: usize = 0x5a5;

#[repr(C, align(16))]
pub struct TrapFrame {
    pub gpr: [usize; 32],

    pub prmd: usize,
    pub era: usize,
    pub estat: usize,
    pub badv: usize,
    pub badi: usize,

    guard: usize,
}

impl TrapFrame {
    pub const fn stack_pointer(&self) -> usize {
        self.gpr[3]
    }

    pub const fn return_address(&self) -> usize {
        self.gpr[1]
    }

    pub const fn exception_code(&self) -> usize {
        (self.estat >> 16) & 0x3f
    }

    pub const fn exception_subcode(&self) -> usize {
        (self.estat >> 22) & 0x1ff
    }

    pub const fn pending_interrupts(&self) -> usize {
        self.estat & 0x1fff
    }

    pub const fn previous_mode_was_user(&self) -> bool {
        const PRMD_PPLV_MASK: usize = 0b11;
        const PLV_USER: usize = 3;

        self.prmd & PRMD_PPLV_MASK == PLV_USER
    }

    pub const fn guard_is_valid(&self) -> bool {
        self.guard == TRAP_FRAME_GUARD
    }

    pub fn advance_pc(&mut self, bytes: usize) {
        self.era = self
            .era
            .checked_add(bytes)
            .expect("exception return PC overflow");
    }

    // M12: Signal frame helpers (LoongArch ABI).

    /// User stack pointer (r3/sp).
    pub fn user_stack_pointer(&self) -> usize {
        self.gpr[3]
    }

    /// Set user stack pointer (r3/sp).
    pub fn set_user_stack_pointer(&mut self, sp: usize) {
        self.gpr[3] = sp;
    }

    /// Set return address (r1/ra).
    pub fn set_return_address(&mut self, ra: usize) {
        self.gpr[1] = ra;
    }

    /// Set argument register (a0=idx 0 → r4, a1=idx 1 → r5, etc.).
    pub fn set_argument_register(&mut self, index: usize, value: usize) {
        self.gpr[4 + index] = value;
    }

    /// Set program counter (era).
    pub fn set_program_counter(&mut self, pc: usize) {
        self.era = pc;
    }
}

const _: () = {
    assert!(core::mem::size_of::<TrapFrame>() == 304);
    assert!(core::mem::align_of::<TrapFrame>() == 16);
};
