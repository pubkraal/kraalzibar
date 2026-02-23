use sqlx::PgPool;

use crate::traits::StorageError;

pub fn validate_schema_name(name: &str) -> Result<(), StorageError> {
    let is_valid = name.starts_with("tenant_")
        && name.len() == 39
        && name[7..].chars().all(|c| c.is_ascii_hexdigit());
    if !is_valid {
        return Err(StorageError::Internal(format!(
            "invalid tenant schema name: {name}"
        )));
    }
    Ok(())
}

pub async fn run_shared_migrations(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tenants (
            id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name        TEXT NOT NULL UNIQUE,
            pg_schema   TEXT NOT NULL UNIQUE,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS api_keys (
            id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id   UUID NOT NULL REFERENCES tenants(id),
            key_id      TEXT NOT NULL UNIQUE,
            key_hash    TEXT NOT NULL,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
            revoked_at  TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_tenant_schema(pool: &PgPool, schema_name: &str) -> Result<(), StorageError> {
    validate_schema_name(schema_name)?;

    fn to_storage_error(e: sqlx::Error) -> StorageError {
        StorageError::Internal(e.to_string())
    }

    let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {schema_name}");
    sqlx::query(&create_schema)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    let create_tuples = format!(
        r#"
        CREATE TABLE IF NOT EXISTS {schema_name}.relation_tuples (
            id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            object_type     TEXT NOT NULL,
            object_id       TEXT NOT NULL,
            relation        TEXT NOT NULL,
            subject_type    TEXT NOT NULL,
            subject_id      TEXT NOT NULL,
            subject_relation TEXT,
            created_tx_id   BIGINT NOT NULL,
            deleted_tx_id   BIGINT NOT NULL DEFAULT 9223372036854775807,
            UNIQUE NULLS NOT DISTINCT (object_type, object_id, relation, subject_type, subject_id,
                   subject_relation, deleted_tx_id)
        )
        "#
    );
    sqlx::query(&create_tuples)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    let create_lookup_idx = format!(
        r#"
        CREATE INDEX IF NOT EXISTS idx_tuples_lookup
        ON {schema_name}.relation_tuples
            (object_type, object_id, relation, deleted_tx_id)
        "#
    );
    sqlx::query(&create_lookup_idx)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    let create_reverse_idx = format!(
        r#"
        CREATE INDEX IF NOT EXISTS idx_tuples_reverse
        ON {schema_name}.relation_tuples
            (subject_type, subject_id, subject_relation, deleted_tx_id)
        "#
    );
    sqlx::query(&create_reverse_idx)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    let create_schema_defs = format!(
        r#"
        CREATE TABLE IF NOT EXISTS {schema_name}.schema_definitions (
            id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            definition  TEXT NOT NULL,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
        )
        "#
    );
    sqlx::query(&create_schema_defs)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    let create_seq = format!("CREATE SEQUENCE IF NOT EXISTS {schema_name}.tx_id_seq");
    sqlx::query(&create_seq)
        .execute(pool)
        .await
        .map_err(to_storage_error)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_schema_name_accepted() {
        let name = "tenant_00000000000000000000000000000000";
        assert!(validate_schema_name(name).is_ok());
    }

    #[test]
    fn valid_hex_schema_name_accepted() {
        let name = "tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";
        assert!(validate_schema_name(name).is_ok());
    }

    #[test]
    fn rejects_sql_injection_attempt() {
        let name = "tenant_; DROP TABLE users; --";
        assert!(validate_schema_name(name).is_err());
    }

    #[test]
    fn rejects_wrong_prefix() {
        let name = "schema_00000000000000000000000000000000";
        assert!(validate_schema_name(name).is_err());
    }

    #[test]
    fn rejects_too_short() {
        let name = "tenant_abc";
        assert!(validate_schema_name(name).is_err());
    }

    #[test]
    fn rejects_non_hex_chars() {
        let name = "tenant_0000000000000000000000000000000g";
        assert!(validate_schema_name(name).is_err());
    }
}
