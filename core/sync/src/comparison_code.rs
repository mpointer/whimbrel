//! Short code derived from a device's public key, shown on both the pending
//! device and the approving device so the user can check they match
//! (docs/15-mobile-app-design.md §5.1).

use base64::{engine::general_purpose::STANDARD, Engine as _};
use sha2::{Digest, Sha256};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// `"K7 · 4Q · 92 · 9K · 3M"`-style code: the first 50 bits of SHA-256 over the
/// raw 32-byte public key, as ten Crockford base32 characters grouped 2·2·2·2·2
/// (50 bits, not 30: a 30-bit code is cheap to grind a colliding key for).
/// `None` for invalid base64 or a key that is not exactly 32 bytes.
pub fn device_comparison_code(public_key_b64: &str) -> Option<String> {
    let key = STANDARD.decode(public_key_b64).ok().filter(|k| k.len() == 32)?;
    let h = Sha256::digest(&key);
    let bits = u64::from_be_bytes([0, h[0], h[1], h[2], h[3], h[4], h[5], h[6]]) >> 6;
    let c = |i: u32| CROCKFORD[((bits >> (5 * (9 - i))) & 31) as usize] as char;
    Some(format!("{}{} · {}{} · {}{} · {}{} · {}{}", c(0), c(1), c(2), c(3), c(4), c(5), c(6), c(7), c(8), c(9)))
}

#[cfg(test)]
mod tests {
    use super::device_comparison_code;
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    fn b64(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    #[test]
    fn fixed_key_gives_pinned_code() {
        // Key = bytes 0..32. sha256 = 630dcd2966c433..., first 50 bits -> CC 6W TA B6 RG
        // (computed independently with shasum + python, see commit message).
        let key: Vec<u8> = (0u8..32).collect();
        assert_eq!(device_comparison_code(&b64(&key)).as_deref(), Some("CC · 6W · TA · B6 · RG"));
    }

    #[test]
    fn different_keys_differ() {
        assert_ne!(device_comparison_code(&b64(&[1u8; 32])), device_comparison_code(&b64(&[2u8; 32])));
    }

    #[test]
    fn invalid_input_is_none() {
        assert_eq!(device_comparison_code(""), None);
        assert_eq!(device_comparison_code("not base64!"), None);
        assert_eq!(device_comparison_code(&b64(&[7u8; 31])), None);
        assert_eq!(device_comparison_code(&b64(&[7u8; 33])), None);
    }

    #[test]
    fn output_is_five_groups_of_two() {
        let code = device_comparison_code(&b64(&[9u8; 32])).unwrap();
        let groups: Vec<&str> = code.split(" · ").collect();
        assert_eq!(groups.len(), 5);
        assert!(groups.iter().all(|g| g.len() == 2));
    }
}
