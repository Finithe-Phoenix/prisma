//! Guest `EXCEPTION_RECORD` construction (x64 SEH delivery payload).
//!
//! When a host fault is turned into a guest exception ([`crate::guest_exception`]
//! maps the code), Windows SEH delivers it through an `EXCEPTION_RECORD`
//! structure placed in guest memory: the dispatcher reads `ExceptionCode`,
//! `ExceptionAddress`, the continuable flag and up to 15 information words from
//! it. This builds that structure's exact 64-bit on-stack byte layout so the
//! guest sees a record byte-identical to what real ntdll would hand it.

use crate::guest_exception::HostFault;

/// `EXCEPTION_NONCONTINUABLE` — set when execution cannot resume past the fault.
pub const EXCEPTION_NONCONTINUABLE: u32 = 0x0000_0001;

/// Max parameters carried in `ExceptionInformation` (the x64 ABI fixes 15).
pub const EXCEPTION_MAXIMUM_PARAMETERS: usize = 15;

/// Serialized size of an x64 `EXCEPTION_RECORD`.
///
/// Code(4) + Flags(4) + RecordPtr(8) + Address(8) + NumParams(4) + pad(4) +
/// Information(15*8) = 152 bytes.
pub const EXCEPTION_RECORD_SIZE: usize = 152;

/// An x64 `EXCEPTION_RECORD`, in host-native form before serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionRecord {
    pub code: u32,
    pub flags: u32,
    /// Guest VA of a chained record (nested exceptions); 0 if none.
    pub record: u64,
    /// Guest PC at which the fault occurred.
    pub address: u64,
    pub information: Vec<u64>,
}

impl ExceptionRecord {
    /// Build a record for `fault` at guest PC `address`, with `information`
    /// parameters (e.g. the faulting access address for an access violation).
    /// The continuable flag is derived from the NTSTATUS severity. Parameters
    /// beyond the ABI's 15 are dropped (a real record cannot carry more).
    #[must_use]
    pub fn from_fault(fault: HostFault, address: u64, information: &[u64]) -> Self {
        let flags = if fault.is_fatal() {
            EXCEPTION_NONCONTINUABLE
        } else {
            0
        };
        let count = information.len().min(EXCEPTION_MAXIMUM_PARAMETERS);
        Self {
            code: fault.guest_exception_code(),
            flags,
            record: 0,
            address,
            information: information[..count].to_vec(),
        }
    }

    /// Number of valid `ExceptionInformation` words (clamped to the ABI max).
    #[must_use]
    pub fn number_parameters(&self) -> u32 {
        // `from_fault` clamps, but a hand-built record might over-fill; clamp
        // here too so the serialized count never exceeds the 15 slots written.
        u32::try_from(self.information.len().min(EXCEPTION_MAXIMUM_PARAMETERS)).unwrap_or(0)
    }

    /// Serialize to the exact 152-byte x64 on-stack layout (little-endian).
    /// `ExceptionInformation` slots past the supplied parameters are zeroed.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; EXCEPTION_RECORD_SIZE] {
        let mut buf = [0u8; EXCEPTION_RECORD_SIZE];
        buf[0..4].copy_from_slice(&self.code.to_le_bytes());
        buf[4..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..16].copy_from_slice(&self.record.to_le_bytes());
        buf[16..24].copy_from_slice(&self.address.to_le_bytes());
        buf[24..28].copy_from_slice(&self.number_parameters().to_le_bytes());
        // bytes 28..32 are alignment padding (left zero).
        for (i, word) in self
            .information
            .iter()
            .take(EXCEPTION_MAXIMUM_PARAMETERS)
            .enumerate()
        {
            let off = 32 + i * 8;
            buf[off..off + 8].copy_from_slice(&word.to_le_bytes());
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_u32(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
    }
    fn le_u64(b: &[u8], off: usize) -> u64 {
        u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
    }

    #[test]
    fn access_violation_record_has_canonical_layout() {
        // Access violations carry [rw_flag, faulting_address] in Information.
        let rec = ExceptionRecord::from_fault(HostFault::Segv, 0x1_4000_1234, &[1, 0xDEAD_BEEF]);
        let b = rec.to_bytes();
        assert_eq!(le_u32(&b, 0), 0xC000_0005); // ExceptionCode
        assert_eq!(le_u32(&b, 4), EXCEPTION_NONCONTINUABLE); // fatal -> noncontinuable
        assert_eq!(le_u64(&b, 8), 0); // no chained record
        assert_eq!(le_u64(&b, 16), 0x1_4000_1234); // ExceptionAddress
        assert_eq!(le_u32(&b, 24), 2); // NumberParameters
        assert_eq!(le_u64(&b, 32), 1); // Information[0]
        assert_eq!(le_u64(&b, 40), 0xDEAD_BEEF); // Information[1]
        assert_eq!(le_u64(&b, 48), 0); // Information[2] zeroed
    }

    #[test]
    fn continuable_fault_clears_noncontinuable_flag() {
        // A breakpoint (0x8.. severity) is continuable.
        let rec = ExceptionRecord::from_fault(HostFault::Trap, 0x2000, &[]);
        let b = rec.to_bytes();
        assert_eq!(le_u32(&b, 0), 0x8000_0003);
        assert_eq!(le_u32(&b, 4), 0); // continuable
        assert_eq!(le_u32(&b, 24), 0); // no parameters
    }

    #[test]
    fn serialized_size_is_exactly_the_abi_record() {
        assert_eq!(
            ExceptionRecord::from_fault(HostFault::Ill, 0, &[])
                .to_bytes()
                .len(),
            152
        );
    }

    #[test]
    fn parameters_beyond_fifteen_are_dropped() {
        let many: Vec<u64> = (0..20).collect();
        let rec = ExceptionRecord::from_fault(HostFault::Segv, 0, &many);
        assert_eq!(rec.number_parameters(), 15);
        let b = rec.to_bytes();
        assert_eq!(le_u32(&b, 24), 15);
        assert_eq!(le_u64(&b, 32 + 14 * 8), 14); // last kept word
    }
}
