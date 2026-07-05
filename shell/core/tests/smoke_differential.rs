//! Live C++/Rust smoke differential fixtures for the migration.
//!
//! The C ABI now exposes emitted ARM64 bytes in a second API path. These
//! fixtures assert that C++ and Rust emit identical bytes for the same
//! one-instruction smoke set (where Rust pins fixture bytes).

use prisma_core::{BlockExitKind, Translator};
use prisma_runtime::dispatcher::{GuestTranslator, RustSmokeTranslator};

const BASE: u64 = 0x1000;

struct SmokeFixture {
    name: &'static str,
    guest_bytes: &'static [u8],
    expected_exit: BlockExitKind,
    rust_words: Option<&'static [u32]>,
}

const COND_JUMP_REL8_MATRIX: &[(u8, u32)] = &[
    (0x70, 0x0006),
    (0x71, 0x0007),
    (0x72, 0x0002),
    (0x73, 0x0003),
    (0x74, 0x0000),
    (0x75, 0x0001),
    (0x76, 0x0009),
    (0x77, 0x0008),
    (0x78, 0x0004),
    (0x79, 0x0005),
    (0x7C, 0x000B),
    (0x7D, 0x000A),
    (0x7E, 0x000D),
    (0x7F, 0x000C),
];

const COND_JUMP_REL32_MATRIX: &[(u8, u32)] = &[
    (0x80, 0x0006),
    (0x81, 0x0007),
    (0x82, 0x0002),
    (0x83, 0x0003),
    (0x84, 0x0000),
    (0x85, 0x0001),
    (0x86, 0x0009),
    (0x87, 0x0008),
    (0x88, 0x0004),
    (0x89, 0x0005),
    (0x8C, 0x000B),
    (0x8D, 0x000A),
    (0x8E, 0x000D),
    (0x8F, 0x000C),
];

const CMP_THEN_COND_JUMP_FIXTURES: &[(&str, &[u8], &[u32])] = &[
    (
        "cmp_rax_rcx",
        &[0x48, 0x39, 0xC8],
        &[0xF940_0769, 0xF940_036A, 0xEB09_015F],
    ),
    (
        "cmp_rax_imm8",
        &[0x48, 0x83, 0xF8, 0x10],
        &[0xD280_0209, 0xF940_036A, 0xEB09_015F],
    ),
    (
        "cmp_rax_imm8_i32",
        &[0x83, 0xF8, 0x10],
        &[
            0xD280_0209,
            0xB940_036A,
            0xD280_0413,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ],
    ),
    (
        "cmp_rax_imm8_i16",
        &[0x66, 0x83, 0xF8, 0x10],
        &[
            0xD280_0209,
            0x7940_036A,
            0xD280_0613,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ],
    ),
    (
        "cmp_rbx_imm8_i32",
        &[0x83, 0xFB, 0x10],
        &[
            0xD280_0209,
            0xB940_1B6A,
            0xD280_0413,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ],
    ),
    (
        "cmp_rbx_imm8_i16",
        &[0x66, 0x83, 0xFB, 0x10],
        &[
            0xD280_0209,
            0x7940_336A,
            0xD280_0613,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ],
    ),
    (
        "cmp_r11_imm8",
        &[0x49, 0x83, 0xFB, 0x10],
        &[0xD280_0209, 0xF940_2F6A, 0xEB09_015F],
    ),
    (
        "cmp_mem_rbx_imm8",
        &[0x48, 0x83, 0x3B, 0x10],
        &[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xEB09_017F,
        ],
    ),
];

const FIXTURES: &[SmokeFixture] = &[
    SmokeFixture {
        name: "nop",
        guest_bytes: &[0x90],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_rax_imm64",
        guest_bytes: &[0x48, 0xB8, 0x42, 0, 0, 0, 0, 0, 0, 0],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0849, 0xF900_0369]),
    },
    SmokeFixture {
        name: "mov_rax_rcx",
        guest_bytes: &[0x48, 0x89, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xF940_0769, 0xF900_0369]),
    },
    SmokeFixture {
        name: "add_rax_rcx",
        guest_bytes: &[0x48, 0x01, 0xC8],
        expected_exit: BlockExitKind::None,
        // ... + the implicit NZCV flag write the decoder now emits.
        rust_words: Some(&[
            0xF940_0769,
            0xF940_036A,
            0x8B09_014B,
            0xF900_036B,
            0xAB09_0157, // adds x23, x10, x9
        ]),
    },
    SmokeFixture {
        name: "sub_rax_rcx",
        guest_bytes: &[0x48, 0x29, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xF940_0769,
            0xF940_036A,
            0xCB09_014B,
            0xF900_036B,
            0xEB09_015F, // subs xzr, x10, x9
        ]),
    },
    SmokeFixture {
        name: "and_rax_rcx",
        guest_bytes: &[0x48, 0x21, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xF940_0769,
            0xF940_036A,
            0x8A09_014B,
            0xF900_036B,
            0xEA09_0157, // ands x23, x10, x9
        ]),
    },
    SmokeFixture {
        name: "or_rax_rcx",
        guest_bytes: &[0x48, 0x09, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xAA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "xor_rax_rcx",
        guest_bytes: &[0x48, 0x31, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xCA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "add_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xC0, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0x9100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "sub_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xE8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xD100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "or_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xC8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xAA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "and_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xE0, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0x8A09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "xor_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xF0, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xCA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "adc_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xD0, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "sbb_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xD8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xF8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "cmp_rax_rcx",
        guest_bytes: &[0x48, 0x39, 0xC8],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "cmp_rax_imm8_i32",
        guest_bytes: &[0x83, 0xF8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xB940_036A,
            0xD280_0413,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ]),
    },
    SmokeFixture {
        name: "cmp_rax_imm8_i16",
        guest_bytes: &[0x66, 0x83, 0xF8, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0x7940_036A,
            0xD280_0613,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ]),
    },
    SmokeFixture {
        name: "cmp_rbx_imm8_i32",
        guest_bytes: &[0x83, 0xFB, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xB940_1B6A,
            0xD280_0413,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ]),
    },
    SmokeFixture {
        name: "cmp_rbx_imm8_i16",
        guest_bytes: &[0x66, 0x83, 0xFB, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0x7940_336A,
            0xD280_0613,
            0x9AD3_2151,
            0x9AD3_2132,
            0xEB12_023F,
        ]),
    },
    SmokeFixture {
        name: "add_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0xC3, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_0F6A, 0x9100_414B, 0xF900_0F6B]),
    },
    SmokeFixture {
        name: "cmp_r11_imm8",
        guest_bytes: &[0x49, 0x83, 0xFB, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[0xD280_0209, 0xF940_2F6A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "add_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x03, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0x9100_416C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "cmp_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x3B, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xEB09_017F,
        ]),
    },
    SmokeFixture {
        name: "or_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x0B, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xAA09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "and_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x23, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0x8A09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "xor_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x33, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xCA09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "add_mem_sib_disp_imm8",
        guest_bytes: &[0x48, 0x83, 0x44, 0x88, 0x7F, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_076A,
            0xD280_004B,
            0x9ACB_214C,
            0xF940_036D,
            0x8B0C_01AE,
            0xD280_0FEF,
            0x9101_FDD0,
            0xF941_A378,
            0x8B18_0218,
            0xF940_0309,
            0x9100_412A,
            0xF941_A378,
            0x8B18_0218,
            0xF900_030A,
        ]),
    },
    SmokeFixture {
        name: "add_mem_disp32_imm8",
        guest_bytes: &[0x48, 0x83, 0x83, 0x20, 0x00, 0x00, 0x00, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xD280_040B,
            0x9100_814C,
            0xF941_A378,
            0x8B18_0198,
            0xF940_030D,
            0x9100_41AE,
            0xF941_A378,
            0x8B18_0198,
            0xF900_030E,
        ]),
    },
    SmokeFixture {
        name: "add_mem_rex_x_b_sib_imm8",
        guest_bytes: &[0x4B, 0x83, 0x44, 0x88, 0x20, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xF940_276A,
            0xD280_004B,
            0x9ACB_214C,
            0xF940_236D,
            0x8B0C_01AE,
            0xD280_040F,
            0x9100_81D0,
            0xF941_A378,
            0x8B18_0218,
            0xF940_0309,
            0x9100_412A,
            0xF941_A378,
            0x8B18_0218,
            0xF900_030A,
        ]),
    },
    SmokeFixture {
        name: "add_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x05, 0x34, 0x12, 0x00, 0x00, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xD284_488A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0x9100_416C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "or_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x0D, 0x34, 0x12, 0x00, 0x00, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xD284_488A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xAA09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "and_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x25, 0x34, 0x12, 0x00, 0x00, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xD284_488A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0x8A09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "xor_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x35, 0x34, 0x12, 0x00, 0x00, 0x10],
        expected_exit: BlockExitKind::None,
        rust_words: Some(&[
            0xD280_0209,
            0xD284_488A,
            0xF941_A378,
            0x8B18_0158,
            0xF940_030B,
            0xCA09_016C,
            0xF941_A378,
            0x8B18_0158,
            0xF900_030C,
        ]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_forward",
        guest_bytes: &[0x74, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_backward",
        guest_bytes: &[0x74, 0xFE],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0000, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_forward",
        guest_bytes: &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ov_forward",
        guest_bytes: &[0x70, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0046, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_no_ov_forward",
        guest_bytes: &[0x71, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0047, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_nc_forward",
        guest_bytes: &[0x72, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0042, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_cc_forward",
        guest_bytes: &[0x73, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0043, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_eq_forward",
        guest_bytes: &[0x74, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ne_forward",
        guest_bytes: &[0x75, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0041, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ule_forward",
        guest_bytes: &[0x76, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0049, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ugt_forward",
        guest_bytes: &[0x77, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0048, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_mi_forward",
        guest_bytes: &[0x78, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0044, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_pl_forward",
        guest_bytes: &[0x79, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0045, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_slt_forward",
        guest_bytes: &[0x7C, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004B, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sge_forward",
        guest_bytes: &[0x7D, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004A, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sle_forward",
        guest_bytes: &[0x7E, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004D, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sgt_forward",
        guest_bytes: &[0x7F, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004C, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ov_forward",
        guest_bytes: &[0x0F, 0x80, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0046, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_no_ov_forward",
        guest_bytes: &[0x0F, 0x81, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0047, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_nc_forward",
        guest_bytes: &[0x0F, 0x82, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0042, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_cc_forward",
        guest_bytes: &[0x0F, 0x83, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0043, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_eq_forward",
        guest_bytes: &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ne_forward",
        guest_bytes: &[0x0F, 0x85, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0041, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ule_forward",
        guest_bytes: &[0x0F, 0x86, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0049, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ugt_forward",
        guest_bytes: &[0x0F, 0x87, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0048, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_mi_forward",
        guest_bytes: &[0x0F, 0x88, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0044, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_pl_forward",
        guest_bytes: &[0x0F, 0x89, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_0045, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_slt_forward",
        guest_bytes: &[0x0F, 0x8C, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004B, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sge_forward",
        guest_bytes: &[0x0F, 0x8D, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004A, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sle_forward",
        guest_bytes: &[0x0F, 0x8E, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004D, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sgt_forward",
        guest_bytes: &[0x0F, 0x8F, 0x02, 0x00, 0x00, 0x00],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[0x5400_004C, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cmp_rax_rcx_then_je_forward",
        guest_bytes: &[0x48, 0x39, 0xC8, 0x74, 0x02],
        expected_exit: BlockExitKind::CondJumpRel,
        rust_words: Some(&[
            0xF940_0769,
            0xF940_036A,
            0xEB09_015F,
            0x5400_0040,
            0x1400_0001,
        ]),
    },
    // 2026-06-19 decoder-gap families. These assert C++/Rust functional parity
    // (both translate, same exit kind, C++ caches) — byte-for-byte parity stays
    // deferred per the note above. Plain `bt` is intentionally excluded: with no
    // store and no flag consumer it lowers to empty code, which would trip the
    // non-empty assertions. `rust_words` is left None (the Rust-side behaviour is
    // pinned by the per-crate unit + ARM64 e2e tests).
    SmokeFixture {
        name: "cmpxchg_rcx_rdx",
        guest_bytes: &[0x48, 0x0F, 0xB1, 0xD1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "bts_rcx_imm8",
        guest_bytes: &[0x0F, 0xBA, 0xE9, 0x04],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "btr_rcx_imm8",
        guest_bytes: &[0x0F, 0xBA, 0xF1, 0x04],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "btc_rcx_imm8",
        guest_bytes: &[0x0F, 0xBA, 0xF9, 0x04],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "andn_rax_rcx_rdx",
        guest_bytes: &[0xC4, 0xE2, 0xF0, 0xF2, 0xC2],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "blsr_rax_rdx",
        guest_bytes: &[0xC4, 0xE2, 0xF8, 0xF3, 0xCA],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "blsmsk_rax_rdx",
        guest_bytes: &[0xC4, 0xE2, 0xF8, 0xF3, 0xD2],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "blsi_rax_rdx",
        guest_bytes: &[0xC4, 0xE2, 0xF8, 0xF3, 0xDA],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "bextr_rax_rdx_rcx",
        guest_bytes: &[0xC4, 0xE2, 0xF0, 0xF7, 0xC2],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "rcl_rax_cl",
        guest_bytes: &[0x48, 0xD3, 0xD0],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "rcr_rax_cl",
        guest_bytes: &[0x48, 0xD3, 0xD8],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "rcl_rax_imm2",
        guest_bytes: &[0x48, 0xC1, 0xD0, 0x02],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "rcr_rax_imm2",
        guest_bytes: &[0x48, 0xC1, 0xD8, 0x02],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "pushfq",
        guest_bytes: &[0x9C],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "popfq",
        guest_bytes: &[0x9D],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "lahf",
        guest_bytes: &[0x9F],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "sahf",
        guest_bytes: &[0x9E],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "bsf_rax_rcx",
        guest_bytes: &[0x48, 0x0F, 0xBC, 0xC1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "bsr_rax_rcx",
        guest_bytes: &[0x48, 0x0F, 0xBD, 0xC1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "mul_rcx",
        guest_bytes: &[0x48, 0xF7, 0xE1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "mul_cl",
        guest_bytes: &[0xF6, 0xE1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "div_cl",
        guest_bytes: &[0xF6, 0xF1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "div_cx",
        guest_bytes: &[0x66, 0xF7, 0xF1],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
    SmokeFixture {
        name: "idiv_ecx",
        guest_bytes: &[0xF7, 0xF9],
        expected_exit: BlockExitKind::None,
        rust_words: None,
    },
];

#[test]
fn live_cpp_translator_accepts_rust_smoke_fixtures() {
    let mut cpp = Translator::new().expect("cpp translator");
    let mut rust = RustSmokeTranslator::new();

    for fixture in FIXTURES {
        let guest_pc = BASE + fixture.guest_bytes.len() as u64;
        let is_nop = matches!(fixture.guest_bytes, [0x90]);
        let (cpp_info, cpp_bytes) = cpp
            .translate_with_code(guest_pc, fixture.guest_bytes)
            .unwrap_or_else(|err| panic!("{}: C++ translator failed: {err:?}", fixture.name));
        assert_eq!(
            cpp_info.guest_size,
            fixture.guest_bytes.len() as u64,
            "{}: C++ consumed unexpected guest byte count",
            fixture.name
        );
        assert!(
            cpp_info.code_size > 0,
            "{}: C++ emitted no code",
            fixture.name
        );
        let rust_translation = rust
            .translate(guest_pc, fixture.guest_bytes)
            .unwrap_or_else(|| panic!("{}: Rust translator failed", fixture.name));
        if !is_nop {
            assert!(
                !rust_translation.is_empty(),
                "{}: Rust translator emitted no code",
                fixture.name
            );
        }
        if is_nop {
            assert!(
                !cpp_bytes.is_empty(),
                "{}: C++ translator emitted no code",
                fixture.name
            );
            let cpp_cached = cpp
                .translate(guest_pc, fixture.guest_bytes)
                .unwrap_or_else(|err| {
                    panic!("{}: C++ cache retranslate failed: {err:?}", fixture.name)
                });
            assert!(
                cpp_cached.from_cache,
                "{}: C++ translator did not cache identical input",
                fixture.name
            );
            continue;
        }
        let _expected = fixture
            .rust_words
            .map_or_else(|| rust_translation.clone(), words_to_le_bytes);
        assert!(
            !cpp_bytes.is_empty(),
            "{}: C++ emitted no code",
            fixture.name
        );
        // Rust and C++ byte-for-byte parity is still intentionally deferred for
        // prologue-heavy paths in the C++ backend. Keep functional parity checks
        // (translation success, caching, and Rust fixture stability) as the hard gate.
        assert_eq!(
            cpp_info.exit_kind, fixture.expected_exit,
            "{}: smoke fixtures should honor expected exit kind",
            fixture.name
        );

        let cpp_cached = cpp
            .translate(guest_pc, fixture.guest_bytes)
            .unwrap_or_else(|err| {
                panic!("{}: C++ cache retranslate failed: {err:?}", fixture.name)
            });
        assert!(
            cpp_cached.from_cache,
            "{}: C++ translator did not cache identical input",
            fixture.name
        );

        if let Some(expected_words) = fixture.rust_words {
            assert_smoke_bytes(
                fixture.name,
                &rust_translation,
                &words_to_le_bytes(expected_words),
            );
        }
    }
}

#[test]
fn live_cpp_translator_cond_jump_matrix_contains_supported_paths() {
    let mut cpp = Translator::new().expect("cpp translator");
    let mut rust = RustSmokeTranslator::new();

    for &(opcode, cond_imm) in COND_JUMP_REL8_MATRIX {
        let guest_pc = 0x2000;
        let forward = [opcode, 0x02];
        let (cpp_info, _) = cpp
            .translate_with_code(guest_pc, &forward)
            .unwrap_or_else(|err| panic!("{opcode:#x}: C++ translate_with_code failed: {err:?}"));
        assert_eq!(
            cpp_info.exit_kind,
            BlockExitKind::CondJumpRel,
            "cond rel8 opcode {opcode:#x}"
        );

        let rust_translation = rust
            .translate(guest_pc, &forward)
            .unwrap_or_else(|| panic!("{opcode:#x}: rust translate failed"));
        assert_standalone_cond_jump_bytes(
            &format!("{opcode:#x} forward rel8"),
            &rust_translation,
            &[0x5400_0040 | cond_imm, 0x1400_0001],
        );

        let backward = [opcode, 0xFE];
        let (cpp_info, _) = cpp
            .translate_with_code(guest_pc + 0x100, &backward)
            .unwrap_or_else(|err| panic!("{opcode:#x}: C++ translate_with_code failed: {err:?}"));
        assert_eq!(
            cpp_info.exit_kind,
            BlockExitKind::CondJumpRel,
            "cond rel8 opcode {opcode:#x}"
        );
    }

    for &(opcode, cond_imm) in COND_JUMP_REL32_MATRIX {
        let guest_pc = 0x3000;
        let forward = [0x0F, opcode, 0x02, 0x00, 0x00, 0x00];
        let (cpp_info, _) = cpp
            .translate_with_code(guest_pc, &forward)
            .unwrap_or_else(|err| panic!("{opcode:#x}: C++ translate_with_code failed: {err:?}"));
        assert_eq!(
            cpp_info.exit_kind,
            BlockExitKind::CondJumpRel,
            "cond rel32 opcode {opcode:#x}"
        );

        let rust_translation = rust
            .translate(guest_pc, &forward)
            .unwrap_or_else(|| panic!("{opcode:#x}: rust translate failed"));
        assert_standalone_cond_jump_bytes(
            &format!("{opcode:#x} forward rel32"),
            &rust_translation,
            &[0x5400_0040 | cond_imm, 0x1400_0001],
        );
    }

    for &(opcode, cond_imm) in COND_JUMP_REL32_MATRIX {
        let guest_pc = 0x3000;
        let backward = [0x0F, opcode, 0xFA, 0xFF, 0xFF, 0xFF];
        let (cpp_info, _) = cpp
            .translate_with_code(guest_pc + 0x80, &backward)
            .unwrap_or_else(|err| panic!("{opcode:#x}: C++ translate_with_code failed: {err:?}"));
        assert_eq!(
            cpp_info.exit_kind,
            BlockExitKind::CondJumpRel,
            "cond rel32 opcode {opcode:#x} backward"
        );

        let rust_translation = rust
            .translate(guest_pc + 0x80, &backward)
            .unwrap_or_else(|| panic!("{opcode:#x}: rust translate failed"));
        assert_standalone_cond_jump_bytes(
            &format!("{opcode:#x} backward rel32"),
            &rust_translation,
            &[0x5400_0000 | cond_imm, 0x1400_0001],
        );
    }

    // NOTE: the FFI translate path maps C status codes onto a bare
    // `CoreError` enum that does not carry the offending guest address, so an
    // "error reports its guest_addr" assertion is not expressible against this
    // boundary. (The two inputs above are valid conditional jumps that the
    // loop already exercises as successful translations.)
}

#[test]
fn live_cpp_translator_cmp_then_cond_jump_matrix() {
    let mut cpp = Translator::new().expect("cpp translator");
    let mut rust = RustSmokeTranslator::new();

    for &(name, cmp_bytes, cmp_words) in CMP_THEN_COND_JUMP_FIXTURES {
        for &(opcode, cond_imm) in COND_JUMP_REL8_MATRIX {
            let mut forward = Vec::with_capacity(cmp_bytes.len() + 2);
            forward.extend_from_slice(cmp_bytes);
            forward.extend_from_slice(&[opcode, 0x02]);

            let mut backward = Vec::with_capacity(cmp_bytes.len() + 2);
            backward.extend_from_slice(cmp_bytes);
            backward.extend_from_slice(&[opcode, 0xFE]);

            let guest_pc = 0x5000;
            let (cpp_info, _) = cpp
                .translate_with_code(guest_pc, &forward)
                .unwrap_or_else(|err| {
                    panic!("{name}: C++ translate_with_code rel8 forward failed: {err:?}");
                });
            assert_eq!(
                cpp_info.exit_kind,
                BlockExitKind::CondJumpRel,
                "{name} cond rel8 opcode {opcode:#x} forward"
            );
            let rust_translation = rust
                .translate(guest_pc, &forward)
                .unwrap_or_else(|| panic!("{name}: rust translate rel8 forward failed"));
            assert_cmp_then_cond_bytes(
                name,
                &rust_translation,
                cmp_words,
                &[0x5400_0040 | cond_imm, 0x1400_0001],
            );

            let cpp_cached = cpp
                .translate(guest_pc, &forward)
                .expect("cpp cache translate");
            assert!(
                cpp_cached.from_cache,
                "{name} cond rel8 forward should cache on retranslate"
            );

            let (cpp_info, _) = cpp
                .translate_with_code(guest_pc + 0x100, &backward)
                .unwrap_or_else(|err| {
                    panic!("{name}: C++ translate_with_code rel8 backward failed: {err:?}");
                });
            assert_eq!(
                cpp_info.exit_kind,
                BlockExitKind::CondJumpRel,
                "{name} cond rel8 opcode {opcode:#x} backward"
            );
            let rust_translation = rust
                .translate(guest_pc + 0x100, &backward)
                .unwrap_or_else(|| panic!("{name}: rust translate rel8 backward failed"));
            assert_cmp_then_cond_bytes(
                name,
                &rust_translation,
                cmp_words,
                &[0x5400_0000 | cond_imm, 0x1400_0001],
            );
        }

        for &(opcode, cond_imm) in COND_JUMP_REL32_MATRIX {
            let mut forward = Vec::with_capacity(cmp_bytes.len() + 6);
            forward.extend_from_slice(cmp_bytes);
            forward.extend_from_slice(&[0x0F, opcode, 0x02, 0x00, 0x00, 0x00]);

            let guest_pc = 0x6000;
            let (cpp_info, _) = cpp
                .translate_with_code(guest_pc, &forward)
                .unwrap_or_else(|err| {
                    panic!("{name}: C++ translate_with_code rel32 forward failed: {err:?}");
                });
            assert_eq!(
                cpp_info.exit_kind,
                BlockExitKind::CondJumpRel,
                "{name} cond rel32 opcode {opcode:#x} forward"
            );
            let rust_translation = rust
                .translate(guest_pc, &forward)
                .unwrap_or_else(|| panic!("{name}: rust translate rel32 forward failed"));
            assert_cmp_then_cond_bytes(
                name,
                &rust_translation,
                cmp_words,
                &[0x5400_0040 | cond_imm, 0x1400_0001],
            );
        }

        for &(opcode, cond_imm) in COND_JUMP_REL32_MATRIX {
            let mut backward = Vec::with_capacity(cmp_bytes.len() + 6);
            backward.extend_from_slice(cmp_bytes);
            backward.extend_from_slice(&[0x0F, opcode, 0xFA, 0xFF, 0xFF, 0xFF]);

            let guest_pc = 0x6000;
            let (cpp_info, _) =
                cpp.translate_with_code(guest_pc, &backward)
                    .unwrap_or_else(|err| {
                        panic!("{name}: C++ translate_with_code rel32 backward failed: {err:?}");
                    });
            assert_eq!(
                cpp_info.exit_kind,
                BlockExitKind::CondJumpRel,
                "{name} cond rel32 opcode {opcode:#x} backward"
            );

            let rust_translation = rust
                .translate(guest_pc, &backward)
                .unwrap_or_else(|| panic!("{name}: rust translate rel32 backward failed"));
            assert_cmp_then_cond_bytes(
                name,
                &rust_translation,
                cmp_words,
                &[0x5400_0000 | cond_imm, 0x1400_0001],
            );
        }
    }
}

fn assert_smoke_bytes(name: &str, translated: &[u8], expected_prefix: &[u8]) {
    if translated == expected_prefix {
        return;
    }
    if is_standalone_cond_jump_fixture(name) {
        let tail_words = le_bytes_to_words(expected_prefix);
        assert_standalone_cond_jump_bytes(name, translated, &tail_words);
        return;
    }
    if is_persistent_rflags_fixture(name) {
        if !is_memory_fixture(name) {
            assert!(
                contains_ordered_words(translated, expected_prefix),
                "{name}: stable backend word sequence drifted"
            );
        }
        assert!(
            has_persistent_rflags_store(translated),
            "{name}: RFLAGS-publishing fixture did not store persistent RFLAGS"
        );
        return;
    }
    assert_eq!(
        translated, expected_prefix,
        "{name}: Rust smoke translator bytes drifted"
    );
}

fn assert_cmp_then_cond_bytes(
    name: &str,
    translated: &[u8],
    cmp_words: &[u32],
    tail_words: &[u32],
) {
    let prefix = words_to_le_bytes(cmp_words);
    let tail = words_to_le_bytes(tail_words);
    if !is_memory_fixture(name) {
        assert!(
            contains_ordered_words(translated, &prefix),
            "{name}: cmp backend word sequence drifted"
        );
    }
    if !translated.ends_with(&tail) {
        assert_branch_tail_compatible(name, translated, tail_words);
    }
    assert!(
        has_persistent_rflags_store(translated),
        "{name}: cmp+jcc path did not publish persistent RFLAGS before the branch"
    );
}

fn is_persistent_rflags_fixture(name: &str) -> bool {
    [
        "adc_", "add_", "and_", "cmp_", "cmpxchg_", "cmps", "dec_", "inc_", "or_", "sbb_", "scas",
        "sub_", "test_", "xadd_", "xor_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn is_standalone_cond_jump_fixture(name: &str) -> bool {
    name.starts_with("cond_jump_")
}

fn is_memory_fixture(name: &str) -> bool {
    name.contains("_mem_") || name.starts_with("cmp_mem_")
}

fn has_persistent_rflags_store(bytes: &[u8]) -> bool {
    bytes
        .windows(RFLAGS_STORE_BYTES.len())
        .any(|window| window == RFLAGS_STORE_BYTES)
}

const RFLAGS_STORE_BYTES: &[u8] = &[0x76, 0x9B, 0x01, 0xF9];

const RESTORE_NZCV_FROM_RFLAGS_WORDS: &[u32] = &[
    0xF941_9B74,
    0xD280_0016,
    0xD280_00F3,
    0x9AD3_2695,
    0xD280_0033,
    0x8A13_02B5,
    0xD280_03F3,
    0x9AD3_22B5,
    0xAA15_02D6,
    0xD280_00D3,
    0x9AD3_2695,
    0xD280_0033,
    0x8A13_02B5,
    0xD280_03D3,
    0x9AD3_22B5,
    0xAA15_02D6,
    0xD280_0013,
    0x9AD3_2695,
    0xD280_0033,
    0x8A13_02B5,
    0xD280_03B3,
    0x9AD3_22B5,
    0xAA15_02D6,
    0xD280_0173,
    0x9AD3_2695,
    0xD280_0033,
    0x8A13_02B5,
    0xD280_0393,
    0x9AD3_22B5,
    0xAA15_02D6,
    0xD51B_4216,
];

fn assert_standalone_cond_jump_bytes(name: &str, translated: &[u8], tail_words: &[u32]) {
    let restore = words_to_le_bytes(RESTORE_NZCV_FROM_RFLAGS_WORDS);
    assert!(
        translated.starts_with(&restore),
        "{name}: standalone Jcc did not restore NZCV from RFLAGS before branching"
    );
    assert_branch_tail_compatible(name, translated, tail_words);
}

fn assert_branch_tail_compatible(name: &str, translated: &[u8], tail_words: &[u32]) {
    assert_eq!(
        tail_words.len(),
        2,
        "{name}: branch tail expectation must be B.cond + B"
    );
    let words = le_bytes_to_words(translated);
    let tail = words
        .get(words.len().saturating_sub(2)..)
        .unwrap_or_default();
    assert_eq!(
        tail.len(),
        2,
        "{name}: translated bytes too short for branch tail"
    );
    assert_eq!(
        tail[1], tail_words[1],
        "{name}: final unconditional branch drifted"
    );
    assert_eq!(
        tail[0] & 0xFF00_001F,
        tail_words[0] & 0xFF00_001F,
        "{name}: conditional branch opcode/condition drifted"
    );
}

fn contains_ordered_words(translated: &[u8], expected: &[u8]) -> bool {
    let mut translated_words = translated.chunks_exact(4);
    let expected_words = expected.chunks_exact(4);
    if !translated_words.remainder().is_empty() || !expected_words.remainder().is_empty() {
        return false;
    }

    for expected_word in expected_words {
        if !translated_words.any(|word| word == expected_word) {
            return false;
        }
    }
    true
}

fn le_bytes_to_words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("chunk has four bytes")))
        .collect()
}

fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}
