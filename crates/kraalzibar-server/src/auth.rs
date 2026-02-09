use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::rngs::OsRng;

use kraalzibar_core::tuple::TenantId;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing authorization header")]
    MissingHeader,

    #[error("invalid api key format")]
    InvalidKeyFormat,

    #[error("unknown api key")]
    UnknownKey,

    #[error("api key has been revoked")]
    RevokedKey,

    #[error("internal authentication error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct ApiKeyRecord {
    pub key_id: String,
    pub key_hash: String,
    pub tenant_id: TenantId,
    pub revoked: bool,
}

#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub key_id: String,
}

pub fn parse_api_key(raw_key: &str) -> Result<(&str, &str), AuthError> {
    let parts: Vec<&str> = raw_key.splitn(3, '_').collect();
    if parts.len() != 3 || parts[0] != "kraalzibar" {
        return Err(AuthError::InvalidKeyFormat);
    }
    let key_id = parts[1];
    let secret = parts[2];
    if key_id.is_empty() || secret.is_empty() {
        return Err(AuthError::InvalidKeyFormat);
    }
    Ok((key_id, secret))
}

pub fn hash_secret(secret: &str) -> Result<String, AuthError> {
    let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(e.to_string()))?;
    Ok(hash.to_string())
}

pub fn verify_secret(secret: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| AuthError::Internal(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(secret.as_bytes(), &parsed_hash)
        .is_ok())
}

pub fn generate_api_key() -> (String, String) {
    use rand::Rng;
    let key_id: String = (0..8)
        .map(|_| {
            let idx = rand::thread_rng().gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();

    let secret: String = (0..32)
        .map(|_| {
            let idx = rand::thread_rng().gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();

    let full_key = format!("kraalzibar_{key_id}_{secret}");
    (full_key, secret)
}

pub fn authenticate(
    raw_key: &str,
    lookup: impl FnOnce(&str) -> Option<ApiKeyRecord>,
) -> Result<TenantContext, AuthError> {
    let (key_id, secret) = parse_api_key(raw_key)?;

    let record = lookup(key_id).ok_or(AuthError::UnknownKey)?;

    if record.revoked {
        return Err(AuthError::RevokedKey);
    }

    let valid = verify_secret(secret, &record.key_hash)?;
    if !valid {
        return Err(AuthError::UnknownKey);
    }

    Ok(TenantContext {
        tenant_id: record.tenant_id,
        key_id: record.key_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_api_key() {
        let (key_id, secret) = parse_api_key("kraalzibar_abc123_secretvalue").unwrap();
        assert_eq!(key_id, "abc123");
        assert_eq!(secret, "secretvalue");
    }

    #[test]
    fn parse_rejects_missing_prefix() {
        let result = parse_api_key("invalid_abc_secret");
        assert!(matches!(result, Err(AuthError::InvalidKeyFormat)));
    }

    #[test]
    fn parse_rejects_too_few_parts() {
        let result = parse_api_key("kraalzibar_onlyone");
        assert!(matches!(result, Err(AuthError::InvalidKeyFormat)));
    }

    #[test]
    fn parse_rejects_empty_key_id() {
        let result = parse_api_key("kraalzibar__secret");
        assert!(matches!(result, Err(AuthError::InvalidKeyFormat)));
    }

    #[test]
    fn hash_and_verify_secret() {
        let hash = hash_secret("mysecret").unwrap();
        assert!(verify_secret("mysecret", &hash).unwrap());
        assert!(!verify_secret("wrongsecret", &hash).unwrap());
    }

    #[test]
    fn generate_key_has_correct_format() {
        let (full_key, _secret) = generate_api_key();
        assert!(full_key.starts_with("kraalzibar_"));
        let (_key_id, _secret) = parse_api_key(&full_key).unwrap();
    }

    #[test]
    fn authenticate_succeeds_with_valid_key() {
        let (full_key, secret) = generate_api_key();
        let (key_id, _) = parse_api_key(&full_key).unwrap();
        let key_hash = hash_secret(&secret).unwrap();

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        let record = ApiKeyRecord {
            key_id: key_id.to_string(),
            key_hash,
            tenant_id: tenant_id.clone(),
            revoked: false,
        };

        let ctx = authenticate(&full_key, |_| Some(record)).unwrap();
        assert_eq!(ctx.tenant_id, tenant_id);
    }

    #[test]
    fn authenticate_rejects_revoked_key() {
        let (full_key, secret) = generate_api_key();
        let (key_id, _) = parse_api_key(&full_key).unwrap();
        let key_hash = hash_secret(&secret).unwrap();

        let record = ApiKeyRecord {
            key_id: key_id.to_string(),
            key_hash,
            tenant_id: TenantId::new(uuid::Uuid::new_v4()),
            revoked: true,
        };

        let result = authenticate(&full_key, |_| Some(record));
        assert!(matches!(result, Err(AuthError::RevokedKey)));
    }

    #[test]
    fn authenticate_rejects_unknown_key() {
        let result = authenticate("kraalzibar_unknown_secret", |_| None);
        assert!(matches!(result, Err(AuthError::UnknownKey)));
    }

    #[test]
    fn authenticate_rejects_wrong_secret() {
        let (_full_key, _) = generate_api_key();
        let key_hash = hash_secret("correct_secret").unwrap();

        let record = ApiKeyRecord {
            key_id: "testid".to_string(),
            key_hash,
            tenant_id: TenantId::new(uuid::Uuid::new_v4()),
            revoked: false,
        };

        let result = authenticate("kraalzibar_testid_wrong_secret", |_| Some(record));
        assert!(matches!(result, Err(AuthError::UnknownKey)));
    }
}
