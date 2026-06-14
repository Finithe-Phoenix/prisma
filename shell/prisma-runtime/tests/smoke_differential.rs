use prisma_runtime::dispatcher::{GuestTranslator, RustSmokeTranslator};

struct SmokeFixture {
    name: &'static str,
    guest_bytes: &'static [u8],
    rust_words: Option<&'static [u32]>,
}

const FIXTURES: &[SmokeFixture] = &[
    SmokeFixture {
        name: "nop",
        guest_bytes: &[0x90],
        rust_words: None,
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
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rax_imm8_i32",
        guest_bytes: &[0x83, 0xF8, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rax_imm8_i16",
        guest_bytes: &[0x66, 0x83, 0xF8, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rbx_imm8_i32",
        guest_bytes: &[0x83, 0xFB, 0x10],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rbx_imm8_i16",
        guest_bytes: &[0x66, 0x83, 0xFB, 0x10],
        rust_words: None,
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
        rust_words: None,
    },
    SmokeFixture {
        name: "cond_jump_rel8_backward",
        guest_bytes: &[0x74, 0xFE],
        rust_words: None,
    },
    SmokeFixture {
        name: "cond_jump_rel32_forward",
        guest_bytes: &[0x0F, 0x84, 0x02, 0x00, 0x00, 0x00],
        rust_words: None,
    },
    SmokeFixture {
        name: "cmp_rax_rcx_then_je_forward",
        guest_bytes: &[0x48, 0x39, 0xC8, 0x74, 0x02],
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
                assert_eq!(translated, Vec::new(), "{}: nop should lower to empty code", fixture.name);
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

fn words_to_le_bytes(words: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

