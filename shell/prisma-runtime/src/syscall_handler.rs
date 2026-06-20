//! Guest syscall handling: policy, classification, and typed dispatch.
//!
//! The runtime intercepts guest x86-64 Linux syscalls. This module owns the
//! security policy (deny-by-default) and the static registry that classifies
//! the common syscalls into categories the eventual real handlers plug into.
//! Actual syscall execution (real I/O, mmap, TLS) is implemented incrementally
//! on top of this dispatch layer in `core/src/runtime/syscall_handler.cpp`.

/// Error returned by a syscall dispatch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    /// The syscall number is not modelled / not permitted by policy.
    Unsupported(u32),
    /// A pointer argument referenced an invalid guest address.
    InvalidAddress,
    /// Policy denied an otherwise-known syscall.
    PermissionDenied,
    /// The syscall was interrupted (EINTR-equivalent).
    Interrupted,
}

/// Category of a guest syscall, used by policy and future real handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallClass {
    /// read/write/open/close/stat...
    Io,
    /// mmap/munmap/mprotect/brk...
    Memory,
    /// exit/exit_group/getpid/...
    Process,
    /// rt_sigaction/rt_sigprocmask/rt_sigreturn...
    Signal,
    /// arch_prctl/set_tid_address/futex/clone...
    Thread,
    /// Not present in the registry.
    Unknown,
}

/// Classify a x86-64 Linux syscall number. Numbers follow the
/// `arch/x86/entry/syscalls/syscall_64.tbl` ABI.
#[must_use]
pub const fn classify(number: u32) -> SyscallClass {
    match number {
        // I/O
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 8 | 16 | 17 | 18 | 19 | 20 | 32 | 33 | 72 | 257 | 262 => {
            SyscallClass::Io
        }
        // Memory
        9 | 10 | 11 | 12 | 25 | 28 => SyscallClass::Memory,
        // Process lifecycle / identity: getpid, gettid, exit, getppid,
        // getpgrp, exit_group, wait4.
        39 | 60 | 61 | 110 | 111 | 186 | 231 => SyscallClass::Process,
        // Signals
        13 | 14 | 15 | 127 | 128 => SyscallClass::Signal,
        // Threading / TLS / futex
        56 | 58 | 158 | 202 | 218 | 273 => SyscallClass::Thread,
        _ => SyscallClass::Unknown,
    }
}

/// Whether a syscall number is present in the registry (i.e. modelled).
#[must_use]
pub const fn is_known(number: u32) -> bool {
    !matches!(classify(number), SyscallClass::Unknown)
}

/// Small policy for syscall handling decisions.
#[derive(Debug, Default, Clone, Copy)]
pub struct SyscallPolicy {
    /// Signals whether unknown syscalls are denied by default.
    deny_unknown: bool,
}

impl SyscallPolicy {
    /// Creates a deny-by-default policy.
    #[must_use]
    pub const fn deny_by_default() -> Self {
        Self { deny_unknown: true }
    }

    /// Creates an allow-by-default policy.
    #[must_use]
    pub const fn allow_by_default() -> Self {
        Self {
            deny_unknown: false,
        }
    }

    /// Returns `true` if unknown syscall numbers should be denied.
    #[must_use]
    pub const fn deny_unknown(&self) -> bool {
        self.deny_unknown
    }
}

/// Minimal syscall handler facade.
#[derive(Debug, Default, Clone)]
pub struct SyscallHandler {
    policy: SyscallPolicy,
}

impl SyscallHandler {
    /// Creates a new handler using deny-by-default policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            policy: SyscallPolicy::deny_by_default(),
        }
    }

    /// Replaces the active policy.
    #[must_use]
    pub const fn with_policy(policy: SyscallPolicy) -> Self {
        Self { policy }
    }

    /// Policy currently configured for this handler.
    #[must_use]
    pub const fn policy(&self) -> SyscallPolicy {
        self.policy
    }

    /// Simulates handling of a guest syscall.
    ///
    /// Returns zero for "allowed" and -1 for "denied". Retained for the
    /// existing smoke path; new code should prefer [`dispatch`](Self::dispatch).
    #[must_use]
    pub const fn handle(&self, syscall_id: u32) -> i32 {
        if self.policy.deny_unknown && syscall_id > 0 {
            -1
        } else {
            0
        }
    }

    /// Classify and admit a syscall under the active policy.
    ///
    /// The first real handlers are the process-identity calls: `getpid` (39)
    /// and `gettid` (186) return the host process id, because the translator
    /// runs the guest *inside* the host process — that is the correct answer a
    /// guest observes (single-threaded guests have tid == pid, matching the
    /// C++ runtime). Every other modelled syscall still returns `Ok(0)` as a
    /// placeholder; unknown numbers under a deny-by-default policy are rejected.
    ///
    /// # Errors
    /// Returns [`SyscallError::Unsupported`] when the number is not modelled
    /// and the policy denies unknown syscalls.
    pub fn dispatch(&self, number: u32, _args: &[u64; 6]) -> Result<u64, SyscallError> {
        if number == 39 || number == 186 {
            return Ok(u64::from(std::process::id()));
        }
        if is_known(number) {
            Ok(0)
        } else if self.policy.deny_unknown {
            Err(SyscallError::Unsupported(number))
        } else {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{classify, is_known, SyscallClass, SyscallError, SyscallHandler, SyscallPolicy};

    #[test]
    fn deny_by_default_blocks_nonzero() {
        let h = SyscallHandler::new();
        assert_eq!(h.handle(1), -1);
        assert_eq!(h.handle(0), 0);
    }

    #[test]
    fn allow_policy_unblocks_nonzero() {
        let policy = SyscallPolicy::allow_by_default();
        let h = SyscallHandler::with_policy(policy);
        assert_eq!(h.handle(1), 0);
    }

    #[test]
    fn classification_of_common_syscalls() {
        assert_eq!(classify(0), SyscallClass::Io); // read
        assert_eq!(classify(1), SyscallClass::Io); // write
        assert_eq!(classify(9), SyscallClass::Memory); // mmap
        assert_eq!(classify(12), SyscallClass::Memory); // brk
        assert_eq!(classify(60), SyscallClass::Process); // exit
        assert_eq!(classify(231), SyscallClass::Process); // exit_group
        assert_eq!(classify(13), SyscallClass::Signal); // rt_sigaction
        assert_eq!(classify(202), SyscallClass::Thread); // futex
        assert_eq!(classify(158), SyscallClass::Thread); // arch_prctl
        assert_eq!(classify(0x0FFF_FFFF), SyscallClass::Unknown);
    }

    #[test]
    fn dispatch_admits_known_and_rejects_unknown_under_deny() {
        let h = SyscallHandler::new(); // deny-by-default
        let args = [0u64; 6];
        assert_eq!(h.dispatch(1, &args), Ok(0)); // write: modelled
        assert_eq!(
            h.dispatch(0x0FFF_FFFF, &args),
            Err(SyscallError::Unsupported(0x0FFF_FFFF))
        );
    }

    #[test]
    fn dispatch_allows_unknown_under_allow_policy() {
        let h = SyscallHandler::with_policy(SyscallPolicy::allow_by_default());
        let args = [0u64; 6];
        assert_eq!(h.dispatch(0x0FFF_FFFF, &args), Ok(0));
    }

    #[test]
    fn is_known_matches_classification() {
        assert!(is_known(0) && is_known(60) && is_known(202));
        assert!(!is_known(0x0FFF_FFFF));
    }

    #[test]
    fn gettid_is_now_classified() {
        assert_eq!(classify(186), SyscallClass::Process); // gettid
        assert!(is_known(186));
    }

    #[test]
    fn getpid_and_gettid_return_the_host_pid() {
        let h = SyscallHandler::new();
        let args = [0u64; 6];
        let pid = u64::from(std::process::id());
        assert_eq!(h.dispatch(39, &args), Ok(pid)); // getpid
        assert_eq!(h.dispatch(186, &args), Ok(pid)); // gettid (== pid, single-threaded)
        assert_ne!(pid, 0); // a real pid, not the placeholder
    }
}
