//! Host-side self-check for the kernel's ALE decode table.
//!
//! `include!` pulls in the exact file the kernel compiles under
//! `feature = "platform-ls2k1000"` (`kernel/src/user/ale_decode.rs`), so the
//! table exercised here is byte-for-byte the table the LS2K1000 emulator uses.
//!
//! Compile and run with the host toolchain (no kernel deps):
//!
//!   rustc --edition 2024 scripts/ale_decode_check.rs \
//!       -o build/host-tools/ale_decode_check.exe
//!   build/host-tools/ale_decode_check.exe

include!("../kernel/src/user/ale_decode.rs");

fn main() {
    // ---- The exact instruction that faulted on the board (Gate A round 1):
    //      era=0x1200dfca0 badv=0x12021538b  =>  ldptr.w r13, r6, 0
    let board = decode_ale_insn(0x2400_00cd).expect("board ldptr.w must decode");
    assert_eq!(board.is_load, true);
    assert_eq!(board.size, 4);
    assert_eq!(board.sign_extend, true);
    assert_eq!(board.rd, 13);

    // ---- LDPTR/STPTR group (8-bit opcode, bits[31:24]).
    // ldptr.w r5, r1, si14=1  (offsets are multiple-of-4; decoding is what we
    // test here — address computation comes from badv in the emulator).
    let d = decode_ale_insn(0x2400_0000 | (1 << 10) | (1 << 5) | 5).expect("ldptr.w");
    assert_eq!((d.is_load, d.size, d.sign_extend, d.rd), (true, 4, true, 5));
    let d = decode_ale_insn(0x2500_0000 | (0 << 10) | (3 << 5) | 2).expect("stptr.w");
    assert_eq!((d.is_load, d.size, d.sign_extend, d.rd), (false, 4, false, 2));
    let d = decode_ale_insn(0x2600_0000 | (7 << 10) | (4 << 5) | 9).expect("ldptr.d");
    assert_eq!((d.is_load, d.size, d.sign_extend, d.rd), (true, 8, false, 9));
    let d = decode_ale_insn(0x2700_0000 | (0x3fff << 10) | (6 << 5) | 31).expect("stptr.d");
    assert_eq!((d.is_load, d.size, d.sign_extend, d.rd), (false, 8, false, 31));

    // ---- LD/ST si12 group (10-bit opcode, bits[31:22]).
    let cases: &[(u32, bool, usize, bool)] = &[
        (0x2800_0000, true, 1, true),   // ld.b
        (0x2840_0000, true, 2, true),   // ld.h
        (0x2880_0000, true, 4, true),   // ld.w
        (0x28c0_0000, true, 8, false),  // ld.d
        (0x2900_0000, false, 1, false), // st.b
        (0x2940_0000, false, 2, false), // st.h
        (0x2980_0000, false, 4, false), // st.w
        (0x29c0_0000, false, 8, false), // st.d
        (0x2a00_0000, true, 1, false),  // ld.bu
        (0x2a40_0000, true, 2, false),  // ld.hu
        (0x2a80_0000, true, 4, false),  // ld.wu
    ];
    for (op, is_load, size, sign_extend) in cases {
        let insn = op | (7 << 5) | 21; // rj=7, rd=21
        let d = decode_ale_insn(insn).unwrap_or_else(|| panic!("opcode {op:#010x} must decode"));
        assert_eq!(d.is_load, *is_load, "opcode {op:#010x}");
        assert_eq!(d.size, *size, "opcode {op:#010x}");
        assert_eq!(d.sign_extend, *sign_extend, "opcode {op:#010x}");
        assert_eq!(d.rd, 21, "opcode {op:#010x}");
    }

    // ---- rd is bits[4:0], identical for both groups.
    assert_eq!(decode_ale_insn(0x2400_0001).unwrap().rd, 1);
    assert_eq!(decode_ale_insn(0x2880_001f).unwrap().rd, 31);

    // ---- Unsupported opcodes must be rejected, not mis-decoded.
    assert!(decode_ale_insn(0x3800_0000).is_none(), "FP opcode must be unsupported");
    assert!(decode_ale_insn(0x2c00_0000).is_none(), "LDX/STX must be unsupported");
    assert!(decode_ale_insn(0x0280_0000).is_none(), "non-load/store must be unsupported");
    assert!(decode_ale_insn(0x0000_0000).is_none(), "opcode 0 must be unsupported");
    assert!(decode_ale_insn(0x2880_0000).is_some(), "ld.w si12=0 must decode");

    // ---- Load value sign/zero extension.
    assert_eq!(load_value_from_bytes(1, true, &[0x80]), 0xffff_ffff_ffff_ff80);
    assert_eq!(load_value_from_bytes(1, false, &[0x80]), 0x80);
    assert_eq!(load_value_from_bytes(2, true, &[0x00, 0x80]), 0xffff_ffff_ffff_8000);
    assert_eq!(load_value_from_bytes(2, false, &[0x00, 0x80]), 0x8000);
    assert_eq!(
        load_value_from_bytes(4, true, &[0, 0, 0, 0x80]),
        0xffff_ffff_8000_0000
    );
    assert_eq!(load_value_from_bytes(4, false, &[0, 0, 0, 0x80]), 0x8000_0000);
    assert_eq!(
        load_value_from_bytes(8, false, &[1, 2, 3, 4, 5, 6, 7, 8]),
        0x0807_0605_0403_0201
    );

    println!("ale_decode_check: PASS (4 ldptr + {} si12 + 5 negative + 7 value asserts)", cases.len());
}
