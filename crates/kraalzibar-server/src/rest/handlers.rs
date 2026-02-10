use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

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

const MAX_BATCH_SIZE: usize = 1000;
const MAX_IDENTIFIER_LENGTH: usize = 256;
const DEFAULT_READ_LIMIT: usize = 1000;

type ApiResult = (StatusCode, Json<serde_json::Value>);

fn json_response<T: Serialize>(status: StatusCode, body: &T) -> ApiResult {
    match serde_json::to_value(body) {
        Ok(v) => (status, Json(v)),
        Err(e) => {
            tracing::error!(error = %e, "response serialization failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal server error"})),
            )
        }
    }
}

fn error_response(status: StatusCode, msg: &str) -> ApiResult {
    (status, Json(serde_json::json!({"error": msg})))
}

fn validate_identifier(field: &str, value: &str) -> Result<(), ApiResult> {
    if value.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            &format!("{field} must not be empty"),
        ));
    }
    if value.len() > MAX_IDENTIFIER_LENGTH {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            &format!("{field} exceeds maximum length of {MAX_IDENTIFIER_LENGTH}"),
        ));
    }
    Ok(())
}

fn resolve_consistency(c: Option<&ConsistencyRequest>) -> Result<Consistency, ApiResult> {
    match c {
        Some(ConsistencyRequest::Full) => Ok(Consistency::FullConsistency),
        Some(ConsistencyRequest::AtLeastAsFresh { token }) => {
            let val: u64 = token.parse().map_err(|_| {
                error_response(StatusCode::BAD_REQUEST, "invalid snapshot token format")
            })?;
            Ok(Consistency::AtLeastAsFresh(SnapshotToken::new(val)))
        }
        Some(ConsistencyRequest::AtExactSnapshot { token }) => {
            let val: u64 = token.parse().map_err(|_| {
                error_response(StatusCode::BAD_REQUEST, "invalid snapshot token format")
            })?;
            Ok(Consistency::AtExactSnapshot(SnapshotToken::new(val)))
        }
        _ => Ok(Consistency::MinimizeLatency),
    }
}

fn api_error_to_response(err: ApiError) -> ApiResult {
    use kraalzibar_core::engine::CheckError;

    match &err {
        ApiError::Check(CheckError::TypeNotFound(_))
        | ApiError::Check(CheckError::PermissionNotFound { .. })
        | ApiError::Check(CheckError::RelationNotFound { .. })
        | ApiError::SchemaNotFound => error_response(StatusCode::NOT_FOUND, &err.to_string()),
        ApiError::Check(CheckError::MaxDepthExceeded(_)) => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, &err.to_string())
        }
        ApiError::Parse(_) | ApiError::Validation(_) => {
            error_response(StatusCode::BAD_REQUEST, &err.to_string())
        }
        ApiError::BreakingChanges(_) => {
            error_response(StatusCode::PRECONDITION_FAILED, &err.to_string())
        }
        ApiError::Check(CheckError::StorageError(_)) | ApiError::Storage(_) => {
            tracing::error!(error = %err, "internal storage error");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        }
    }
}

pub async fn check_permission<F>(
    State(state): State<AppState<F>>,
    Json(req): Json<CheckPermissionRequest>,
) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    if let Err(e) = validate_identifier("resource_type", &req.resource_type)
        .and_then(|()| validate_identifier("resource_id", &req.resource_id))
        .and_then(|()| validate_identifier("permission", &req.permission))
        .and_then(|()| validate_identifier("subject_type", &req.subject_type))
        .and_then(|()| validate_identifier("subject_id", &req.subject_id))
    {
        return e;
    }

    let input = CheckPermissionInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        subject_type: req.subject_type,
        subject_id: req.subject_id,
        consistency: match resolve_consistency(req.consistency.as_ref()) {
            Ok(c) => c,
            Err(resp) => return resp,
        },
    };

    match state
        .service
        .check_permission(&state.tenant_id, input)
        .await
    {
        Ok(result) => {
            let checked_at = result.snapshot.map(|s| s.value().to_string());
            json_response(
                StatusCode::OK,
                &CheckPermissionResponse {
                    allowed: result.allowed,
                    checked_at,
                },
            )
        }
        Err(e) => api_error_to_response(e),
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
    if let Err(e) = validate_identifier("resource_type", &req.resource_type)
        .and_then(|()| validate_identifier("resource_id", &req.resource_id))
        .and_then(|()| validate_identifier("permission", &req.permission))
    {
        return e;
    }

    let input = ExpandPermissionInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        consistency: match resolve_consistency(req.consistency.as_ref()) {
            Ok(c) => c,
            Err(resp) => return resp,
        },
    };

    match state
        .service
        .expand_permission_tree(&state.tenant_id, input)
        .await
    {
        Ok(output) => {
            let tree_json = expand_tree_to_json(&output.tree);
            json_response(
                StatusCode::OK,
                &ExpandPermissionResponse {
                    tree: tree_json,
                    expanded_at: output.snapshot.map(|s| s.to_string()),
                },
            )
        }
        Err(e) => api_error_to_response(e),
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
    if let Err(e) = validate_identifier("resource_type", &req.resource_type)
        .and_then(|()| validate_identifier("permission", &req.permission))
        .and_then(|()| validate_identifier("subject_type", &req.subject_type))
        .and_then(|()| validate_identifier("subject_id", &req.subject_id))
    {
        return e;
    }

    let input = LookupResourcesInput {
        resource_type: req.resource_type,
        permission: req.permission,
        subject_type: req.subject_type,
        subject_id: req.subject_id,
        consistency: match resolve_consistency(req.consistency.as_ref()) {
            Ok(c) => c,
            Err(resp) => return resp,
        },
        limit: req.limit.map(|l| l.min(10_000)),
    };

    match state
        .service
        .lookup_resources(&state.tenant_id, input)
        .await
    {
        Ok(output) => json_response(
            StatusCode::OK,
            &LookupResourcesResponse {
                resource_ids: output.resource_ids,
                looked_up_at: output.snapshot.map(|s| s.to_string()),
            },
        ),
        Err(e) => api_error_to_response(e),
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
    if let Err(e) = validate_identifier("resource_type", &req.resource_type)
        .and_then(|()| validate_identifier("resource_id", &req.resource_id))
        .and_then(|()| validate_identifier("permission", &req.permission))
        .and_then(|()| validate_identifier("subject_type", &req.subject_type))
    {
        return e;
    }

    let input = LookupSubjectsInput {
        object_type: req.resource_type,
        object_id: req.resource_id,
        permission: req.permission,
        subject_type: req.subject_type,
        consistency: match resolve_consistency(req.consistency.as_ref()) {
            Ok(c) => c,
            Err(resp) => return resp,
        },
    };

    match state.service.lookup_subjects(&state.tenant_id, input).await {
        Ok(output) => {
            let subjects: Vec<SubjectResponse> = output
                .subjects
                .into_iter()
                .map(|s| SubjectResponse {
                    subject_type: s.subject_type,
                    subject_id: s.subject_id,
                    subject_relation: s.subject_relation,
                })
                .collect();
            json_response(
                StatusCode::OK,
                &LookupSubjectsResponse {
                    subjects,
                    looked_up_at: output.snapshot.map(|s| s.to_string()),
                },
            )
        }
        Err(e) => api_error_to_response(e),
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
    if req.updates.len() > MAX_BATCH_SIZE {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!("batch size exceeds maximum of {MAX_BATCH_SIZE}"),
        );
    }

    let mut writes = Vec::new();
    let mut deletes = Vec::new();

    for update in &req.updates {
        if let Err(e) = validate_identifier("resource_type", &update.resource_type)
            .and_then(|()| validate_identifier("resource_id", &update.resource_id))
            .and_then(|()| validate_identifier("relation", &update.relation))
            .and_then(|()| validate_identifier("subject_type", &update.subject_type))
            .and_then(|()| validate_identifier("subject_id", &update.subject_id))
        {
            return e;
        }

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
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("unknown operation: {}", update.operation),
                );
            }
        }
    }

    match state
        .service
        .write_relationships(&state.tenant_id, &writes, &deletes)
        .await
    {
        Ok(token) => json_response(
            StatusCode::OK,
            &WriteRelationshipsResponse {
                written_at: token.value().to_string(),
            },
        ),
        Err(e) => api_error_to_response(e),
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

    let consistency = match resolve_consistency(req.consistency.as_ref()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let limit = Some(req.limit.unwrap_or(DEFAULT_READ_LIMIT));

    match state
        .service
        .read_relationships(&state.tenant_id, &filter, consistency, limit)
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
            json_response(StatusCode::OK, &ReadRelationshipsResponse { relationships })
        }
        Err(e) => api_error_to_response(e),
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
        Ok(result) => json_response(
            StatusCode::OK,
            &WriteSchemaResponse {
                breaking_changes_overridden: result.breaking_changes_overridden,
            },
        ),
        Err(e) => api_error_to_response(e),
    }
}

pub async fn read_schema<F>(State(state): State<AppState<F>>) -> impl IntoResponse
where
    F: StoreFactory + 'static,
    F::Store: RelationshipStore + SchemaStore,
{
    match state.service.read_schema(&state.tenant_id).await {
        Ok(Some(schema)) => json_response(StatusCode::OK, &ReadSchemaResponse { schema }),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "no schema has been written"),
        Err(e) => api_error_to_response(e),
    }
}

pub async fn watch() -> impl IntoResponse {
    error_response(StatusCode::NOT_IMPLEMENTED, "watch is not yet implemented")
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
        let (server, _metrics) = make_test_server_with_metrics();
        server
    }

    fn make_test_server_with_metrics() -> (TestServer, Arc<crate::metrics::Metrics>) {
        let factory = Arc::new(InMemoryStoreFactory::new());
        let service = Arc::new(AuthzService::new(
            factory,
            EngineConfig::default(),
            SchemaLimits::default(),
        ));
        let tenant_id = TenantId::new(uuid::Uuid::new_v4());
        let metrics = Arc::new(crate::metrics::Metrics::new());
        let state = AppState {
            service,
            tenant_id,
            metrics: Arc::clone(&metrics),
        };
        let app = create_router(state);
        (TestServer::new(app).unwrap(), metrics)
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

    #[tokio::test]
    async fn metrics_increment_on_requests() {
        let (server, metrics) = make_test_server_with_metrics();

        assert_eq!(metrics.request_total(), 0);

        server.get("/healthz").await;
        assert_eq!(metrics.request_total(), 1);
        assert_eq!(metrics.request_success(), 1);

        server.get("/v1/schema").await;
        assert_eq!(metrics.request_total(), 2);
        assert_eq!(metrics.request_error(), 1);
    }

    #[tokio::test]
    async fn rejects_empty_resource_type() {
        let server = make_test_server();

        let response = server
            .post("/v1/permissions/check")
            .json(&json!({
                "resource_type": "",
                "resource_id": "readme",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice"
            }))
            .await;

        response.assert_status(axum::http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json();
        assert!(body["error"].as_str().unwrap().contains("resource_type"));
    }

    #[tokio::test]
    async fn rejects_batch_exceeding_limit() {
        let server = make_test_server();

        let updates: Vec<serde_json::Value> = (0..1001)
            .map(|i| {
                json!({
                    "operation": "touch",
                    "resource_type": "document",
                    "resource_id": format!("doc{i}"),
                    "relation": "viewer",
                    "subject_type": "user",
                    "subject_id": "alice"
                })
            })
            .collect();

        let response = server
            .post("/v1/relationships/write")
            .json(&json!({"updates": updates}))
            .await;

        response.assert_status(axum::http::StatusCode::BAD_REQUEST);
        let body: serde_json::Value = response.json();
        assert!(body["error"].as_str().unwrap().contains("batch size"));
    }

    #[tokio::test]
    async fn breaking_schema_returns_412() {
        let server = make_test_server();
        setup_schema(&server).await;

        // Try to write a breaking schema without force
        let response = server
            .post("/v1/schema")
            .json(&json!({"schema": "definition user {}"}))
            .await;

        response.assert_status(axum::http::StatusCode::PRECONDITION_FAILED);
    }

    #[tokio::test]
    async fn expand_response_includes_snapshot_token() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/expand")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view",
                "consistency": {"type": "full"}
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(
            body["expanded_at"].is_string(),
            "expand response should include expanded_at token: {body}"
        );
    }

    #[tokio::test]
    async fn lookup_resources_response_includes_snapshot_token() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/resources")
            .json(&json!({
                "resource_type": "document",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice",
                "consistency": {"type": "full"}
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(
            body["looked_up_at"].is_string(),
            "lookup_resources response should include looked_up_at token: {body}"
        );
    }

    #[tokio::test]
    async fn lookup_subjects_response_includes_snapshot_token() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        let response = server
            .post("/v1/permissions/subjects")
            .json(&json!({
                "resource_type": "document",
                "resource_id": "readme",
                "permission": "can_view",
                "subject_type": "user",
                "consistency": {"type": "full"}
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        assert!(
            body["looked_up_at"].is_string(),
            "lookup_subjects response should include looked_up_at token: {body}"
        );
    }

    #[tokio::test]
    async fn lookup_resources_clamps_limit() {
        let server = make_test_server();
        setup_schema(&server).await;
        write_tuple(&server, "readme", "viewer", "alice").await;

        // Sending a very large limit should not cause an error — it gets clamped
        let response = server
            .post("/v1/permissions/resources")
            .json(&json!({
                "resource_type": "document",
                "permission": "can_view",
                "subject_type": "user",
                "subject_id": "alice",
                "limit": 999_999_999
            }))
            .await;

        response.assert_status_ok();
        let body: serde_json::Value = response.json();
        let ids = body["resource_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 1);
    }
}
