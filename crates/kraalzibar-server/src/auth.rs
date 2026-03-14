use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use rand::rngs::OsRng;

use crate::audit;
use kraalzibar_core::tuple::TenantId;

fn argon2_instance() -> Result<Argon2<'static>, AuthError> {
    let params = Params::new(47_104, 1, 1, None).map_err(|e| AuthError::Internal(e.to_string()))?;

    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

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
    let argon2 = argon2_instance()?;
    let hash = argon2
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(e.to_string()))?;

    Ok(hash.to_string())
}

pub fn verify_secret(secret: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| AuthError::Internal(e.to_string()))?;

    Ok(argon2_instance()?
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
    let (key_id, secret) = parse_api_key(raw_key).inspect_err(|e| {
        audit::audit_auth_failure(&e.to_string(), None);
    })?;

    let record = match lookup(key_id) {
        Some(r) => r,
        None => {
            audit::audit_auth_failure("unknown api key", Some(key_id));
            return Err(AuthError::UnknownKey);
        }
    };

    if record.revoked {
        audit::audit_auth_failure("api key revoked", Some(key_id));
        return Err(AuthError::RevokedKey);
    }

    let valid = verify_secret(secret, &record.key_hash)?;
    if !valid {
        audit::audit_auth_failure("invalid secret", Some(key_id));
        return Err(AuthError::UnknownKey);
    }

    audit::audit_auth_success(&record.tenant_id, &record.key_id);
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
    fn hash_uses_argon2id_with_owasp_params() {
        let hash = hash_secret("testsecret").unwrap();
        let parsed = PasswordHash::new(&hash).unwrap();

        assert_eq!(
            parsed.algorithm,
            argon2::Algorithm::Argon2id.ident(),
            "algorithm must be argon2id"
        );

        let m_cost = parsed.params.get_str("m").unwrap().parse::<u32>().unwrap();
        let t_cost = parsed.params.get_str("t").unwrap().parse::<u32>().unwrap();
        let p_cost = parsed.params.get_str("p").unwrap().parse::<u32>().unwrap();

        assert_eq!(m_cost, 47_104, "memory cost must be 46 MiB (OWASP)");
        assert_eq!(t_cost, 1, "time cost must be 1 (OWASP)");
        assert_eq!(p_cost, 1, "parallelism must be 1 (OWASP)");
    }

    #[test]
    fn verify_secret_accepts_hash_from_old_default_params() {
        let salt = argon2::password_hash::SaltString::generate(&mut OsRng);
        let old_argon2 = Argon2::default();
        let hash = old_argon2
            .hash_password(b"testsecret", &salt)
            .unwrap()
            .to_string();

        let parsed = PasswordHash::new(&hash).unwrap();
        let m_cost = parsed.params.get_str("m").unwrap().parse::<u32>().unwrap();
        assert_eq!(m_cost, 19_456, "fixture must use old default m_cost");

        assert!(
            verify_secret("testsecret", &hash).unwrap(),
            "verify_secret must accept hashes produced by old Argon2::default() params"
        );
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
