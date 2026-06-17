pub const TRAP_FRAME_GUARD: usize = 0x5a5;

#[repr(C, align(16))]
pub struct TrapFrame {
    pub gpr: [usize; 32],

    pub sstatus: usize,
    pub sepc: usize,
    pub scause: usize,
    pub stval: usize,

    guard: usize,
    _padding: usize,
}

impl TrapFrame {
    pub const fn stack_pointer(&self) -> usize {
        self.gpr[2]
    }

    pub const fn return_address(&self) -> usize {
        self.gpr[1]
    }

    pub const fn is_interrupt(&self) -> bool {
        self.scause >> (usize::BITS - 1) != 0
    }

    pub const fn cause_code(&self) -> usize {
        self.scause & (usize::MAX >> 1)
    }

    pub const fn previous_mode_was_user(&self) -> bool {
        const SSTATUS_SPP: usize = 1 << 8;

        self.sstatus & SSTATUS_SPP == 0
    }

    pub const fn guard_is_valid(&self) -> bool {
        self.guard == TRAP_FRAME_GUARD
    }

    pub fn advance_pc(&mut self, bytes: usize) {
        self.sepc = self
            .sepc
            .checked_add(bytes)
            .expect("trap return PC overflow");
    }

    // M12: Signal frame helpers (RISC-V ABI).

    /// User stack pointer (x2/sp).
    pub fn user_stack_pointer(&self) -> usize {
        self.gpr[2]
    }

    /// Set user stack pointer (x2/sp).
    pub fn set_user_stack_pointer(&mut self, sp: usize) {
        self.gpr[2] = sp;
    }

    /// Set return address (x1/ra).
    pub fn set_return_address(&mut self, ra: usize) {
        self.gpr[1] = ra;
    }

    /// Set argument register (a0=idx 0 → x10, a1=idx 1 → x11, etc.).
    pub fn set_argument_register(&mut self, index: usize, value: usize) {
        self.gpr[10 + index] = value;
    }

    /// Set program counter (sepc).
    pub fn set_program_counter(&mut self, pc: usize) {
        self.sepc = pc;
    }
}

const _: () = {
    assert!(core::mem::size_of::<TrapFrame>() == 304);
    assert!(core::mem::align_of::<TrapFrame>() == 16);
};
