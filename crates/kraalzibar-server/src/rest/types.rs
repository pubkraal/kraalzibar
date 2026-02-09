use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CheckPermissionRequest {
    pub resource_type: String,
    pub resource_id: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub consistency: Option<ConsistencyRequest>,
}

#[derive(Debug, Serialize)]
pub struct CheckPermissionResponse {
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExpandPermissionRequest {
    pub resource_type: String,
    pub resource_id: String,
    pub permission: String,
    #[serde(default)]
    pub consistency: Option<ConsistencyRequest>,
}

#[derive(Debug, Serialize)]
pub struct ExpandPermissionResponse {
    pub tree: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct LookupResourcesRequest {
    pub resource_type: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub consistency: Option<ConsistencyRequest>,
}

#[derive(Debug, Serialize)]
pub struct LookupResourcesResponse {
    pub resource_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct LookupSubjectsRequest {
    pub resource_type: String,
    pub resource_id: String,
    pub permission: String,
    pub subject_type: String,
    #[serde(default)]
    pub consistency: Option<ConsistencyRequest>,
}

#[derive(Debug, Serialize)]
pub struct LookupSubjectsResponse {
    pub subjects: Vec<SubjectResponse>,
}

#[derive(Debug, Serialize)]
pub struct SubjectResponse {
    pub subject_type: String,
    pub subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_relation: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteRelationshipsRequest {
    pub updates: Vec<RelationshipUpdate>,
}

#[derive(Debug, Deserialize)]
pub struct RelationshipUpdate {
    pub operation: String,
    pub resource_type: String,
    pub resource_id: String,
    pub relation: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub subject_relation: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WriteRelationshipsResponse {
    pub written_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadRelationshipsRequest {
    #[serde(default)]
    pub filter: Option<RelationshipFilterRequest>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub consistency: Option<ConsistencyRequest>,
}

#[derive(Debug, Deserialize)]
pub struct RelationshipFilterRequest {
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub relation: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadRelationshipsResponse {
    pub relationships: Vec<RelationshipResponse>,
}

#[derive(Debug, Serialize)]
pub struct RelationshipResponse {
    pub resource_type: String,
    pub resource_id: String,
    pub relation: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_relation: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteSchemaRequest {
    pub schema: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize)]
pub struct WriteSchemaResponse {
    pub breaking_changes_overridden: bool,
}

#[derive(Debug, Serialize)]
pub struct ReadSchemaResponse {
    pub schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ConsistencyRequest {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "minimize_latency")]
    MinimizeLatency,
    #[serde(rename = "at_least_as_fresh")]
    AtLeastAsFresh { token: String },
    #[serde(rename = "at_exact_snapshot")]
    AtExactSnapshot { token: String },
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}
