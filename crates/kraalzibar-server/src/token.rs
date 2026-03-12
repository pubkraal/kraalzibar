use kraalzibar_core::tuple::SnapshotToken;

use crate::error::ApiError;

static OBFUSCATION_KEY: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

fn get_key() -> u64 {
    *OBFUSCATION_KEY.get_or_init(rand::random::<u64>)
}

pub fn encode_snapshot(token: SnapshotToken) -> String {
    let obfuscated = token.value() ^ get_key();
    format!("{obfuscated:016x}")
}

pub fn decode_snapshot(encoded: &str) -> Result<SnapshotToken, ApiError> {
    let obfuscated = u64::from_str_radix(encoded, 16).map_err(|_| ApiError::InvalidToken)?;
    let value = obfuscated ^ get_key();

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
}
