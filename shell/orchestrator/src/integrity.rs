//! sha256 verification of downloaded artefacts (Wine bundles,
//! DXVK / VKD3D binaries, future translation cache slabs).

use std::io::Read;
use std::path::Path;

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

    /// Hash a file by streaming it in fixed-size chunks.
    ///
    /// Reads through an 8 KiB window, so memory stays constant no matter how
    /// large the artefact is — a downloaded Wine/DXVK bundle can be hundreds of
    /// MB, and must never be slurped whole into RAM just to be hashed.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, IntegrityError> {
        let mut f = std::fs::File::open(path).map_err(|e| IntegrityError::Io(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut window = [0u8; 8 * 1024];
        loop {
            let n = f
                .read(&mut window)
                .map_err(|e| IntegrityError::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&window[..n]);
        }
        let out = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
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
    #[error("I/O error reading artefact: {0}")]
    Io(String),
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

/// Verify a file on disk against `expected`, streaming it in bounded memory.
///
/// The download path for components (Wine/DXVK/VKD3D bundles): hash the file
/// without loading it whole, then compare. Errors on I/O failure or mismatch.
pub fn verify_file<P: AsRef<Path>>(path: P, expected: &Sha256Hash) -> Result<(), IntegrityError> {
    let actual = Sha256Hash::from_file(path)?;
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

    #[test]
    fn from_file_matches_from_bytes() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"prisma-artefact").unwrap();
        assert_eq!(
            Sha256Hash::from_file(tmp.path()).unwrap(),
            Sha256Hash::from_bytes(b"prisma-artefact")
        );
    }

    #[test]
    fn from_file_streams_across_chunk_boundaries() {
        // > 64 KiB so the read loop runs several windows; the streamed hash
        // must equal the one-shot hash of the same bytes (the property that a
        // chunk-boundary bug would break).
        let big: Vec<u8> = b"prisma".iter().cycle().take(200_000).copied().collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &big).unwrap();
        assert_eq!(
            Sha256Hash::from_file(tmp.path()).unwrap(),
            Sha256Hash::from_bytes(&big)
        );
    }

    #[test]
    fn verify_file_accepts_and_rejects() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"bundle").unwrap();
        assert!(verify_file(tmp.path(), &Sha256Hash::from_bytes(b"bundle")).is_ok());
        assert!(matches!(
            verify_file(tmp.path(), &Sha256Hash::from_bytes(b"tampered")),
            Err(IntegrityError::Mismatch { .. })
        ));
    }

    #[test]
    fn from_file_missing_errors() {
        assert!(matches!(
            Sha256Hash::from_file("/no/such/artefact.bin"),
            Err(IntegrityError::Io(_))
        ));
    }
}
