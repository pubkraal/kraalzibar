# Kraalzibar Rust SDK

Rust client for the [Kraalzibar](https://github.com/pubkraal/kraalzibar) authorization server. Communicates over gRPC using tonic.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
kraalzibar-client = { git = "https://github.com/pubkraal/kraalzibar" }
```

## Quick Start

```rust
use kraalzibar_client::{
    CheckPermissionRequest, ClientOptions, Consistency, KraalzibarClient,
    RelationshipOperation, RelationshipUpdate,
};
use kraalzibar_core::tuple::{ObjectRef, SubjectRef};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = KraalzibarClient::connect(
        "http://localhost:50051",
        ClientOptions::default(),
    ).await?;

    // Write schema
    client.write_schema(
        "definition user {}\n\ndefinition document {\n  relation viewer: user\n  permission view = viewer\n}",
        false,
    ).await?;

    // Create a relationship
    client.write_relationships(vec![RelationshipUpdate {
        operation: RelationshipOperation::Touch,
        object: ObjectRef::new("document", "readme"),
        relation: "viewer".to_string(),
        subject: SubjectRef::direct("user", "alice"),
    }]).await?;

    // Check permission
    let result = client.check_permission(CheckPermissionRequest {
        resource: ObjectRef::new("document", "readme"),
        permission: "view".to_string(),
        subject: SubjectRef::direct("user", "alice"),
        consistency: Consistency::FullConsistency,
    }).await?;

    println!("alice can view: {}", result.allowed);
    Ok(())
}
```

## Authentication

Pass an API key via `ClientOptions` to authenticate with a production server:

```rust
let options = ClientOptions {
    api_key: Some("kraalzibar_abcd1234_secretsecretsecretsecretsecretsec".to_string()),
    ..Default::default()
};

let mut client = KraalzibarClient::connect("http://server:50051", options).await?;
```

The API key is sent as a `Bearer` token in the `authorization` metadata header on every gRPC request. The key is redacted in `Debug` output.

## Client Options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `api_key` | `Option<String>` | `None` | API key for authentication |
| `timeout` | `Duration` | 30s | Per-request timeout |
| `connect_timeout` | `Duration` | 5s | Connection establishment timeout |

```rust
use std::time::Duration;
use kraalzibar_client::ClientOptions;

let options = ClientOptions {
    api_key: Some("kraalzibar_...".to_string()),
    timeout: Duration::from_secs(10),
    connect_timeout: Duration::from_secs(2),
};
```

## API Reference

### `KraalzibarClient::connect`

```rust
pub async fn connect(endpoint: &str, options: ClientOptions) -> Result<Self, ClientError>
```

Connects to a kraalzibar gRPC server.

### `KraalzibarClient::from_channel`

```rust
pub fn from_channel(channel: Channel, options: ClientOptions) -> Self
```

Creates a client from an existing tonic `Channel` (useful for custom transport configuration).

### `check_permission`

```rust
pub async fn check_permission(&mut self, request: CheckPermissionRequest) -> Result<CheckPermissionResponse, ClientError>
```

Checks whether a subject has a permission on a resource. Returns `allowed: bool` and an optional `checked_at` snapshot token.

### `write_relationships`

```rust
pub async fn write_relationships(&mut self, updates: Vec<RelationshipUpdate>) -> Result<WriteRelationshipsResponse, ClientError>
```

Creates or deletes relationships atomically. Returns an optional `written_at` snapshot token.

### `read_relationships`

```rust
pub async fn read_relationships(&mut self, request: ReadRelationshipsRequest) -> Result<Vec<Relationship>, ClientError>
```

Reads relationships matching the given filter.

### `lookup_resources`

```rust
pub async fn lookup_resources(&mut self, request: LookupResourcesRequest) -> Result<Vec<String>, ClientError>
```

Finds all resource IDs of a given type that the subject has a permission on. Collects from a server-side stream.

### `lookup_subjects`

```rust
pub async fn lookup_subjects(&mut self, request: LookupSubjectsRequest) -> Result<Vec<SubjectRef>, ClientError>
```

Finds all subjects of a given type with a permission on a resource.

### `write_schema` / `read_schema`

```rust
pub async fn write_schema(&mut self, schema: &str, force: bool) -> Result<WriteSchemaResponse, ClientError>
pub async fn read_schema(&mut self) -> Result<Option<String>, ClientError>
```

Write or read the authorization schema. Set `force: true` to allow breaking changes.

### `expand_permission_tree`

```rust
pub async fn expand_permission_tree(&mut self, request: ExpandPermissionTreeRequest) -> Result<ExpandPermissionTreeResponse, ClientError>
```

Expands the full permission tree for debugging.

## Consistency

The `Consistency` enum controls read consistency:

| Variant | Description |
|---------|-------------|
| `FullConsistency` | Read at the latest snapshot (strongest) |
| `MinimizeLatency` | Allow stale reads for lower latency |
| `AtLeastAsFresh(token)` | Read at least as fresh as the given snapshot |
| `AtExactSnapshot(token)` | Read at exactly the given snapshot |

## Error Handling

All methods return `Result<_, ClientError>`. Variants:

| Variant | Description |
|---------|-------------|
| `Connection(msg)` | Failed to connect to the server |
| `InvalidArgument(msg)` | Invalid request parameters |
| `NotFound(msg)` | Resource not found |
| `PermissionDenied(msg)` | Insufficient permissions |
| `Unauthenticated(msg)` | Missing or invalid API key |
| `FailedPrecondition(msg)` | Precondition failed (e.g., breaking schema change) |
| `Internal(msg)` | Internal server error |
| `Timeout` | Request deadline exceeded |
| `Status(status)` | Unmapped gRPC status |

```rust
use kraalzibar_client::ClientError;

match client.check_permission(request).await {
    Ok(resp) => println!("allowed: {}", resp.allowed),
    Err(ClientError::Unauthenticated(msg)) => eprintln!("auth failed: {msg}"),
    Err(ClientError::Timeout) => eprintln!("request timed out"),
    Err(e) => eprintln!("error: {e}"),
}
```

## Testing

Integration tests require a running kraalzibar server and are marked `#[ignore]`:

```bash
# Start the server
cargo run --release &

# Run integration tests
cargo test -p kraalzibar-client -- --ignored
```
