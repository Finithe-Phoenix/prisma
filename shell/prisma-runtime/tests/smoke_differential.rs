use prisma_runtime::dispatcher::{GuestTranslator, RustSmokeTranslator};

struct SmokeFixture {
    name: &'static str,
    guest_bytes: &'static [u8],
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

const FIXTURES: &[SmokeFixture] = &[
    SmokeFixture {
        name: "nop",
        guest_bytes: &[0x90],
        rust_words: None,
    },
    SmokeFixture {
        name: "pause",
        guest_bytes: &[0xF3, 0x90],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "fwait",
        guest_bytes: &[0x9B],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "mov_rax_imm64",
        guest_bytes: &[0x48, 0xB8, 0x42, 0, 0, 0, 0, 0, 0, 0],
        rust_words: Some(&[0xD280_0849, 0xF900_0369]),
    },
    SmokeFixture {
        name: "mov_rax_rcx",
        guest_bytes: &[0x48, 0x89, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF900_0369]),
    },
    SmokeFixture {
        name: "add_rax_rcx",
        guest_bytes: &[0x48, 0x01, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0x8B09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "sub_rax_rcx",
        guest_bytes: &[0x48, 0x29, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xCB09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "and_rax_rcx",
        guest_bytes: &[0x48, 0x21, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0x8A09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "or_rax_rcx",
        guest_bytes: &[0x48, 0x09, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xAA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "xor_rax_rcx",
        guest_bytes: &[0x48, 0x31, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xCA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "add_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xC0, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0x9100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "sub_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xE8, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xD100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "or_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xC8, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xAA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "and_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xE0, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0x8A09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "xor_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xF0, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xCA09_014B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "adc_rax_imm8_placeholder",
        guest_bytes: &[0x48, 0x83, 0xD0, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0x9100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "sbb_rax_imm8_placeholder",
        guest_bytes: &[0x48, 0x83, 0xD8, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xD100_414B, 0xF900_036B]),
    },
    SmokeFixture {
        name: "cmp_rax_imm8",
        guest_bytes: &[0x48, 0x83, 0xF8, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_036A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "cmp_rax_rcx",
        guest_bytes: &[0x48, 0x39, 0xC8],
        rust_words: Some(&[0xF940_0769, 0xF940_036A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "cmp_rax_imm8_i32",
        guest_bytes: &[0x83, 0xF8, 0x10],
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
        rust_words: Some(&[0xD280_0209, 0xF940_0F6A, 0x9100_414B, 0xF900_0F6B]),
    },
    SmokeFixture {
        name: "cmp_r11_imm8",
        guest_bytes: &[0x49, 0x83, 0xFB, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_2F6A, 0xEB09_015F]),
    },
    SmokeFixture {
        name: "add_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x03, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF940_014B,
            0x9100_416C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "cmp_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x3B, 0x10],
        rust_words: Some(&[0xD280_0209, 0xF940_0F6A, 0xF940_014B, 0xEB09_017F]),
    },
    SmokeFixture {
        name: "or_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x0B, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF940_014B,
            0xAA09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "and_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x23, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF940_014B,
            0x8A09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "xor_mem_rbx_imm8",
        guest_bytes: &[0x48, 0x83, 0x33, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xF940_014B,
            0xCA09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "add_mem_sib_disp_imm8",
        guest_bytes: &[0x48, 0x83, 0x44, 0x88, 0x7F, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_076A,
            0xD280_004B,
            0x9ACB_214C,
            0xF940_036D,
            0x8B0C_01AE,
            0xD280_0FEF,
            0x9101_FDD0,
            0xF940_0209,
            0x9100_412A,
            0xF900_020A,
        ]),
    },
    SmokeFixture {
        name: "add_mem_disp32_imm8",
        guest_bytes: &[0x48, 0x83, 0x83, 0x20, 0x00, 0x00, 0x00, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_0F6A,
            0xD280_040B,
            0x9100_814C,
            0xF940_018D,
            0x9100_41AE,
            0xF900_018E,
        ]),
    },
    SmokeFixture {
        name: "add_mem_rex_x_b_sib_imm8",
        guest_bytes: &[0x4B, 0x83, 0x44, 0x88, 0x20, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xF940_276A,
            0xD280_004B,
            0x9ACB_214C,
            0xF940_236D,
            0x8B0C_01AE,
            0xD280_040F,
            0x9100_81D0,
            0xF940_0209,
            0x9100_412A,
            0xF900_020A,
        ]),
    },
    SmokeFixture {
        name: "add_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x05, 0x34, 0x12, 0x00, 0x00, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xD284_478A,
            0xF940_014B,
            0x9100_416C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "or_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x0D, 0x34, 0x12, 0x00, 0x00, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xD284_478A,
            0xF940_014B,
            0xAA09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "and_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x25, 0x34, 0x12, 0x00, 0x00, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xD284_478A,
            0xF940_014B,
            0x8A09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "xor_mem_rip_relative_imm8",
        guest_bytes: &[0x48, 0x83, 0x35, 0x34, 0x12, 0x00, 0x00, 0x10],
        rust_words: Some(&[
            0xD280_0209,
            0xD284_478A,
            0xF940_014B,
            0xCA09_016C,
            0xF900_014C,
        ]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_forward",
        guest_bytes: &[0x74, 0x02],
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_backward",
        guest_bytes: &[0x74, 0xFE],
        rust_words: Some(&[0x5400_0000, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_forward",
        guest_bytes: &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ov_forward",
        guest_bytes: &[0x70, 0x02],
        rust_words: Some(&[0x5400_0046, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_no_ov_forward",
        guest_bytes: &[0x71, 0x02],
        rust_words: Some(&[0x5400_0047, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_nc_forward",
        guest_bytes: &[0x72, 0x02],
        rust_words: Some(&[0x5400_0042, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_cc_forward",
        guest_bytes: &[0x73, 0x02],
        rust_words: Some(&[0x5400_0043, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_eq_forward",
        guest_bytes: &[0x74, 0x02],
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ne_forward",
        guest_bytes: &[0x75, 0x02],
        rust_words: Some(&[0x5400_0041, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ule_forward",
        guest_bytes: &[0x76, 0x02],
        rust_words: Some(&[0x5400_0049, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_ugt_forward",
        guest_bytes: &[0x77, 0x02],
        rust_words: Some(&[0x5400_0048, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_mi_forward",
        guest_bytes: &[0x78, 0x02],
        rust_words: Some(&[0x5400_0044, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_pl_forward",
        guest_bytes: &[0x79, 0x02],
        rust_words: Some(&[0x5400_0045, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_slt_forward",
        guest_bytes: &[0x7C, 0x02],
        rust_words: Some(&[0x5400_004B, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sge_forward",
        guest_bytes: &[0x7D, 0x02],
        rust_words: Some(&[0x5400_004A, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sle_forward",
        guest_bytes: &[0x7E, 0x02],
        rust_words: Some(&[0x5400_004D, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel8_sgt_forward",
        guest_bytes: &[0x7F, 0x02],
        rust_words: Some(&[0x5400_004C, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ov_forward",
        guest_bytes: &[0x0F, 0x80, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0046, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_no_ov_forward",
        guest_bytes: &[0x0F, 0x81, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0047, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_nc_forward",
        guest_bytes: &[0x0F, 0x82, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0042, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_cc_forward",
        guest_bytes: &[0x0F, 0x83, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0043, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_eq_forward",
        guest_bytes: &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0040, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ne_forward",
        guest_bytes: &[0x0F, 0x85, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0041, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ule_forward",
        guest_bytes: &[0x0F, 0x86, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0049, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_ugt_forward",
        guest_bytes: &[0x0F, 0x87, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0048, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_mi_forward",
        guest_bytes: &[0x0F, 0x88, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0044, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_pl_forward",
        guest_bytes: &[0x0F, 0x89, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_0045, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_slt_forward",
        guest_bytes: &[0x0F, 0x8C, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_004B, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sge_forward",
        guest_bytes: &[0x0F, 0x8D, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_004A, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sle_forward",
        guest_bytes: &[0x0F, 0x8E, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_004D, 0x1400_0001]),
    },
    SmokeFixture {
        name: "cond_jump_rel32_sgt_forward",
        guest_bytes: &[0x0F, 0x8F, 0x02, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x5400_004C, 0x1400_0001]),
    },
    SmokeFixture {
        name: "call_rel32",
        guest_bytes: &[0xE8, 0x00, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x1400_0001]),
    },
    SmokeFixture {
        name: "jump_rel8_forward",
        guest_bytes: &[0xEB, 0x02],
        rust_words: Some(&[0x1400_0001]),
    },
    SmokeFixture {
        name: "jump_rel8_backward",
        guest_bytes: &[0xEB, 0xFE],
        rust_words: Some(&[0x1400_0000]),
    },
    SmokeFixture {
        name: "jump_rel32",
        guest_bytes: &[0xE9, 0x00, 0x00, 0x00, 0x00],
        rust_words: Some(&[0x1400_0001]),
    },
    SmokeFixture {
        name: "ret",
        guest_bytes: &[0xC3],
        rust_words: Some(&[0xD65F_03C0]),
    },
    SmokeFixture {
        name: "ret_imm16",
        guest_bytes: &[0xC2, 0x10, 0x00],
        rust_words: Some(&[0xF940_1375, 0x9100_62B5, 0xF900_1375, 0xD65F_03C0]),
    },
    SmokeFixture {
        name: "iret",
        guest_bytes: &[0xCF],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rax_rcx_then_je_forward",
        guest_bytes: &[0x48, 0x39, 0xC8, 0x74, 0x02],
        rust_words: Some(&[
            0xF940_0769,
            0xF940_036A,
            0xEB09_015F,
            0x5400_0040,
            0x1400_0001,
        ]),
    },
    SmokeFixture {
        name: "push_rm64_reg",
        guest_bytes: &[0xFF, 0xF0],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_r8_rm8_reg",
        guest_bytes: &[0x8A, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "add_rm8_r8_reg",
        guest_bytes: &[0x00, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "nop_multibyte",
        guest_bytes: &[0x0F, 0x1F, 0x00],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "prefetch_t0",
        guest_bytes: &[0x0F, 0x18, 0x08],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "prefetchw",
        guest_bytes: &[0x0F, 0x0D, 0x08],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "femms",
        guest_bytes: &[0x0F, 0x0E],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "emms",
        guest_bytes: &[0x0F, 0x77],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "lfence",
        guest_bytes: &[0x0F, 0xAE, 0xE8],
        rust_words: None,
    },
    SmokeFixture {
        name: "mfence",
        guest_bytes: &[0x0F, 0xAE, 0xF0],
        rust_words: None,
    },
    SmokeFixture {
        name: "sfence",
        guest_bytes: &[0x0F, 0xAE, 0xF8],
        rust_words: None,
    },
    SmokeFixture {
        name: "clflush",
        guest_bytes: &[0x0F, 0xAE, 0x38],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "clflush_sib_mem",
        guest_bytes: &[0x0F, 0xAE, 0xBC, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "clwb",
        guest_bytes: &[0x66, 0x0F, 0xAE, 0x30],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "clwb_sib_mem",
        guest_bytes: &[0x66, 0x0F, 0xAE, 0xB4, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "clflushopt",
        guest_bytes: &[0x66, 0x0F, 0xAE, 0x38],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "clflushopt_sib_mem",
        guest_bytes: &[0x66, 0x0F, 0xAE, 0xBC, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "endbr64",
        guest_bytes: &[0xF3, 0x0F, 0x1E, 0xFA],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "rdtsc",
        guest_bytes: &[0x0F, 0x31],
        rust_words: None,
    },
    SmokeFixture {
        name: "xgetbv",
        guest_bytes: &[0x0F, 0x01, 0xD0],
        rust_words: None,
    },
    SmokeFixture {
        name: "lldt",
        guest_bytes: &[0x0F, 0x00, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "ltr",
        guest_bytes: &[0x0F, 0x00, 0x18],
        rust_words: None,
    },
    SmokeFixture {
        name: "syscall",
        guest_bytes: &[0x0F, 0x05],
        rust_words: None,
    },
    SmokeFixture {
        name: "ud2",
        guest_bytes: &[0x0F, 0x0B],
        rust_words: None,
    },
    SmokeFixture {
        name: "ud1_reg",
        guest_bytes: &[0x0F, 0xB9, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "ud1_sib_mem",
        guest_bytes: &[0x0F, 0xB9, 0x84, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "ud0_mem",
        guest_bytes: &[0x0F, 0xFF, 0x40, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "ud0_sib_mem",
        guest_bytes: &[0x0F, 0xFF, 0x84, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "invd",
        guest_bytes: &[0x0F, 0x08],
        rust_words: None,
    },
    SmokeFixture {
        name: "wbinvd",
        guest_bytes: &[0x0F, 0x09],
        rust_words: None,
    },
    SmokeFixture {
        name: "wbnoinvd",
        guest_bytes: &[0xF3, 0x0F, 0x09],
        rust_words: None,
    },
    SmokeFixture {
        name: "clts",
        guest_bytes: &[0x0F, 0x06],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_r_cr0",
        guest_bytes: &[0x0F, 0x20, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_r_dr0",
        guest_bytes: &[0x0F, 0x21, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_cr0_r",
        guest_bytes: &[0x0F, 0x22, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_dr0_r",
        guest_bytes: &[0x0F, 0x23, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "sysret",
        guest_bytes: &[0x0F, 0x07],
        rust_words: None,
    },
    SmokeFixture {
        name: "wrmsr",
        guest_bytes: &[0x0F, 0x30],
        rust_words: None,
    },
    SmokeFixture {
        name: "rdmsr",
        guest_bytes: &[0x0F, 0x32],
        rust_words: None,
    },
    SmokeFixture {
        name: "rdpmc",
        guest_bytes: &[0x0F, 0x33],
        rust_words: None,
    },
    SmokeFixture {
        name: "getsec",
        guest_bytes: &[0x0F, 0x37],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmread_reg",
        guest_bytes: &[0x0F, 0x78, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmread_sib_mem",
        guest_bytes: &[0x0F, 0x78, 0x84, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmwrite_mem",
        guest_bytes: &[0x0F, 0x79, 0x40, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmwrite_sib_mem",
        guest_bytes: &[0x0F, 0x79, 0x84, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "sysenter",
        guest_bytes: &[0x0F, 0x34],
        rust_words: None,
    },
    SmokeFixture {
        name: "sysexit",
        guest_bytes: &[0x0F, 0x35],
        rust_words: None,
    },
    SmokeFixture {
        name: "rsm",
        guest_bytes: &[0x0F, 0xAA],
        rust_words: None,
    },
    SmokeFixture {
        name: "lgdt",
        guest_bytes: &[0x0F, 0x01, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "lgdt_sib_mem",
        guest_bytes: &[0x0F, 0x01, 0x94, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "lidt",
        guest_bytes: &[0x0F, 0x01, 0x18],
        rust_words: None,
    },
    SmokeFixture {
        name: "lidt_sib_mem",
        guest_bytes: &[0x0F, 0x01, 0x9C, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "lmsw",
        guest_bytes: &[0x0F, 0x01, 0xF0],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmcall",
        guest_bytes: &[0x0F, 0x01, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmlaunch",
        guest_bytes: &[0x0F, 0x01, 0xC2],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmresume",
        guest_bytes: &[0x0F, 0x01, 0xC3],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmxoff",
        guest_bytes: &[0x0F, 0x01, 0xC4],
        rust_words: None,
    },
    SmokeFixture {
        name: "monitor",
        guest_bytes: &[0x0F, 0x01, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "mwait",
        guest_bytes: &[0x0F, 0x01, 0xC9],
        rust_words: None,
    },
    SmokeFixture {
        name: "clac",
        guest_bytes: &[0x0F, 0x01, 0xCA],
        rust_words: None,
    },
    SmokeFixture {
        name: "stac",
        guest_bytes: &[0x0F, 0x01, 0xCB],
        rust_words: None,
    },
    SmokeFixture {
        name: "xsetbv",
        guest_bytes: &[0x0F, 0x01, 0xD1],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmfunc",
        guest_bytes: &[0x0F, 0x01, 0xD4],
        rust_words: None,
    },
    SmokeFixture {
        name: "vmrun",
        guest_bytes: &[0x0F, 0x01, 0xD8],
        rust_words: None,
    },
    SmokeFixture {
        name: "invlpga",
        guest_bytes: &[0x0F, 0x01, 0xDF],
        rust_words: None,
    },
    SmokeFixture {
        name: "swapgs",
        guest_bytes: &[0x0F, 0x01, 0xF8],
        rust_words: None,
    },
    SmokeFixture {
        name: "invlpg",
        guest_bytes: &[0x0F, 0x01, 0x38],
        rust_words: None,
    },
    SmokeFixture {
        name: "invlpg_sib_mem",
        guest_bytes: &[0x0F, 0x01, 0xBC, 0x88, 0x20, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "int3",
        guest_bytes: &[0xCC],
        rust_words: None,
    },
    SmokeFixture {
        name: "int_0x80",
        guest_bytes: &[0xCD, 0x80],
        rust_words: None,
    },
    SmokeFixture {
        name: "push_es_invalid",
        guest_bytes: &[0x06],
        rust_words: None,
    },
    SmokeFixture {
        name: "pop_es_invalid",
        guest_bytes: &[0x07],
        rust_words: None,
    },
    SmokeFixture {
        name: "push_cs_invalid",
        guest_bytes: &[0x0E],
        rust_words: None,
    },
    SmokeFixture {
        name: "push_ss_invalid",
        guest_bytes: &[0x16],
        rust_words: None,
    },
    SmokeFixture {
        name: "pop_ss_invalid",
        guest_bytes: &[0x17],
        rust_words: None,
    },
    SmokeFixture {
        name: "push_ds_invalid",
        guest_bytes: &[0x1E],
        rust_words: None,
    },
    SmokeFixture {
        name: "pop_ds_invalid",
        guest_bytes: &[0x1F],
        rust_words: None,
    },
    SmokeFixture {
        name: "daa_invalid",
        guest_bytes: &[0x27],
        rust_words: None,
    },
    SmokeFixture {
        name: "das_invalid",
        guest_bytes: &[0x2F],
        rust_words: None,
    },
    SmokeFixture {
        name: "aaa_invalid",
        guest_bytes: &[0x37],
        rust_words: None,
    },
    SmokeFixture {
        name: "aas_invalid",
        guest_bytes: &[0x3F],
        rust_words: None,
    },
    SmokeFixture {
        name: "pusha_invalid",
        guest_bytes: &[0x60],
        rust_words: None,
    },
    SmokeFixture {
        name: "popa_invalid",
        guest_bytes: &[0x61],
        rust_words: None,
    },
    SmokeFixture {
        name: "call_far_invalid",
        guest_bytes: &[0x9A],
        rust_words: None,
    },
    SmokeFixture {
        name: "into_invalid",
        guest_bytes: &[0xCE],
        rust_words: None,
    },
    SmokeFixture {
        name: "salc_invalid",
        guest_bytes: &[0xD6],
        rust_words: None,
    },
    SmokeFixture {
        name: "aam_invalid",
        guest_bytes: &[0xD4, 0x0A],
        rust_words: None,
    },
    SmokeFixture {
        name: "aad_invalid",
        guest_bytes: &[0xD5, 0x0A],
        rust_words: None,
    },
    SmokeFixture {
        name: "jmp_far_invalid",
        guest_bytes: &[0xEA],
        rust_words: None,
    },
    SmokeFixture {
        name: "icebp",
        guest_bytes: &[0xF1],
        rust_words: None,
    },
    SmokeFixture {
        name: "hlt",
        guest_bytes: &[0xF4],
        rust_words: None,
    },
    SmokeFixture {
        name: "cli",
        guest_bytes: &[0xFA],
        rust_words: None,
    },
    SmokeFixture {
        name: "sti",
        guest_bytes: &[0xFB],
        rust_words: None,
    },
    SmokeFixture {
        name: "in_al_imm8",
        guest_bytes: &[0xE4, 0x20],
        rust_words: None,
    },
    SmokeFixture {
        name: "out_dx_eax",
        guest_bytes: &[0xEF],
        rust_words: None,
    },
    SmokeFixture {
        name: "cpuid",
        guest_bytes: &[0x0F, 0xA2],
        rust_words: None,
    },
    SmokeFixture {
        name: "imul_r64_rm64_reg",
        guest_bytes: &[0x48, 0x0F, 0xAF, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "imul_r64_rm64_imm8",
        guest_bytes: &[0x48, 0x6B, 0xC1, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_moffs_to_rax",
        guest_bytes: &[0x48, 0xA1, 0, 0, 0, 0, 0, 0, 0, 0],
        rust_words: None,
    },
    SmokeFixture {
        name: "xchg_acc_rcx",
        guest_bytes: &[0x48, 0x91],
        rust_words: None,
    },
    SmokeFixture {
        name: "mov_r8_imm8",
        guest_bytes: &[0xB1, 0x7F],
        rust_words: None,
    },
    SmokeFixture {
        name: "cdqe",
        guest_bytes: &[0x48, 0x98],
        rust_words: None,
    },
    SmokeFixture {
        name: "pop_rm64_reg",
        guest_bytes: &[0x8F, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "cqo",
        guest_bytes: &[0x48, 0x99],
        rust_words: None,
    },
    SmokeFixture {
        name: "or_r64_rm64",
        guest_bytes: &[0x48, 0x0B, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "test_rm8_r8",
        guest_bytes: &[0x84, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "leave",
        guest_bytes: &[0xC9],
        rust_words: None,
    },
    SmokeFixture {
        name: "lea_r32_m",
        guest_bytes: &[0x8D, 0x41, 0x08],
        rust_words: None,
    },
    SmokeFixture {
        name: "enter_level0",
        guest_bytes: &[0xC8, 0x20, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "jrcxz_rel8",
        guest_bytes: &[0xE3, 0x02],
        rust_words: None,
    },
    SmokeFixture {
        name: "loop_rel8",
        guest_bytes: &[0xE2, 0xFE],
        rust_words: None,
    },
    SmokeFixture {
        name: "movzx_r32_rm8_reg",
        guest_bytes: &[0x0F, 0xB6, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "movsxd_r32_rm32_reg",
        guest_bytes: &[0x63, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "arpl_invalid",
        guest_bytes: &[0x66, 0x63, 0xC1],
        rust_words: None,
    },
    SmokeFixture {
        name: "arpl_invalid_mem",
        guest_bytes: &[0x66, 0x63, 0x40, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "sbb_acc_imm8_placeholder",
        guest_bytes: &[0x1C, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "adc_rm64_r64_placeholder",
        guest_bytes: &[0x48, 0x11, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "xadd_rm64_r64_placeholder",
        guest_bytes: &[0x48, 0x0F, 0xC1, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "movnti_m64_r64",
        guest_bytes: &[0x48, 0x0F, 0xC3, 0x08],
        rust_words: None,
    },
    SmokeFixture {
        name: "group4_inc_rm8_placeholder",
        guest_bytes: &[0xFE, 0xC0],
        rust_words: None,
    },
    SmokeFixture {
        name: "xchg_rm8_r8_reg",
        guest_bytes: &[0x86, 0xC8],
        rust_words: None,
    },
    SmokeFixture {
        name: "xlat",
        guest_bytes: &[0xD7],
        rust_words: None,
    },
    SmokeFixture {
        name: "fnop",
        guest_bytes: &[0xD9, 0xD0],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "fnclex",
        guest_bytes: &[0xDB, 0xE2],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "fclex",
        guest_bytes: &[0x9B, 0xDB, 0xE2],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "fninit",
        guest_bytes: &[0xDB, 0xE3],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "finit",
        guest_bytes: &[0x9B, 0xDB, 0xE3],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "wait_fnop",
        guest_bytes: &[0x9B, 0xD9, 0xD0],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "cld",
        guest_bytes: &[0xFC],
        rust_words: Some(&[]),
    },
    SmokeFixture {
        name: "lodsb",
        guest_bytes: &[0xAC],
        rust_words: None,
    },
    SmokeFixture {
        name: "stosb",
        guest_bytes: &[0xAA],
        rust_words: None,
    },
    SmokeFixture {
        name: "scasb",
        guest_bytes: &[0xAE],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmpsb",
        guest_bytes: &[0xA6],
        rust_words: None,
    },
    SmokeFixture {
        name: "movsb",
        guest_bytes: &[0xA4],
        rust_words: None,
    },
    SmokeFixture {
        name: "movsq",
        guest_bytes: &[0x48, 0xA5],
        rust_words: None,
    },
    SmokeFixture {
        name: "lodsw",
        guest_bytes: &[0x66, 0xAD],
        rust_words: None,
    },
];

#[test]
fn rust_smoke_translator_matches_pinned_backend_bytes() {
    let mut translator = RustSmokeTranslator::new();

    for fixture in FIXTURES {
        let expected = match fixture.rust_words {
            Some(words) => Some(words_to_le_bytes(words)),
            None => None,
        };
        let is_nop = fixture.guest_bytes == &[0x90];
        let translated = translator
            .translate(0x1000, fixture.guest_bytes)
            .expect("rust translator should emit bytes");
        if expected.is_none() {
            if is_nop {
                assert_eq!(
                    translated,
                    Vec::new(),
                    "{}: nop should lower to empty code",
                    fixture.name
                );
            } else {
                assert!(
                    !translated.is_empty(),
                    "{}: Rust emitted no code",
                    fixture.name
                );
            }
        }
        match expected {
            Some(expected_bytes) => assert_eq!(translated, expected_bytes, "{}", fixture.name),
            None => {}
        }
    }
}

#[test]
fn rust_smoke_translator_cond_jump_rel_matrix() {
    let mut translator = RustSmokeTranslator::new();

    for &(opcode, cond_imm) in COND_JUMP_REL8_MATRIX {
        let forward = [opcode, 0x02];
        assert_eq!(
            translator.translate(0x1000, &forward),
            Some(words_to_le_bytes(&[0x5400_0040 | cond_imm, 0x1400_0001,])),
            "cond rel8 opcode {opcode:#x} forward should emit expected branch immediates",
        );

        let backward = [opcode, 0xFE];
        assert_eq!(
            translator.translate(0x1000, &backward),
            Some(words_to_le_bytes(&[0x5400_0000 | cond_imm, 0x1400_0001,])),
            "cond rel8 opcode {opcode:#x} backward should emit expected branch immediates",
        );
    }

    for &(opcode, cond_imm) in COND_JUMP_REL32_MATRIX {
        let forward = [0x0F, opcode, 0x02, 0x00, 0x00, 0x00];
        assert_eq!(
            translator.translate(0x1000, &forward),
            Some(words_to_le_bytes(&[0x5400_0040 | cond_imm, 0x1400_0001,])),
            "cond rel32 opcode {opcode:#x} forward should emit expected branch immediates",
        );

        let backward = [0x0F, opcode, 0xFA, 0xFF, 0xFF, 0xFF];
        assert_eq!(
            translator.translate(0x1000, &backward),
            Some(words_to_le_bytes(&[0x5400_0000 | cond_imm, 0x1400_0001,])),
            "cond rel32 opcode {opcode:#x} self-target should emit expected branch immediates",
        );
    }

    assert!(translator.translate(0x1000, &[0x7A, 0x00]).is_none());
    assert!(translator.translate(0x1000, &[0x7B, 0x00]).is_none());
    assert!(translator
        .translate(0x1000, &[0x0F, 0x8A, 0x00, 0x00, 0x00, 0x00])
        .is_none());
    assert!(translator
        .translate(0x1000, &[0x0F, 0x8B, 0x00, 0x00, 0x00, 0x00])
        .is_none());
}

fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}
