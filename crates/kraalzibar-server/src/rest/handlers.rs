use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use kraalzibar_core::engine::ExpandTree;
use kraalzibar_core::tuple::{ObjectRef, SnapshotToken, SubjectRef, TupleFilter, TupleWrite};
use kraalzibar_storage::traits::{RelationshipStore, SchemaStore, StoreFactory};

use crate::error::ApiError;
use crate::service::{
    CheckPermissionInput, Consistency, ExpandPermissionInput, LookupResourcesInput,
    LookupSubjectsInput,
};

use super::AppState;
use super::types::*;

fn resolve_consistency(c: Option<&ConsistencyRequest>) -> Consistency {
    match c {
        Some(ConsistencyRequest::Full) => Consistency::FullConsistency,
        Some(ConsistencyRequest::AtLeastAsFresh { token }) => {
            let val: u64 = token.parse().unwrap_or(0);
            Consistency::AtLeastAsFresh(SnapshotToken::new(val))
        }
        Some(ConsistencyRequest::AtExactSnapshot { token }) => {
            let val: u64 = token.parse().unwrap_or(0);
            Consistency::AtExactSnapshot(SnapshotToken::new(val))
        }
        _ => Consistency::MinimizeLatency,
    }
}

fn api_error_to_response(err: ApiError) -> (StatusCode, Json<ErrorResponse>) {
    use kraalzibar_core::engine::CheckError;

    let status = match &err {
        ApiError::Check(CheckError::TypeNotFound(_))
        | ApiError::Check(CheckError::PermissionNotFound { .. })
        | ApiError::Check(CheckError::RelationNotFound { .. })
        | ApiError::SchemaNotFound => StatusCode::NOT_FOUND,
        ApiError::Check(CheckError::MaxDepthExceeded(_)) => StatusCode::UNPROCESSABLE_ENTITY,
        ApiError::Parse(_) | ApiError::Validation(_) => StatusCode::BAD_REQUEST,
        ApiError::BreakingChanges(_) => StatusCode::CONFLICT,
        ApiError::Check(CheckError::StorageError(_)) | ApiError::Storage(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };

    (
        status,
        Json(ErrorResponse {
            error: err.to_string(),
        }),
    )
}

pub async fn check_permission<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<CheckPermissionRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let input = CheckPermissionInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        subject_type: req.subject_type,
        subject_id: req.subject_id,
        consistency: resolve_consistency(req.consistency.as_ref()),
    };

    match state
        .service
        .check_permission(&state.tenant_id, input)
        .await
    {
        Ok(result) => {
            let checked_at = result.snapshot.map(|s| s.value().to_string());
            (
                StatusCode::OK,
                Json(
                    serde_json::to_value(CheckPermissionResponse {
                        allowed: result.allowed,
                        checked_at,
                    })
                    .unwrap(),
                ),
            )
        }
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn expand_permission<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<ExpandPermissionRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let input = ExpandPermissionInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        consistency: resolve_consistency(req.consistency.as_ref()),
    };

    match state
        .service
        .expand_permission_tree(&state.tenant_id, input)
        .await
    {
        Ok(tree) => {
            let tree_json = expand_tree_to_json(&tree);
            (
                StatusCode::OK,
                Json(serde_json::to_value(ExpandPermissionResponse { tree: tree_json }).unwrap()),
            )
        }
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

fn expand_tree_to_json(tree: &ExpandTree) -> serde_json::Value {
    match tree {
        ExpandTree::Leaf { subject } => serde_json::json!({
            "type": "leaf",
            "subject": {
                "subject_type": subject.subject_type,
                "subject_id": subject.subject_id,
                "subject_relation": subject.subject_relation,
            }
        }),
        ExpandTree::This { subjects, .. } => serde_json::json!({
            "type": "this",
            "children": subjects.iter().map(expand_tree_to_json).collect::<Vec<_>>()
        }),
        ExpandTree::Union { children } => serde_json::json!({
            "type": "union",
            "children": children.iter().map(expand_tree_to_json).collect::<Vec<_>>()
        }),
        ExpandTree::Intersection { children } => serde_json::json!({
            "type": "intersection",
            "children": children.iter().map(expand_tree_to_json).collect::<Vec<_>>()
        }),
        ExpandTree::Exclusion { base, excluded } => serde_json::json!({
            "type": "exclusion",
            "base": expand_tree_to_json(base),
            "excluded": expand_tree_to_json(excluded),
        }),
        ExpandTree::Arrow { children, .. } => serde_json::json!({
            "type": "arrow",
            "children": children.iter().map(expand_tree_to_json).collect::<Vec<_>>()
        }),
    }
}

pub async fn lookup_resources<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<LookupResourcesRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let input = LookupResourcesInput {
        resource_type: req.resource_type,
        permission: req.permission,
        subject_type: req.subject_type,
        subject_id: req.subject_id,
        consistency: resolve_consistency(req.consistency.as_ref()),
        limit: req.limit,
    };

    match state
        .service
        .lookup_resources(&state.tenant_id, input)
        .await
    {
        Ok(ids) => (
            StatusCode::OK,
            Json(serde_json::to_value(LookupResourcesResponse { resource_ids: ids }).unwrap()),
        ),
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn lookup_subjects<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<LookupSubjectsRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let input = LookupSubjectsInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        subject_type: req.subject_type,
        consistency: resolve_consistency(req.consistency.as_ref()),
    };

    match state.service.lookup_subjects(&state.tenant_id, input).await {
        Ok(subjects) => {
            let subjects: Vec<SubjectResponse> = subjects
                .into_iter()
                .map(|s| SubjectResponse {
                    subject_type: s.subject_type,
                    subject_id: s.subject_id,
                    subject_relation: s.subject_relation,
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::to_value(LookupSubjectsResponse { subjects }).unwrap()),
            )
        }
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn write_relationships<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<WriteRelationshipsRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let mut writes = Vec::new();
    let mut deletes = Vec::new();

    for update in &req.updates {
        let object = ObjectRef::new(&update.resource_type, &update.resource_id);
        let subject = match &update.subject_relation {
            Some(rel) => SubjectRef::userset(&update.subject_type, &update.subject_id, rel),
            None => SubjectRef::direct(&update.subject_type, &update.subject_id),
        };

        match update.operation.as_str() {
            "touch" => writes.push(TupleWrite::new(object, &update.relation, subject)),
            "delete" => deletes.push(TupleFilter {
                object_type: Some(update.resource_type.clone()),
                object_id: Some(update.resource_id.clone()),
                relation: Some(update.relation.clone()),
                subject_type: Some(update.subject_type.clone()),
                subject_id: Some(update.subject_id.clone()),
                subject_relation: None,
            }),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(
                        serde_json::to_value(ErrorResponse {
                            error: format!("unknown operation: {}", update.operation),
                        })
                        .unwrap(),
                    ),
                );
            }
        }
    }

    match state
        .service
        .write_relationships(&state.tenant_id, &writes, &deletes)
        .await
    {
        Ok(token) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(WriteRelationshipsResponse {
                    written_at: token.value().to_string(),
                })
                .unwrap(),
            ),
        ),
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn read_relationships<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<ReadRelationshipsRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    let filter = req
        .filter
        .map(|f| TupleFilter {
            object_type: f.resource_type,
            object_id: f.resource_id,
            relation: f.relation,
            subject_type: f.subject_type,
            subject_id: f.subject_id,
            subject_relation: None,
        })
        .unwrap_or_default();

    let consistency = resolve_consistency(req.consistency.as_ref());

    match state
        .service
        .read_relationships(&state.tenant_id, &filter, consistency, req.limit)
        .await
    {
        Ok(tuples) => {
            let relationships: Vec<RelationshipResponse> = tuples
                .iter()
                .map(|t| RelationshipResponse {
                    resource_type: t.object.object_type.clone(),
                    resource_id: t.object.object_id.clone(),
                    relation: t.relation.clone(),
                    subject_type: t.subject.subject_type.clone(),
                    subject_id: t.subject.subject_id.clone(),
                    subject_relation: t.subject.subject_relation.clone(),
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::to_value(ReadRelationshipsResponse { relationships }).unwrap()),
            )
        }
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn write_schema<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<WriteSchemaRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    match state
        .service
        .write_schema(&state.tenant_id, &req.schema, req.force)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(WriteSchemaResponse {
                    breaking_changes_overridden: result.breaking_changes_overridden,
                })
                .unwrap(),
            ),
        ),
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn read_schema<F>(State(state): State<AppState<F>>) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    match state.service.read_schema(&state.tenant_id).await {
        Ok(Some(schema)) => (
            StatusCode::OK,
            Json(serde_json::to_value(ReadSchemaResponse { schema }).unwrap()),
        ),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(
                serde_json::to_value(ErrorResponse {
                    error: "no schema has been written".to_string(),
                })
                .unwrap(),
            ),
        ),
        Err(e) => {
            let (status, Json(body)) = api_error_to_response(e);
            (status, Json(serde_json::to_value(body).unwrap()))
        }
    }
}

pub async fn watch() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({"error": "watch is not yet implemented"})),
    )
}

pub async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

#[cfg(test)]
mod tests {
    use super::super::{AppState, create_router};
    use crate::service::AuthzService;
    use axum_test::TestServer;
    use kraalzibar_core::engine::EngineConfig;
    use kraalzibar_core::schema::SchemaLimits;
    use kraalzibar_core::tuple::TenantId;
    use kraalzibar_storage::InMemoryStoreFactory;
    use serde_json::json;
    use std::sync::Arc;

    fn make_test_server() -> TestServer {
        let factory = Arc::new(InMemoryStoreFactory::new());
        let service = Arc::new(AuthzService::new(
            factory,
            EngineConfig::default(),
            SchemaLimits::default(),
        ));
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        let state = AppState { service, tenant_id };
        let app = create_router(state);
        TestServer::new(app).unwrap()
    }

    const SCHEMA: &str = r#"
        definition user {}
        definition document {
            relation viewer: user
            relation editor: user
            permission can_view = viewer + editor
        }
    "#;

    async fn setup_schema(server: &TestServer) {
        server
            .post("/v1/schema")
            .json(&json!({"schema": SCHEMA}))
            .await;
    }

    async fn write_tuple(server: &TestServer, resource_id: &str, relation: &str, subject_id: &str) {
        server
            .post("/v1/relationships/write")
            .json(&json!({
                "updates": [{
                    "operation": "touch",
                    "resource_type": "document",
                    "resource_id": resource_id,
                    "relation": relation,
                    "subject_type": "user",
                    "subject_id": subject_id
                }]
            }))
            .await;
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let server = make_test_server();
        let response = server.get("/healthz").await;
        response.assert_status_ok();
        response.assert_json(&json!({"status": "ok"}));
    }

    #[tokio::test]
    async fn watch_returns_501() {
        let server = make_test_server();
        let response = server.get("/v1/watch").await;
        response.assert_status(axum::http::StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn check_permission_grants_access() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/check")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["allowed"], true);
    }

    #[tokio::test]
    async fn check_permission_denies_access() {
        let server = make_test_server();
        setup_schema(&server).await;

        let response = server
            .post("/v1/permissions/check")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["allowed"], false);
    }

    #[tokio::test]
    async fn write_and_read_relationships() {
        let server = make_test_server();
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/relationships/read")
            .json(&json!({
                "filter": {"resource_type": "document"}
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert_eq!(body["relationships"].as_array().unwrap().len(), 1);
        assert_eq!(body["relationships"][0]["subject_id"], "alice");
    }

    #[tokio::test]
    async fn write_and_read_schema() {
        let server = make_test_server();

        let response = server
            .post("/v1/schema")
            .json(&json!({"schema": SCHEMA}))
            .await;
        response.assert_status_ok();

        let response = server.get("/v1/schema").await;
        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body["schema"].as_str().unwrap().contains("document"));
    }

    #[tokio::test]
    async fn read_schema_returns_404_when_empty() {
        let server = make_test_server();
        let response = server.get("/v1/schema").await;
        response.assert_status(axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn expand_permission_returns_tree() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/expand")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body["tree"].is_object());
    }

    #[tokio::test]
    async fn lookup_resources_returns_matches() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;
        write_tuple(&server, "design", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/resources")
            .json(&json!({
                "resource_type": "document",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let ids = body["resource_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn lookup_subjects_returns_matches() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;
        write_tuple(&server, "readme", "editor", "bob").await;

        let response = server
            .post("/v1/permissions/subjects")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view",
                "subject_type": "user"
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let subjects = body["subjects"].as_array().unwrap();
        assert!(subjects.len() >= 2);
    }

    #[tokio::test]
    async fn write_relationships_returns_token() {
        let server = make_test_server();

        let response = server
            .post("/v1/relationships/write")
            .json(&json!({
                "updates": [{
                    "operation": "touch",
                    "resource_type": "document",
                    "resource_id": "readme",
                    "relation": "viewer",
                    "subject_type": "user",
                    "subject_id": "alice"
                }]
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(body["written_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn unknown_operation_returns_400() {
        let server = make_test_server();

        let response = server
            .post("/v1/relationships/write")
            .json(&json!({
                "updates": [{
                    "operation": "invalid",
                    "resource_type": "document",
                    "resource_id": "readme",
                    "relation": "viewer",
                    "subject_type": "user",
                    "subject_id": "alice"
                }]
            }))
            .await;

        response.assert_status(axum::http::StatusCode::BAD_REQUEST);
    }
}
