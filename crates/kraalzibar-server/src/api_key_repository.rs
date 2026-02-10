use kraalzibar_core::tuple::TenantId;
use sqlx::PgPool;

use crate::auth::ApiKeyRecord;

#[derive(Debug, Clone)]
pub struct ApiKeyRepository {
    pool: PgPool,
}

impl ApiKeyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn lookup_by_key_id(
        &self,
        key_id: &str,
    ) -> Result<Option<ApiKeyRecord>, sqlx::Error> {
        let row = sqlx::query_as::<_, ApiKeyRow>(
            "SELECT key_id, key_hash, tenant_id, (revoked_at IS NOT NULL) as revoked FROM api_keys WHERE key_id = $1",
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ApiKeyRecord {
            key_id: r.key_id,
            key_hash: r.key_hash,
            tenant_id: TenantId::new(r.tenant_id),
            revoked: r.revoked,
        }))
    }

    pub async fn insert(
        &self,
        tenant_id: &TenantId,
        key_id: &str,
        key_hash: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO api_keys (tenant_id, key_id, key_hash) VALUES ($1, $2, $3)")
            .bind(tenant_id.as_uuid())
            .bind(key_id)
            .bind(key_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ApiKeyRow {
    key_id: String,
    key_hash: String,
    tenant_id: uuid::Uuid,
    revoked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn lookup_returns_none_for_unknown_key() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool);

        let result = repo.lookup_by_key_id("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn insert_and_lookup_round_trip() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-roundtrip").await;

        let key_hash = crate::auth::hash_secret("testsecret").unwrap();
        repo.insert(&tenant_id, "testkey1", &key_hash)
            .await
            .unwrap();

        let record = repo.lookup_by_key_id("testkey1").await.unwrap().unwrap();
        assert_eq!(record.key_id, "testkey1");
        assert_eq!(record.tenant_id, tenant_id);
        assert!(!record.revoked);
        assert!(crate::auth::verify_secret("testsecret", &record.key_hash).unwrap());
    }

    #[tokio::test]
    #[ignore]
    async fn revoked_key_returns_revoked_true() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-revoked").await;

        let key_hash = crate::auth::hash_secret("secret2").unwrap();
        repo.insert(&tenant_id, "revokedkey", &key_hash)
            .await
            .unwrap();

        sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE key_id = $1")
            .bind("revokedkey")
            .execute(&pool)
            .await
            .unwrap();

        let record = repo.lookup_by_key_id("revokedkey").await.unwrap().unwrap();
        assert!(record.revoked);
    }

    async fn test_pool() -> PgPool {
        use std::sync::OnceLock;
        use testcontainers::runners::AsyncRunner;
        use testcontainers_modules::postgres::Postgres;

        static POOL: OnceLock<PgPool> = OnceLock::new();

        if let Some(pool) = POOL.get() {
            return pool.clone();
        }

        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@localhost:{port}/postgres");
        let pool = PgPool::connect(&url).await.unwrap();

        kraalzibar_storage::postgres::migrations::run_shared_migrations(&pool)
            .await
            .unwrap();

        // Leak the container so it lives for the process duration
        std::mem::forget(container);

        POOL.get_or_init(|| pool).clone()
    }

    async fn provision_test_tenant(pool: &PgPool, tenant_id: &TenantId, name: &str) {
        let factory = kraalzibar_storage::postgres::PostgresStoreFactory::new(pool.clone());
        factory.provision_tenant(tenant_id, name).await.unwrap();
    }
}
