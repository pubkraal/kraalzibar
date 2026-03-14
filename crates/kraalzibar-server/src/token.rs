use kraalzibar_core::tuple::SnapshotToken;

use crate::error::ApiError;

const ENCODED_TOKEN_LEN: usize = 16;

static OBFUSCATION_KEY: std::sync::OnceLock<(u64, u64)> = std::sync::OnceLock::new();

fn get_keys() -> (u64, u64) {
    *OBFUSCATION_KEY.get_or_init(|| {
        let mul = rand::random::<u64>() | 1; // ensure odd for invertibility
        let add = rand::random::<u64>();
        (mul, add)
    })
}

fn obfuscate(value: u64) -> u64 {
    let (mul, add) = get_keys();
    value.wrapping_mul(mul).wrapping_add(add)
}

fn deobfuscate(obfuscated: u64) -> u64 {
    let (mul, add) = get_keys();
    let inv = mod_inverse(mul);
    obfuscated.wrapping_sub(add).wrapping_mul(inv)
}

fn mod_inverse(a: u64) -> u64 {
    // Compute modular multiplicative inverse mod 2^64 using extended GCD.
    // Only works when `a` is odd (guaranteed by get_keys).
    let mut t: u64 = 0;
    let mut new_t: u64 = 1;
    let mut r: u128 = 1u128 << 64;
    let mut new_r: u128 = a as u128;

    while new_r != 0 {
        let quotient = r / new_r;
        let tmp_t = new_t;
        new_t = t.wrapping_sub((quotient as u64).wrapping_mul(new_t));
        t = tmp_t;
        let tmp_r = new_r;
        new_r = r - quotient * new_r;
        r = tmp_r;
    }

    t
}

pub fn encode_snapshot(token: SnapshotToken) -> String {
    let obfuscated = obfuscate(token.value());
    format!("{obfuscated:016x}")
}

pub fn decode_snapshot(encoded: &str) -> Result<SnapshotToken, ApiError> {
    if encoded.len() != ENCODED_TOKEN_LEN {
        return Err(ApiError::InvalidToken);
    }

    let obfuscated = u64::from_str_radix(encoded, 16).map_err(|_| ApiError::InvalidToken)?;
    let value = deobfuscate(obfuscated);

    Ok(SnapshotToken::new(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let original = SnapshotToken::new(42);
        let encoded = encode_snapshot(original);
        let decoded = decode_snapshot(&encoded).unwrap();

        assert_eq!(decoded, original);
    }

    #[test]
    fn encoded_token_is_not_raw_number() {
        let token = SnapshotToken::new(42);
        let encoded = encode_snapshot(token);

        assert_ne!(encoded, "42");
        assert_ne!(encoded, format!("{:016x}", 42));
    }

    #[test]
    fn encoded_token_is_hex_string() {
        let token = SnapshotToken::new(12345);
        let encoded = encode_snapshot(token);

        assert_eq!(encoded.len(), 16);
        assert!(encoded.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn decode_rejects_invalid_hex() {
        let result = decode_snapshot("not-valid-hex!!!");
        assert!(result.is_err());
    }

    #[test]
    fn decode_rejects_empty_string() {
        let result = decode_snapshot("");
        assert!(result.is_err());
    }

    #[test]
    fn round_trip_zero() {
        let original = SnapshotToken::new(0);
        let encoded = encode_snapshot(original);
        let decoded = decode_snapshot(&encoded).unwrap();

        assert_eq!(decoded, original);
    }

    #[test]
    fn round_trip_max_u64() {
        let original = SnapshotToken::new(u64::MAX);
        let encoded = encode_snapshot(original);
        let decoded = decode_snapshot(&encoded).unwrap();

        assert_eq!(decoded, original);
    }

    #[test]
    fn different_values_produce_different_encodings() {
        let a = encode_snapshot(SnapshotToken::new(1));
        let b = encode_snapshot(SnapshotToken::new(2));

        assert_ne!(a, b);
    }

    #[test]
    fn decode_rejects_wrong_length() {
        assert!(decode_snapshot("abc").is_err());
        assert!(decode_snapshot("0123456789abcdef0").is_err());
    }
}
