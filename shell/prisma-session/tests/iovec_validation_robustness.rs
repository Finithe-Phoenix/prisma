//! Robustness coverage for `validate_iovec_array` (the readv/writev safety
//! preamble). A deterministic spread of bases/lengths/counts across mapped,
//! unmapped, read-only, and boundary-crossing regions, asserting the
//! composition guarantee: when validation accepts an array, every buffer it
//! returns really lies in mapped (and writable-when-asked) guest memory — and
//! it never panics on malformed input.

use prisma_orchestrator::address_space::{AddressSpace, Protection};
use prisma_runtime::guest_structs::Iovec;
use prisma_session::guest_io::validate_iovec_array;

fn space() -> AddressSpace {
    let mut s = AddressSpace::new();
    s.map(0x1000, 0x1000, Protection::ReadWrite, "rw").unwrap();
    s.map(0x4000, 0x1000, Protection::ReadOnly, "ro").unwrap();
    s
}

fn encode(iovs: &[Iovec]) -> Vec<u8> {
    iovs.iter().flat_map(|iv| iv.to_guest_bytes()).collect()
}

#[test]
fn ok_implies_every_buffer_revalidates() {
    let s = space();
    // Mapped boundaries, unmapped holes, the RO region, and a wild address.
    let bases = [0u64, 0x1000, 0x1800, 0x1ff0, 0x4000, 0x4800, 0x9999_0000];
    let lens = [0u64, 1, 0x10, 0x800, 0x1000, 0x2000];
    for need_write in [false, true] {
        for &base in &bases {
            for &len in &lens {
                let bytes = encode(&[Iovec { base, len }]);
                if let Ok(got) = validate_iovec_array(&s, &bytes, 1, need_write) {
                    assert_eq!(got.len(), 1);
                    // Composition guarantee: an accepted buffer really validates.
                    assert!(
                        s.validate_range(got[0].base, got[0].len, need_write)
                            .is_ok(),
                        "accepted iovec base={base:#x} len={len:#x} must re-validate"
                    );
                }
                // Err is fine; the point is it must never panic.
            }
        }
    }
}

#[test]
fn truncated_arrays_are_rejected_for_every_count() {
    let s = space();
    let one = encode(&[Iovec {
        base: 0x1000,
        len: 0x10,
    }]);
    // Asking for more iovecs than the bytes hold is rejected, never a panic.
    for count in 2..16 {
        assert!(validate_iovec_array(&s, &one, count, false).is_err());
    }
}

#[test]
fn multi_iovec_all_in_bounds_validates() {
    let s = space();
    let iovs = [
        Iovec {
            base: 0x1000,
            len: 0x100,
        },
        Iovec {
            base: 0x1200,
            len: 0x100,
        },
        Iovec {
            base: 0x1ff0,
            len: 0x10,
        }, // ends exactly at the region end
    ];
    let bytes = encode(&iovs);
    let got = validate_iovec_array(&s, &bytes, 3, true).expect("all in mapped RW");
    assert_eq!(got, iovs);
}

#[test]
fn zero_count_is_empty_regardless_of_bytes() {
    let s = space();
    let junk = vec![0xABu8; 40];
    assert_eq!(validate_iovec_array(&s, &junk, 0, true), Ok(Vec::new()));
    assert_eq!(validate_iovec_array(&s, &[], 0, false), Ok(Vec::new()));
}
