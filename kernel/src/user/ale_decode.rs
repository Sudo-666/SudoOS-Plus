// Pure decoding for the LS2K1000 unaligned-access (ALE) fix-up path.
//
// This file is deliberately dependency-free (only `core`): the kernel
// includes it via `mod ale_decode` under `feature = "platform-ls2k1000"`,
// and the host-side self-check `scripts/ale_decode_check.rs` compiles the
// exact same file standalone with `include!`. That way the table the board
// emulator runs is byte-for-byte the table the tests exercise.

/// Why an ALE instruction could not be emulated.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AleEmulationError {
    /// The faulting instruction could not be fetched (ERA unmapped).
    InstructionFetch,
    /// The opcode is not an emulatable integer load/store (e.g. FP ld/st).
    UnsupportedOpcode(u32),
    /// The checked user read of `badv` failed (unmapped / no read permission).
    LoadFault(usize),
    /// The checked user write of `badv` failed (unmapped / no write permission).
    StoreFault(usize),
}

impl core::fmt::Debug for AleEmulationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AleEmulationError::InstructionFetch => f.write_str("InstructionFetch"),
            AleEmulationError::UnsupportedOpcode(insn) => {
                write!(f, "UnsupportedOpcode({insn:#010x})")
            }
            AleEmulationError::LoadFault(addr) => write!(f, "LoadFault({addr:#x})"),
            AleEmulationError::StoreFault(addr) => write!(f, "StoreFault({addr:#x})"),
        }
    }
}

/// A decoded ALE-fixable integer load/store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedAleInsn {
    /// `true` for a load (read `badv`, write `rd`); `false` for a store.
    pub is_load: bool,
    /// Access width in bytes: 1, 2, 4 or 8.
    pub size: usize,
    /// Whether a load sign-extends into the destination register.
    pub sign_extend: bool,
    /// Destination register for loads, source register for stores.
    pub rd: usize,
}

/// Decode one LoongArch integer load/store instruction word.
///
/// Two opcode groups, per the LoongArch base ISA (Vol 1, "Basic Integer
/// Load/Store Instructions"):
///   * LDPTR/STPTR — 8-bit opcode in bits[31:24] (`ldptr.w` … `stptr.d`);
///   * LD/ST si12  — 10-bit opcode in bits[31:22] (`ld.b` … `ld.wu`).
///
/// Both groups place `rd` in bits[4:0] and `rj` in bits[9:5]. Returns `None`
/// for anything else (FP loads, LDX/STX register-indexed forms, non-load/store
/// instructions).
pub fn decode_ale_insn(insn: u32) -> Option<DecodedAleInsn> {
    let (is_load, size, sign_extend) = match insn & 0xff00_0000 {
        0x2400_0000 => Some((true, 4, true)),   // ldptr.w rd, rj, si14
        0x2500_0000 => Some((false, 4, false)), // stptr.w
        0x2600_0000 => Some((true, 8, false)),  // ldptr.d
        0x2700_0000 => Some((false, 8, false)), // stptr.d
        _ => None,
    }
    .or_else(|| match insn & 0xffc0_0000 {
        0x2800_0000 => Some((true, 1, true)),   // ld.b
        0x2840_0000 => Some((true, 2, true)),   // ld.h
        0x2880_0000 => Some((true, 4, true)),   // ld.w
        0x28c0_0000 => Some((true, 8, false)),  // ld.d
        0x2900_0000 => Some((false, 1, false)), // st.b
        0x2940_0000 => Some((false, 2, false)), // st.h
        0x2980_0000 => Some((false, 4, false)), // st.w
        0x29c0_0000 => Some((false, 8, false)), // st.d
        0x2a00_0000 => Some((true, 1, false)),  // ld.bu
        0x2a40_0000 => Some((true, 2, false)),  // ld.hu
        0x2a80_0000 => Some((true, 4, false)),  // ld.wu
        _ => None,
    })?;

    Some(DecodedAleInsn {
        is_load,
        size,
        sign_extend,
        rd: (insn & 0x1f) as usize,
    })
}

/// Assemble the value a load places into `rd`, honouring sign/zero extension.
///
/// `bytes` must contain at least `size` little-endian bytes.
pub fn load_value_from_bytes(size: usize, sign_extend: bool, bytes: &[u8]) -> u64 {
    match (size, sign_extend) {
        (1, true) => bytes[0] as i8 as u64,
        (1, false) => bytes[0] as u64,
        (2, true) => u16::from_le_bytes([bytes[0], bytes[1]]) as i16 as u64,
        (2, false) => u16::from_le_bytes([bytes[0], bytes[1]]) as u64,
        (4, true) => {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i32 as u64
        }
        (4, false) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64,
        (8, false) => u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]),
        _ => unreachable!("decode_ale_insn only yields sizes 1/2/4/8"),
    }
}
