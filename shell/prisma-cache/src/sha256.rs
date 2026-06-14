//! SHA-256 helpers for the Rust cache crate.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// SHA-256 digest type, 32 bytes in big-endian order.
pub type Sha256Hash = [u8; 32];

#[must_use]
pub fn sha256(bytes: &[u8]) -> Sha256Hash {
    Sha256::digest(bytes).into()
}

#[must_use]
pub fn to_hex(digest: Sha256Hash) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[must_use]
pub fn from_hex(value: &str) -> Option<Sha256Hash> {
    if value.len() != 64 {
        return None;
    }

    let mut out = [0u8; 32];
    for (i, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

const fn hex_value(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        let digest = sha256(b"prisma");
        assert_eq!(to_hex(digest).len(), std::mem::size_of::<Sha256Hash>() * 2);
        let parsed = from_hex(&to_hex(digest)).unwrap();
        assert_eq!(digest, parsed);
    }
}
