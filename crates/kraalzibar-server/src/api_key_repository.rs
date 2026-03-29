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
            "SELECT key_id, key_hash, tenant_id, \
             (revoked_at IS NOT NULL) as revoked, \
             (expires_at IS NOT NULL AND expires_at < now()) as expired \
             FROM api_keys WHERE key_id = $1",
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ApiKeyRecord {
            key_id: r.key_id,
            key_hash: r.key_hash,
            tenant_id: TenantId::new(r.tenant_id),
            revoked: r.revoked,
            expired: r.expired,
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

    pub async fn insert_with_expiry(
        &self,
        tenant_id: &TenantId,
        key_id: &str,
        key_hash: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO api_keys (tenant_id, key_id, key_hash, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tenant_id.as_uuid())
        .bind(key_id)
        .bind(key_hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn revoke_by_key_id(&self, key_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE api_keys SET revoked_at = now() \
             WHERE key_id = $1 AND revoked_at IS NULL",
        )
        .bind(key_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_for_tenant(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ApiKeyInfo>, sqlx::Error> {
        let rows = sqlx::query_as::<_, ApiKeyInfoRow>(
            "SELECT key_id, created_at, revoked_at, expires_at \
             FROM api_keys WHERE tenant_id = $1 ORDER BY created_at",
        )
        .bind(tenant_id.as_uuid())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ApiKeyInfo {
                key_id: r.key_id,
                created_at: r.created_at,
                revoked_at: r.revoked_at,
                expires_at: r.expires_at,
            })
            .collect())
    }
}

#[derive(sqlx::FromRow)]
struct ApiKeyRow {
    key_id: String,
    key_hash: String,
    tenant_id: uuid::Uuid,
    revoked: bool,
    expired: bool,
}

#[derive(Debug, Clone)]
pub struct ApiKeyInfo {
    pub key_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(sqlx::FromRow)]
struct ApiKeyInfoRow {
    key_id: String,
    created_at: chrono::DateTime<chrono::Utc>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
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

    #[tokio::test]
    #[ignore]
    async fn expired_key_returns_expired_true() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-expired").await;

        let key_hash = crate::auth::hash_secret("secret3").unwrap();
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        repo.insert_with_expiry(&tenant_id, "expiredkey", &key_hash, past)
            .await
            .unwrap();

        let record = repo.lookup_by_key_id("expiredkey").await.unwrap().unwrap();
        assert!(record.expired);
    }

    #[tokio::test]
    #[ignore]
    async fn non_expired_key_returns_expired_false() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-not-expired").await;

        let key_hash = crate::auth::hash_secret("secret4").unwrap();
        let future = chrono::Utc::now() + chrono::Duration::hours(24);
        repo.insert_with_expiry(&tenant_id, "futurekey", &key_hash, future)
            .await
            .unwrap();

        let record = repo.lookup_by_key_id("futurekey").await.unwrap().unwrap();
        assert!(!record.expired);
    }

    #[tokio::test]
    #[ignore]
    async fn revoke_by_key_id_marks_key_revoked() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-revoke-cmd").await;

        let key_hash = crate::auth::hash_secret("secret5").unwrap();
        repo.insert(&tenant_id, "revoketest1", &key_hash)
            .await
            .unwrap();

        let revoked = repo.revoke_by_key_id("revoketest1").await.unwrap();
        assert!(revoked);

        let record = repo.lookup_by_key_id("revoketest1").await.unwrap().unwrap();
        assert!(record.revoked);
    }

    #[tokio::test]
    #[ignore]
    async fn revoke_unknown_key_returns_false() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool);

        let revoked = repo.revoke_by_key_id("nonexistentkey").await.unwrap();
        assert!(!revoked);
    }

    #[tokio::test]
    #[ignore]
    async fn revoke_already_revoked_key_returns_false() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-double-revoke").await;

        let key_hash = crate::auth::hash_secret("secret6").unwrap();
        repo.insert(&tenant_id, "doublerevoke", &key_hash)
            .await
            .unwrap();

        let first = repo.revoke_by_key_id("doublerevoke").await.unwrap();
        assert!(first);

        let second = repo.revoke_by_key_id("doublerevoke").await.unwrap();
        assert!(!second);
    }

    #[tokio::test]
    #[ignore]
    async fn list_for_tenant_returns_all_keys() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-list-keys").await;

        let key_hash = crate::auth::hash_secret("s1").unwrap();
        repo.insert(&tenant_id, "listkey1", &key_hash)
            .await
            .unwrap();
        repo.insert(&tenant_id, "listkey2", &key_hash)
            .await
            .unwrap();

        let keys = repo.list_for_tenant(&tenant_id).await.unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].key_id, "listkey1");
        assert_eq!(keys[1].key_id, "listkey2");
        assert!(keys[0].revoked_at.is_none());
        assert!(keys[0].expires_at.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn list_for_tenant_with_no_keys_returns_empty() {
        let pool = test_pool().await;
        let repo = ApiKeyRepository::new(pool.clone());

        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        provision_test_tenant(&pool, &tenant_id, "test-list-empty").await;

        let keys = repo.list_for_tenant(&tenant_id).await.unwrap();
        assert!(keys.is_empty());
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
