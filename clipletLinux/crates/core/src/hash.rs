//! `entries.content_hash`: how duplicates are recognized.
//!
//! For now this is plain SHA-256 (hex) of the content bytes. The database design doc wants an
//! HMAC-SHA256 keyed with a key derived from the space key, so every paired device computes the
//! same value; once that key exists, switch to HMAC here and recompute the hashes of local rows.

use sha2::{Digest, Sha256};

/// Incremental hasher, for content streamed to disk.
#[derive(Default, Clone)]
pub struct ContentHasher(Sha256);

impl ContentHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn finish(self) -> String {
        self.0
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut h = ContentHasher::new();
    h.update(bytes);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex() {
        assert_eq!(
            hash_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut h = ContentHasher::new();
        h.update(b"a");
        h.update(b"bc");
        assert_eq!(h.finish(), hash_bytes(b"abc"));
    }
}
