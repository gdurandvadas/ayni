use sha2::{Digest, Sha256};
use std::fmt::Write;

/// Encode bytes as lowercase hexadecimal without relying on a digest output's
/// formatting traits.
#[must_use]
pub fn lower_hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

/// Return the lowercase hexadecimal SHA-256 digest for the supplied bytes.
#[must_use]
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    lower_hex(Sha256::digest(bytes))
}

/// Return the canonical `sha256:<lowercase hex>` fingerprint for the supplied bytes.
#[must_use]
pub fn sha256_fingerprint(bytes: impl AsRef<[u8]>) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

#[cfg(test)]
mod tests {
    use super::{lower_hex, sha256_fingerprint, sha256_hex};

    #[test]
    fn encodes_bytes_as_lowercase_hex() {
        assert_eq!(lower_hex([0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }

    #[test]
    fn hashes_bytes_with_sha256() {
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_hex(b"abc"), digest);
        assert_eq!(sha256_fingerprint(b"abc"), format!("sha256:{digest}"));
    }
}
