//! Validated scatter/gather for `readv`/`writev`.
//!
//! A vectored I/O syscall hands the kernel a guest pointer to an array of
//! `iovec`s, each naming a guest buffer. Before the host touches any of those
//! buffers it must check the array is fully readable AND every buffer it
//! describes lies in mapped (and, for a read INTO guest memory, writable) guest
//! memory — otherwise a malformed `iovec` would make the host read or write out
//! of bounds. This composes [`Iovec`] decoding with [`AddressSpace::validate_range`]
//! into that check.

use prisma_orchestrator::address_space::{AddressSpace, RangeError};
use prisma_runtime::guest_structs::Iovec;

/// Why an `iovec` array failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoVecError {
    /// The `iovec` array does not fit in the bytes read from guest memory.
    TruncatedArray,
    /// `count * sizeof(iovec)` overflows.
    CountOverflow,
    /// The buffer named by the `iovec` at `index` failed range validation.
    BadBuffer { index: usize, err: RangeError },
}

/// Decode and validate `count` guest `iovec`s from `array_bytes`, checking each
/// buffer lies in mapped guest memory. `need_write` is `true` when the syscall
/// writes INTO the guest buffers (`readv`), `false` when it reads from them
/// (`writev`). Returns the validated `iovec`s for the caller to gather/scatter.
///
/// `array_bytes` is the guest memory already read at the array pointer; the
/// caller is responsible for having validated that pointer's own range first.
///
/// # Errors
/// [`IoVecError::CountOverflow`] / [`IoVecError::TruncatedArray`] if the array
/// does not fit, or [`IoVecError::BadBuffer`] for the first buffer that is not
/// mapped (or not writable when `need_write`).
pub fn validate_iovec_array(
    space: &AddressSpace,
    array_bytes: &[u8],
    count: usize,
    need_write: bool,
) -> Result<Vec<Iovec>, IoVecError> {
    let need = count
        .checked_mul(Iovec::SIZE)
        .ok_or(IoVecError::CountOverflow)?;
    let raw = array_bytes.get(..need).ok_or(IoVecError::TruncatedArray)?;

    let mut out = Vec::with_capacity(count);
    for index in 0..count {
        let off = index * Iovec::SIZE;
        let iov = Iovec::from_guest_bytes(&raw[off..]).ok_or(IoVecError::TruncatedArray)?;
        space
            .validate_range(iov.base, iov.len, need_write)
            .map_err(|err| IoVecError::BadBuffer { index, err })?;
        out.push(iov);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{validate_iovec_array, IoVecError};
    use prisma_orchestrator::address_space::{AddressSpace, Protection, RangeError};
    use prisma_runtime::guest_structs::Iovec;

    fn space() -> AddressSpace {
        let mut s = AddressSpace::new();
        s.map(0x3000, 0x2000, Protection::ReadWrite, "data")
            .unwrap();
        s
    }

    /// Encode an array of iovecs to the wire form a guest would pass.
    fn array(iovs: &[Iovec]) -> Vec<u8> {
        iovs.iter().flat_map(|iv| iv.to_guest_bytes()).collect()
    }

    #[test]
    fn validates_in_bounds_buffers() {
        let s = space();
        let iovs = [
            Iovec {
                base: 0x3000,
                len: 0x100,
            },
            Iovec {
                base: 0x3100,
                len: 0x200,
            },
        ];
        let bytes = array(&iovs);
        let got = validate_iovec_array(&s, &bytes, 2, true).expect("all in bounds");
        assert_eq!(got, iovs);
    }

    #[test]
    fn rejects_a_buffer_outside_mapped_memory() {
        let s = space();
        let iovs = [
            Iovec {
                base: 0x3000,
                len: 0x100,
            },
            Iovec {
                base: 0x9000,
                len: 0x10,
            }, // unmapped
        ];
        let bytes = array(&iovs);
        assert_eq!(
            validate_iovec_array(&s, &bytes, 2, false),
            Err(IoVecError::BadBuffer {
                index: 1,
                err: RangeError::Unmapped
            })
        );
    }

    #[test]
    fn rejects_write_into_read_only_buffer() {
        let mut s = AddressSpace::new();
        s.map(0x3000, 0x1000, Protection::ReadOnly, "ro").unwrap();
        let bytes = array(&[Iovec {
            base: 0x3000,
            len: 0x10,
        }]);
        // writev (read from guest) is fine; readv (write into guest) is not.
        assert!(validate_iovec_array(&s, &bytes, 1, false).is_ok());
        assert_eq!(
            validate_iovec_array(&s, &bytes, 1, true),
            Err(IoVecError::BadBuffer {
                index: 0,
                err: RangeError::NotWritable
            })
        );
    }

    #[test]
    fn rejects_truncated_array() {
        let s = space();
        let bytes = array(&[Iovec {
            base: 0x3000,
            len: 0x10,
        }]); // one iovec
             // Asking for two reads past the buffer.
        assert_eq!(
            validate_iovec_array(&s, &bytes, 2, false),
            Err(IoVecError::TruncatedArray)
        );
    }

    #[test]
    fn zero_count_is_ok_and_empty() {
        let s = space();
        assert_eq!(validate_iovec_array(&s, &[], 0, true), Ok(Vec::new()));
    }
}
