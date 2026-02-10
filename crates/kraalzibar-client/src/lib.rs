pub mod client;
pub mod config;
pub mod conversions;
pub mod error;
pub mod interceptor;
pub mod proto;

pub use client::{
    CheckPermissionRequest, CheckPermissionResponse, ExpandPermissionTreeRequest,
    ExpandPermissionTreeResponse, KraalzibarClient, LookupResourcesRequest, LookupSubjectsRequest,
    ReadRelationshipsRequest, Relationship, RelationshipOperation, RelationshipUpdate,
    WriteRelationshipsResponse, WriteSchemaResponse,
};
pub use config::ClientOptions;
pub use conversions::Consistency;
pub use error::ClientError;
