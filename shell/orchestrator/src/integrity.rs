//! sha256 verification of downloaded artefacts (Wine bundles,
//! DXVK / VKD3D binaries, future translation cache slabs).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sha256Hash(pub [u8; 32]);

impl Sha256Hash {
    #[must_use]
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(buf);
        let out = h.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
        Self(arr)
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::with_capacity(64);
        for b in self.0 {
            write!(s, "{b:02x}").expect("writing to String cannot fail");
        }
        s
    }

    pub fn from_hex(hex: &str) -> Result<Self, IntegrityError> {
        if hex.len() != 64 {
            return Err(IntegrityError::BadHexLength(hex.len()));
        }
        let mut arr = [0u8; 32];
        for (i, byte_pair) in hex.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(byte_pair).map_err(|_| IntegrityError::BadHexChar)?;
            arr[i] = u8::from_str_radix(s, 16).map_err(|_| IntegrityError::BadHexChar)?;
        }
        Ok(Self(arr))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntegrityError {
    #[error("hex digest must be 64 chars long, got {0}")]
    BadHexLength(usize),
    #[error("hex digest contains non-hex characters")]
    BadHexChar,
    #[error("hash mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
}

/// Verify that `bytes` hashes to `expected`.
pub fn verify(bytes: &[u8], expected: &Sha256Hash) -> Result<(), IntegrityError> {
    let actual = Sha256Hash::from_bytes(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch {
            expected: expected.to_hex(),
            actual: actual.to_hex(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_hashes_to_known_constant() {
        let h = Sha256Hash::from_bytes(b"");
        assert_eq!(
            h.to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn round_trip_hex() {
        let h = Sha256Hash::from_bytes(b"prisma");
        let hex = h.to_hex();
        let restored = Sha256Hash::from_hex(&hex).unwrap();
        assert_eq!(h, restored);
    }

    #[test]
    fn verify_accepts_matching_digest() {
        let buf = b"prisma-test-payload";
        let h = Sha256Hash::from_bytes(buf);
        assert!(verify(buf, &h).is_ok());
    }

    #[test]
    fn verify_rejects_mismatch() {
        let buf = b"prisma-test-payload";
        let wrong = Sha256Hash::from_bytes(b"different");
        assert!(matches!(
            verify(buf, &wrong),
            Err(IntegrityError::Mismatch { .. })
        ));
    }

    #[test]
    fn from_hex_rejects_short_input() {
        assert_eq!(
            Sha256Hash::from_hex("abc"),
            Err(IntegrityError::BadHexLength(3))
        );
    }

    #[test]
    fn from_hex_rejects_non_hex() {
        let bad = "z".repeat(64);
        assert_eq!(Sha256Hash::from_hex(&bad), Err(IntegrityError::BadHexChar));
    }
}
