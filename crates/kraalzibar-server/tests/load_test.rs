use std::sync::Arc;
use std::time::Instant;

use kraalzibar_core::engine::EngineConfig;
use kraalzibar_core::schema::SchemaLimits;
use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TenantId, TupleWrite};
use kraalzibar_server::service::{
    AuthzService, CheckPermissionInput, Consistency, LookupResourcesInput,
};
use kraalzibar_storage::InMemoryStoreFactory;

const SCHEMA: &str = r#"
definition user {}
definition group {
    relation member: user
}
definition folder {
    relation viewer: user | group#member
    permission can_view = viewer
}
definition document {
    relation parent: folder
    relation viewer: user | group#member
    permission can_view = viewer + parent->can_view
}
"#;

fn make_service() -> (AuthzService<InMemoryStoreFactory>, TenantId) {
    let factory = Arc::new(InMemoryStoreFactory::new());
    let service = AuthzService::new(factory, EngineConfig::default(), SchemaLimits::default());
    let tenant_id = TenantId::new(uuid::Uuid::new_v4());
    (service, tenant_id)
}

fn make_check_input(
    object_type: &str,
    object_id: &str,
    permission: &str,
    subject_type: &str,
    subject_id: &str,
    consistency: Consistency,
) -> CheckPermissionInput {
    CheckPermissionInput {
        object_type: object_type.to_string(),
        object_id: object_id.to_string(),
        permission: permission.to_string(),
        subject_type: subject_type.to_string(),
        subject_id: subject_id.to_string(),
        consistency,
    }
}

#[tokio::test]
#[ignore]
async fn load_test_1000_tuples_hierarchical_check() {
    let (service, tenant_id) = make_service();
    service
        .write_schema(&tenant_id, SCHEMA, false)
        .await
        .unwrap();

    // Create 10 folders, each with 100 documents
    let mut writes = Vec::new();
    for folder_idx in 0..10 {
        let folder_id = format!("folder{folder_idx}");
        writes.push(TupleWrite::new(
            ObjectRef::new("folder", &folder_id),
            "viewer",
            SubjectRef::direct("user", "alice"),
        ));

        for doc_idx in 0..100 {
            let doc_id = format!("doc_{folder_idx}_{doc_idx}");
            writes.push(TupleWrite::new(
                ObjectRef::new("document", &doc_id),
                "parent",
                SubjectRef::userset("folder", &folder_id, "can_view"),
            ));
        }
    }

    assert_eq!(writes.len(), 1010);

    let token = service
        .write_relationships(&tenant_id, &writes, &[])
        .await
        .unwrap();

    // Run 100 permission checks across the hierarchy
    let start = Instant::now();
    for i in 0..100 {
        let folder_idx = i % 10;
        let doc_idx = i % 100;
        let doc_id = format!("doc_{folder_idx}_{doc_idx}");

        let result = service
            .check_permission(
                &tenant_id,
                make_check_input(
                    "document",
                    &doc_id,
                    "can_view",
                    "user",
                    "alice",
                    Consistency::AtExactSnapshot(token),
                ),
            )
            .await
            .unwrap();

        assert!(result.allowed, "alice should have access to {doc_id}");
    }

    let elapsed = start.elapsed();
    eprintln!(
        "load_test_1000_tuples: 100 hierarchical checks in {:.2?} ({:.2?}/check)",
        elapsed,
        elapsed / 100,
    );
}

#[tokio::test]
#[ignore]
async fn load_test_10000_tuples_lookup_resources() {
    let (service, tenant_id) = make_service();
    service
        .write_schema(&tenant_id, SCHEMA, false)
        .await
        .unwrap();

    // Create 10000 documents with direct viewer access for alice
    let writes: Vec<TupleWrite> = (0..10_000)
        .map(|i| {
            TupleWrite::new(
                ObjectRef::new("document", format!("doc{i}")),
                "viewer",
                SubjectRef::direct("user", "alice"),
            )
        })
        .collect();

    // Write in batches of 1000 (batch size limit)
    for chunk in writes.chunks(1000) {
        service
            .write_relationships(&tenant_id, chunk, &[])
            .await
            .unwrap();
    }

    let start = Instant::now();
    let output = service
        .lookup_resources(
            &tenant_id,
            LookupResourcesInput {
                resource_type: "document".to_string(),
                permission: "can_view".to_string(),
                subject_type: "user".to_string(),
                subject_id: "alice".to_string(),
                consistency: Consistency::FullConsistency,
                limit: None,
            },
        )
        .await
        .unwrap();

    let elapsed = start.elapsed();
    eprintln!(
        "load_test_10000_tuples: lookup_resources found {} resources in {:.2?}",
        output.resource_ids.len(),
        elapsed,
    );

    // Default limit is 1000, so we should get at most 1000 back
    assert!(
        !output.resource_ids.is_empty(),
        "should find at least some resources"
    );
}

#[tokio::test]
#[ignore]
async fn load_test_cache_warm_vs_cold() {
    let (service, tenant_id) = make_service();
    service
        .write_schema(&tenant_id, SCHEMA, false)
        .await
        .unwrap();

    // Create a small graph for the check
    let writes = vec![
        TupleWrite::new(
            ObjectRef::new("folder", "f1"),
            "viewer",
            SubjectRef::direct("user", "alice"),
        ),
        TupleWrite::new(
            ObjectRef::new("document", "d1"),
            "parent",
            SubjectRef::userset("folder", "f1", "can_view"),
        ),
    ];

    let token = service
        .write_relationships(&tenant_id, &writes, &[])
        .await
        .unwrap();

    let input = || {
        make_check_input(
            "document",
            "d1",
            "can_view",
            "user",
            "alice",
            Consistency::AtExactSnapshot(token),
        )
    };

    // Cold check: first call populates both schema and check caches
    let cold_start = Instant::now();
    service.check_permission(&tenant_id, input()).await.unwrap();
    let cold_elapsed = cold_start.elapsed();

    // Warm run: all checks hit both schema and check caches (same snapshot key)
    let warm_start = Instant::now();
    for _ in 0..100 {
        service.check_permission(&tenant_id, input()).await.unwrap();
    }
    let warm_elapsed = warm_start.elapsed();

    eprintln!(
        "load_test_cache: 1 cold check in {:.2?}, 100 warm checks in {:.2?} ({:.2?}/check)",
        cold_elapsed,
        warm_elapsed,
        warm_elapsed / 100,
    );
}
