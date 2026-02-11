pub mod migrations;
mod queries;

use std::collections::HashMap;
use std::sync::Mutex;

use sqlx::PgPool;

use kraalzibar_core::tuple::{SnapshotToken, TenantId, Tuple, TupleFilter, TupleWrite};

use crate::traits::{RelationshipStore, SchemaStore, StorageError, StoreFactory};

fn schema_name_for_tenant(tenant_id: &TenantId) -> String {
    let simple = tenant_id.as_uuid().as_simple().to_string();
    format!("tenant_{simple}")
}

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
    schema: String,
}

impl RelationshipStore for PostgresStore {
    async fn write(
        &self,
        writes: &[TupleWrite],
        deletes: &[TupleFilter],
    ) -> Result<SnapshotToken, StorageError> {
        for filter in deletes {
            if !filter.has_any_field() {
                return Err(StorageError::EmptyDeleteFilter);
            }
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::Internal(e.to_string()))?;

        let tx_id = queries::next_tx_id(&mut *tx, &self.schema).await?;

        for filter in deletes {
            queries::delete_matching_tuples(&mut *tx, &self.schema, filter, tx_id).await?;
        }

        for w in writes {
            queries::insert_tuple(&mut *tx, &self.schema, w, tx_id).await?;
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::Internal(e.to_string()))?;

        #[allow(clippy::cast_sign_loss)]
        Ok(SnapshotToken::new(tx_id as u64))
    }

    async fn read(
        &self,
        filter: &TupleFilter,
        snapshot: Option<SnapshotToken>,
        limit: Option<usize>,
    ) -> Result<Vec<Tuple>, StorageError> {
        let snap = match snapshot {
            Some(token) => {
                let val = token.value();
                let current = queries::current_tx_id(&self.pool, &self.schema).await?;
                #[allow(clippy::cast_sign_loss)]
                if val > current as u64 {
                    return Err(StorageError::SnapshotAhead {
                        requested: val,
                        #[allow(clippy::cast_sign_loss)]
                        current: current as u64,
                    });
                }
                val as i64
            }
            None => queries::current_tx_id(&self.pool, &self.schema).await?,
        };

        queries::read_tuples(&self.pool, &self.schema, filter, snap, limit).await
    }

    async fn snapshot(&self) -> Result<SnapshotToken, StorageError> {
        let current = queries::current_tx_id(&self.pool, &self.schema).await?;
        #[allow(clippy::cast_sign_loss)]
        Ok(SnapshotToken::new(current as u64))
    }

    async fn list_object_ids(
        &self,
        object_type: &str,
        snapshot: Option<SnapshotToken>,
        limit: Option<usize>,
    ) -> Result<Vec<String>, StorageError> {
        let snap = match snapshot {
            Some(token) => {
                let val = token.value();
                let current = queries::current_tx_id(&self.pool, &self.schema).await?;
                #[allow(clippy::cast_sign_loss)]
                if val > current as u64 {
                    return Err(StorageError::SnapshotAhead {
                        requested: val,
                        #[allow(clippy::cast_sign_loss)]
                        current: current as u64,
                    });
                }
                val as i64
            }
            None => queries::current_tx_id(&self.pool, &self.schema).await?,
        };

        queries::list_distinct_object_ids(&self.pool, &self.schema, object_type, snap, limit).await
    }
}

impl SchemaStore for PostgresStore {
    async fn write_schema(&self, definition: &str) -> Result<(), StorageError> {
        queries::write_schema_definition(&self.pool, &self.schema, definition).await
    }

    async fn read_schema(&self) -> Result<Option<String>, StorageError> {
        queries::read_latest_schema(&self.pool, &self.schema).await
    }
}

pub struct PostgresStoreFactory {
    pool: PgPool,
    schemas: Mutex<HashMap<TenantId, String>>,
}

impl PostgresStoreFactory {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            schemas: Mutex::new(HashMap::new()),
        }
    }

    pub async fn provision_tenant(
        &self,
        tenant_id: &TenantId,
        name: &str,
    ) -> Result<(), StorageError> {
        let schema_name = schema_name_for_tenant(tenant_id);

        sqlx::query(
            "INSERT INTO tenants (id, name, pg_schema) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(tenant_id.as_uuid())
        .bind(name)
        .bind(&schema_name)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::Internal(e.to_string()))?;

        migrations::create_tenant_schema(&self.pool, &schema_name).await?;

        let mut schemas = self.schemas.lock().unwrap();
        schemas.insert(tenant_id.clone(), schema_name);

        Ok(())
    }
}

impl StoreFactory for PostgresStoreFactory {
    type Store = PostgresStore;

    fn for_tenant(&self, tenant_id: &TenantId) -> PostgresStore {
        let schemas = self.schemas.lock().unwrap();
        let schema = schemas
            .get(tenant_id)
            .cloned()
            .unwrap_or_else(|| schema_name_for_tenant(tenant_id));

        PostgresStore {
            pool: self.pool.clone(),
            schema,
        }
    }
}

#[cfg(test)]
mod pg_tests {
    use super::*;
    use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TupleFilter, TupleWrite};
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    use uuid::Uuid;

    async fn setup_pg() -> (PgPool, testcontainers::ContainerAsync<Postgres>) {
        let container = Postgres::default().start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url = format!("postgres://postgres:postgres@localhost:{port}/postgres");
        let pool = PgPool::connect(&url).await.unwrap();

        migrations::run_shared_migrations(&pool).await.unwrap();

        (pool, container)
    }

    fn make_write(obj_type: &str, obj_id: &str, relation: &str, subj: SubjectRef) -> TupleWrite {
        TupleWrite::new(ObjectRef::new(obj_type, obj_id), relation, subj)
    }

    #[tokio::test]
    #[ignore]
    async fn pg_provision_and_write_read() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);
        let tenant_id = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_id, "test-tenant")
            .await
            .unwrap();

        let store = factory.for_tenant(&tenant_id);

        let snap = store.snapshot().await.unwrap();
        assert_eq!(snap.value(), 0);

        let token = store
            .write(
                &[make_write(
                    "doc",
                    "readme",
                    "viewer",
                    SubjectRef::direct("user", "john"),
                )],
                &[],
            )
            .await
            .unwrap();
        assert_eq!(token.value(), 1);

        let results = store
            .read(&TupleFilter::default(), None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object, ObjectRef::new("doc", "readme"));
        assert_eq!(results[0].relation, "viewer");
        assert_eq!(results[0].subject, SubjectRef::direct("user", "john"));
    }

    #[tokio::test]
    #[ignore]
    async fn pg_snapshot_consistency() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);
        let tenant_id = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_id, "test-snap")
            .await
            .unwrap();

        let store = factory.for_tenant(&tenant_id);

        let snap1 = store
            .write(
                &[make_write(
                    "doc",
                    "1",
                    "viewer",
                    SubjectRef::direct("user", "a"),
                )],
                &[],
            )
            .await
            .unwrap();

        store
            .write(
                &[make_write(
                    "doc",
                    "2",
                    "viewer",
                    SubjectRef::direct("user", "b"),
                )],
                &[],
            )
            .await
            .unwrap();

        let results = store
            .read(&TupleFilter::default(), Some(snap1), None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object.object_id, "1");
    }

    #[tokio::test]
    #[ignore]
    async fn pg_delete_and_mvcc() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);
        let tenant_id = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_id, "test-del")
            .await
            .unwrap();

        let store = factory.for_tenant(&tenant_id);

        let snap_before = store
            .write(
                &[make_write(
                    "doc",
                    "1",
                    "viewer",
                    SubjectRef::direct("user", "a"),
                )],
                &[],
            )
            .await
            .unwrap();

        let delete_filter = TupleFilter {
            object_type: Some("doc".to_string()),
            object_id: Some("1".to_string()),
            ..Default::default()
        };
        store.write(&[], &[delete_filter]).await.unwrap();

        let after_delete = store
            .read(&TupleFilter::default(), None, None)
            .await
            .unwrap();
        assert!(after_delete.is_empty());

        let before_delete = store
            .read(&TupleFilter::default(), Some(snap_before), None)
            .await
            .unwrap();
        assert_eq!(before_delete.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn pg_tenant_isolation() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);

        let tenant_a = TenantId::new(Uuid::new_v4());
        let tenant_b = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_a, "tenant-a")
            .await
            .unwrap();
        factory
            .provision_tenant(&tenant_b, "tenant-b")
            .await
            .unwrap();

        let store_a = factory.for_tenant(&tenant_a);
        let store_b = factory.for_tenant(&tenant_b);

        store_a
            .write(
                &[make_write(
                    "doc",
                    "1",
                    "viewer",
                    SubjectRef::direct("user", "a"),
                )],
                &[],
            )
            .await
            .unwrap();

        let results_b = store_b
            .read(&TupleFilter::default(), None, None)
            .await
            .unwrap();
        assert!(results_b.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn pg_list_object_ids() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);
        let tenant_id = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_id, "test-list-ids")
            .await
            .unwrap();

        let store = factory.for_tenant(&tenant_id);

        store
            .write(
                &[
                    make_write("doc", "readme", "viewer", SubjectRef::direct("user", "a")),
                    make_write("doc", "readme", "editor", SubjectRef::direct("user", "b")),
                    make_write("doc", "design", "viewer", SubjectRef::direct("user", "a")),
                    make_write("folder", "root", "viewer", SubjectRef::direct("user", "a")),
                ],
                &[],
            )
            .await
            .unwrap();

        let ids = store.list_object_ids("doc", None, None).await.unwrap();
        assert_eq!(ids, vec!["design", "readme"]);

        let ids = store.list_object_ids("folder", None, None).await.unwrap();
        assert_eq!(ids, vec!["root"]);

        let ids = store.list_object_ids("doc", None, Some(1)).await.unwrap();
        assert_eq!(ids.len(), 1);

        let ids = store.list_object_ids("unknown", None, None).await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn pg_schema_write_and_read() {
        let (pool, _container) = setup_pg().await;
        let factory = PostgresStoreFactory::new(pool);
        let tenant_id = TenantId::new(Uuid::new_v4());
        factory
            .provision_tenant(&tenant_id, "test-schema")
            .await
            .unwrap();

        let store = factory.for_tenant(&tenant_id);

        let schema = store.read_schema().await.unwrap();
        assert_eq!(schema, None);

        store.write_schema("definition user {}").await.unwrap();
        let schema = store.read_schema().await.unwrap();
        assert_eq!(schema, Some("definition user {}".to_string()));

        store
            .write_schema("definition group { relation member: user }")
            .await
            .unwrap();
        let schema = store.read_schema().await.unwrap();
        assert_eq!(
            schema,
            Some("definition group { relation member: user }".to_string())
        );
    }
}
