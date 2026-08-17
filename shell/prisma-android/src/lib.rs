//! Android JNI composition root for real Prisma guest execution.
//!
//! The execution probe is deliberately small and freestanding: it is a real
//! PE32+ x86-64 image whose instructions pass through the production loader,
//! translator, ARM64 backend, JIT allocator, and session loop. On an AArch64
//! Android runtime the emitted ARM64 code must execute and terminate with the
//! expected guest exit status. On every other architecture it reports
//! `UNAVAILABLE` instead of fabricating a successful result.

use prisma_orchestrator::module_table::ModuleTable;
use prisma_session::{RunOutcome, Session};
use sha2::{Digest, Sha256};

const IMAGE_BASE: u64 = 0x1_4000_0000;
const ENTRY_RVA: u32 = 0x1000;
const EXPECTED_EXIT: i32 = 42;

/// Run the real x86-64 -> ARM64 execution probe and return a stable evidence
/// record suitable for presentation by the Android app.
#[must_use]
pub fn run_execution_probe() -> String {
    let pe = probe_pe();
    let sha256 = hex_sha256(&pe);
    let mut session = match Session::load(&pe, &ModuleTable::new()) {
        Ok(session) => session,
        Err(error) => {
            return format!(
                "FAILED|stage=load|guest=x86_64-pe|host={}|sha256={sha256}|error={error:?}",
                std::env::consts::ARCH,
            );
        }
    };
    let entry_pc = session.entry_pc();
    let translation = match session.translate_at(entry_pc) {
        Some(block) => block,
        None => {
            return format!(
                "FAILED|stage=translate|guest=x86_64-pe|host={}|sha256={sha256}",
                std::env::consts::ARCH,
            );
        }
    };
    let arm64_bytes = translation.code.len();
    let guest_bytes = translation.guest_bytes;
    let mut prepared = match session.prepare(IMAGE_BASE + 0x2_0000, &[b"prisma-probe.exe"], &[]) {
        Ok(prepared) => prepared,
        Err(error) => {
            return format!(
                "FAILED|stage=prepare|guest=x86_64-pe|host={}|sha256={sha256}|error={error:?}",
                std::env::consts::ARCH,
            );
        }
    };

    match session.run(
        &mut prepared.ctx,
        &mut prepared.mem,
        &mut prepared.state,
        8,
    ) {
        RunOutcome::Exited(EXPECTED_EXIT) => format!(
            "REAL|guest=x86_64-pe|host={}|entry=0x{entry_pc:x}|guest_bytes={guest_bytes}|arm64_bytes={arm64_bytes}|exit={EXPECTED_EXIT}|sha256={sha256}",
            std::env::consts::ARCH,
        ),
        RunOutcome::ExecUnavailable(error) => format!(
            "UNAVAILABLE|guest=x86_64-pe|host={}|entry=0x{entry_pc:x}|guest_bytes={guest_bytes}|arm64_bytes={arm64_bytes}|sha256={sha256}|error={error:?}",
            std::env::consts::ARCH,
        ),
        outcome => format!(
            "FAILED|stage=execute|guest=x86_64-pe|host={}|entry=0x{entry_pc:x}|guest_bytes={guest_bytes}|arm64_bytes={arm64_bytes}|sha256={sha256}|outcome={outcome:?}",
            std::env::consts::ARCH,
        ),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// Minimal PE32+ containing `mov eax, 231; mov edi, 42; syscall`.
fn probe_pe() -> Vec<u8> {
    let code = [
        0xB8, 0xE7, 0x00, 0x00, 0x00, // mov eax, 231 (exit_group)
        0xBF, 0x2A, 0x00, 0x00, 0x00, // mov edi, 42
        0x0F, 0x05, // syscall
    ];
    let mut image = vec![0_u8; 64 + 4 + 20 + 240 + 40];
    image[0] = b'M';
    image[1] = b'Z';
    image[0x3C..0x40].copy_from_slice(&64_u32.to_le_bytes());
    image[64..68].copy_from_slice(b"PE\0\0");

    let coff = 68;
    image[coff..coff + 2].copy_from_slice(&0x8664_u16.to_le_bytes());
    image[coff + 2..coff + 4].copy_from_slice(&1_u16.to_le_bytes());
    image[coff + 16..coff + 18].copy_from_slice(&240_u16.to_le_bytes());

    let optional = coff + 20;
    image[optional..optional + 2].copy_from_slice(&0x020B_u16.to_le_bytes());
    image[optional + 16..optional + 20].copy_from_slice(&ENTRY_RVA.to_le_bytes());
    image[optional + 24..optional + 32].copy_from_slice(&IMAGE_BASE.to_le_bytes());
    image[optional + 56..optional + 60].copy_from_slice(&0x1_0000_u32.to_le_bytes());

    let section = optional + 240;
    image[section..section + 5].copy_from_slice(b".text");
    image[section + 8..section + 12].copy_from_slice(&0x1000_u32.to_le_bytes());
    image[section + 12..section + 16].copy_from_slice(&ENTRY_RVA.to_le_bytes());
    image[section + 16..section + 20]
        .copy_from_slice(&u32::try_from(code.len()).unwrap_or(0).to_le_bytes());
    let raw_offset = u32::try_from(image.len()).unwrap_or(0);
    image[section + 20..section + 24].copy_from_slice(&raw_offset.to_le_bytes());
    image.extend_from_slice(&code);
    image
}

#[cfg(target_os = "android")]
mod jni_api {
    use jni::objects::JClass;
    use jni::sys::jstring;
    use jni::JNIEnv;

    /// JNI entry point used by `OrchestratorJni.runExecutionProbe()`.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_prismaemu_app_OrchestratorJni_runExecutionProbe(
        env: JNIEnv,
        _class: JClass,
    ) -> jstring {
        match env.new_string(super::run_execution_probe()) {
            Ok(value) => value.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_real_on_arm64_and_explicitly_unavailable_elsewhere() {
        let report = run_execution_probe();
        if cfg!(target_arch = "aarch64") {
            assert!(report.starts_with("REAL|"), "{report}");
            assert!(report.contains("exit=42"), "{report}");
        } else {
            assert!(report.starts_with("UNAVAILABLE|"), "{report}");
        }
        assert!(report.contains("guest=x86_64-pe"), "{report}");
        assert!(report.contains("arm64_bytes="), "{report}");
        assert!(report.contains("sha256="), "{report}");
    }
}
