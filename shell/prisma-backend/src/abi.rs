//! ABI helpers for backend block entry/exit shims.
//!
//! This module mirrors the C++ `backend::abi` interface at a compatibility
//! level suitable for phase-5 Rust migration. It exposes stable signatures so
//! upper layers can compile while full calling-convention emission is being
//! ported.

use crate::assembler::Arm64Assembler;

/// Register used to carry the `CpuStateFrame*` across block bodies (AAPCS64 x27).
pub const K_STATE_PTR_REG: u8 = 27;

/// Number of ARM64EC-visible callee-saved register pairs preserved by the block
/// prologue.
///
/// x30 shares the final slot with the state pointer; x29 remains untouched so
/// generated code never depends on a native-only frame chain.
pub const K_CALLEE_SAVED_PAIR_COUNT: usize = 4;

const CALLEE_SAVED_PAIRS: [(u8, u8); K_CALLEE_SAVED_PAIR_COUNT] =
    [(19, 20), (21, 22), (25, 26), (K_STATE_PTR_REG, 30)];

/// Patchable epilogue metadata for direct chaining tails.
#[derive(Debug, Clone, Default)]
pub struct PatchableTailEpilogue {
    pub branch_offset: usize,
    pub fallback_offset: usize,
}

/// Emits a full block prologue.
pub fn emit_block_prologue(em: &mut Arm64Assembler) {
    for (rt, rt2) in CALLEE_SAVED_PAIRS {
        em.stp_x_pre_sp(rt, rt2, -16);
    }
    em.mov_x(K_STATE_PTR_REG, 0);
}

/// Emits a full block epilogue and return sequence.
pub fn emit_block_epilogue_and_ret(em: &mut Arm64Assembler) {
    for (rt, rt2) in CALLEE_SAVED_PAIRS.iter().rev() {
        em.ldp_x_post_sp(*rt, *rt2, 16);
    }
    em.ret();
}

/// Emits a patchable tail epilogue and returns offsets.
///
/// The Rust migration currently emits placeholders for the branch/fallback
/// offsets until real tail patch emission is implemented.
pub fn emit_block_epilogue_patchable_tail(em: &mut Arm64Assembler) -> PatchableTailEpilogue {
    let branch_offset = em.cursor_offset();
    em.b(4);
    let fallback_offset = em.cursor_offset();
    emit_block_epilogue_and_ret(em);
    PatchableTailEpilogue {
        branch_offset,
        fallback_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prologue_preserves_callee_saved_pairs_and_state_pointer() {
        let mut asm = Arm64Assembler::new();
        emit_block_prologue(&mut asm);

        assert_eq!(
            asm.finish(),
            vec![
                0xA9BF_53F3,
                0xA9BF_5BF5,
                0xA9BF_6BF9,
                0xA9BF_7BFB,
                0xAA00_03FB,
            ]
        );
    }

    #[test]
    fn epilogue_restores_pairs_in_reverse_and_returns() {
        let mut asm = Arm64Assembler::new();
        emit_block_epilogue_and_ret(&mut asm);

        assert_eq!(
            asm.finish(),
            vec![
                0xA8C1_7BFB,
                0xA8C1_6BF9,
                0xA8C1_5BF5,
                0xA8C1_53F3,
                0xD65F_03C0,
            ]
        );
    }

    #[test]
    fn arm64ec_wrapper_uses_only_admitted_registers() {
        for pair in CALLEE_SAVED_PAIRS {
            for register in <[u8; 2]>::from(pair) {
                assert!(
                    ![13, 14, 18, 23, 24, 28].contains(&register),
                    "x{register} is unavailable to generated ARM64EC code"
                );
                assert_ne!(register, 29, "x29 is reserved for native frame-pointer use");
            }
        }
        assert_eq!(CALLEE_SAVED_PAIRS.last(), Some(&(K_STATE_PTR_REG, 30)));
    }

    #[test]
    fn patchable_tail_records_branch_and_fallback_offsets() {
        let mut asm = Arm64Assembler::new();
        let tail = emit_block_epilogue_patchable_tail(&mut asm);
        let words = asm.finish();

        assert_eq!(tail.branch_offset, 0);
        assert_eq!(tail.fallback_offset, 4);
        assert_eq!(words[0], 0x1400_0001);
        assert_eq!(words.last().copied(), Some(0xD65F_03C0));
    }
}
