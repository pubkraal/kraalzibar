use sqlx::PgPool;

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
            key_hash    TEXT NOT NULL UNIQUE,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
            revoked_at  TIMESTAMPTZ
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_tenant_schema(pool: &PgPool, schema_name: &str) -> Result<(), sqlx::Error> {
    let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {schema_name}");
    sqlx::query(&create_schema).execute(pool).await?;

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
            UNIQUE(object_type, object_id, relation, subject_type, subject_id,
                   subject_relation, deleted_tx_id)
        )
        "#
    );
    sqlx::query(&create_tuples).execute(pool).await?;

    let create_lookup_idx = format!(
        r#"
        CREATE INDEX IF NOT EXISTS idx_tuples_lookup
        ON {schema_name}.relation_tuples
            (object_type, object_id, relation, deleted_tx_id)
        "#
    );
    sqlx::query(&create_lookup_idx).execute(pool).await?;

    let create_reverse_idx = format!(
        r#"
        CREATE INDEX IF NOT EXISTS idx_tuples_reverse
        ON {schema_name}.relation_tuples
            (subject_type, subject_id, subject_relation, deleted_tx_id)
        "#
    );
    sqlx::query(&create_reverse_idx).execute(pool).await?;

    let create_schema_defs = format!(
        r#"
        CREATE TABLE IF NOT EXISTS {schema_name}.schema_definitions (
            id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            definition  TEXT NOT NULL,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
        )
        "#
    );
    sqlx::query(&create_schema_defs).execute(pool).await?;

    let create_seq = format!("CREATE SEQUENCE IF NOT EXISTS {schema_name}.tx_id_seq");
    sqlx::query(&create_seq).execute(pool).await?;

    Ok(())
}
