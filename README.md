# Kraalzibar

A Zanzibar-inspired Relationship-Based Access Control (ReBAC) authorization
server written in Rust.

Kraalzibar stores relationship tuples, evaluates permission checks via graph
traversal, and exposes gRPC + REST APIs. Each tenant gets a fully isolated
authorization schema and data space.

## Features

- **Relationship tuples** — store and query `object#relation@subject` relationships
- **Schema DSL** — define types, relations, and computed permissions per tenant
- **Permission checks** — evaluate access via recursive graph traversal (union, intersection, exclusion, arrow)
- **Multi-tenancy** — isolated PostgreSQL schema per tenant
- **gRPC + REST APIs** — dual protocol support with consistent behavior
- **Snapshot consistency** — tokens to prevent the new-enemy problem
- **Caching** — schema cache (TTL) and check cache (snapshot-keyed) with moka
- **Prometheus metrics** — request counters, latency histograms, cache hit/miss rates at `/metrics`
- **Audit logging** — structured events for schema writes, relationship writes, and authentication via `tracing`
- **OpenTelemetry tracing** — optional OTLP export with per-request spans (feature-gated)
- **Client SDKs** — Rust (gRPC), Go (gRPC), TypeScript (REST)

## Requirements

- Rust 1.85+ (edition 2024)
- PostgreSQL 16+ (for persistent storage)
- Docker (for local development stack)

## Getting Started

### Build

```bash
cargo build --release
```

### Run

The server operates in two modes based on whether a database URL is configured:

**Dev mode** (no database URL) — in-memory storage, no authentication:

```bash
# Default config starts in dev mode
cargo run --release

# Or explicitly with no database section in TOML
cargo run --release -- --config config.toml
```

**Production mode** (database URL configured) — PostgreSQL storage, API key auth:

```bash
# Set database URL in config.toml or via environment variable
KRAALZIBAR_DATABASE_URL="postgresql://user:pass@localhost:5432/kraalzibar" cargo run --release
```

### CLI Subcommands

```bash
# Start the server (default)
kraalzibar-server serve --config config.toml

# Run database migrations
kraalzibar-server migrate --config config.toml

# Provision a new tenant
kraalzibar-server provision-tenant --name acme --config config.toml

# Create an API key for a tenant
kraalzibar-server create-api-key --tenant-name acme --config config.toml
```

### Test

```bash
# Unit and in-memory tests (281 Rust tests)
cargo test --workspace

# PostgreSQL integration tests (requires Docker)
cargo test -- --ignored

# Go SDK tests
cd sdks/go && go test ./...

# TypeScript SDK tests
cd sdks/typescript && npm test
```

### Docker Compose

The full observability stack includes PostgreSQL, Prometheus, Jaeger, and Grafana:

```bash
cd docker && docker compose up -d
```

| Service    | URL                        | Purpose                    |
|------------|----------------------------|----------------------------|
| gRPC       | `localhost:50051`          | gRPC API                   |
| REST       | `localhost:8080`           | REST API + `/metrics`      |
| Prometheus | `http://localhost:9090`    | Metrics dashboard          |
| Jaeger     | `http://localhost:16686`   | Distributed traces         |
| Grafana    | `http://localhost:3000`    | Dashboards (admin/admin)   |
| PostgreSQL | `localhost:5432`           | Persistent storage         |

### Configuration

Configuration uses TOML with environment variable overrides (`KRAALZIBAR_` prefix):

```toml
[grpc]
host = "0.0.0.0"
port = 50051

[rest]
host = "0.0.0.0"
port = 8080

[database]
url = "postgresql://kraalzibar:kraalzibar@localhost:5432/kraalzibar"
max_connections = 10

[cache]
schema_cache_capacity = 100
schema_cache_ttl_seconds = 30
check_cache_capacity = 10000
check_cache_ttl_seconds = 300

[tracing]
enabled = false
otlp_endpoint = "http://localhost:4317"
service_name = "kraalzibar"
sample_rate = 1.0

[log]
level = "info"
format = "pretty"  # or "json"
```

## Project Structure

```
crates/
  kraalzibar-core/      # Domain types, schema DSL parser, graph engine
  kraalzibar-storage/   # Storage traits, in-memory + PostgreSQL backends
  kraalzibar-server/    # gRPC + REST server, config, metrics, audit, CLI
  kraalzibar-client/    # Rust SDK (gRPC client)
sdks/
  go/                   # Go SDK (gRPC)
  typescript/           # TypeScript SDK (REST)
docker/
  docker-compose.yml    # Full local stack with observability
  prometheus.yml        # Prometheus scrape config
  grafana/              # Grafana datasource provisioning
proto/
  kraalzibar/v1/        # Protocol Buffer definitions
tests/
  integration/          # Shared integration test scenarios (YAML)
```

## Quick Start Guide

Without a database URL, the server runs in dev mode with in-memory storage and
no authentication. You can start making API calls immediately.

> **Note:** In dev mode, data is stored in memory and lost when the server
> restarts. Configure a database URL for persistent storage with API key
> authentication (see [Tenant Setup](#tenant-setup-postgresql) below).

### 1. Start the server

```bash
cargo run --release
```

The server starts on gRPC `:50051` and REST `:8080` by default.

### 2. Write an authorization schema

Define your types, relations, and permissions using the schema DSL:

```bash
curl -s -X POST http://localhost:8080/v1/schema \
  -H 'Content-Type: application/json' \
  -d '{
    "schema": "definition user {}\n\ndefinition document {\n  relation viewer: user\n  relation editor: user\n  permission view = viewer + editor\n  permission edit = editor\n}"
  }' | jq .
```

Response:

```json
{ "breaking_changes_overridden": false }
```

### 3. Create relationships

Grant `user:alice` the `editor` relation on `document:readme`, and `user:bob`
the `viewer` relation:

```bash
curl -s -X POST http://localhost:8080/v1/relationships/write \
  -H 'Content-Type: application/json' \
  -d '{
    "updates": [
      {
        "operation": "touch",
        "resource_type": "document",
        "resource_id": "readme",
        "relation": "editor",
        "subject_type": "user",
        "subject_id": "alice"
      },
      {
        "operation": "touch",
        "resource_type": "document",
        "resource_id": "readme",
        "relation": "viewer",
        "subject_type": "user",
        "subject_id": "bob"
      }
    ]
  }' | jq .
```

Response:

```json
{ "written_at": "1" }
```

### 4. Check permissions

Check if `user:alice` can `edit` `document:readme`:

```bash
curl -s -X POST http://localhost:8080/v1/permissions/check \
  -H 'Content-Type: application/json' \
  -d '{
    "resource_type": "document",
    "resource_id": "readme",
    "permission": "edit",
    "subject_type": "user",
    "subject_id": "alice"
  }' | jq .
```

```json
{ "allowed": true, "checked_at": "1" }
```

Alice can also `view` (because `permission view = viewer + editor`):

```bash
curl -s -X POST http://localhost:8080/v1/permissions/check \
  -H 'Content-Type: application/json' \
  -d '{
    "resource_type": "document",
    "resource_id": "readme",
    "permission": "view",
    "subject_type": "user",
    "subject_id": "alice"
  }' | jq .
```

```json
{ "allowed": true, "checked_at": "1" }
```

Bob can `view` but not `edit`:

```bash
curl -s -X POST http://localhost:8080/v1/permissions/check \
  -H 'Content-Type: application/json' \
  -d '{
    "resource_type": "document",
    "resource_id": "readme",
    "permission": "edit",
    "subject_type": "user",
    "subject_id": "bob"
  }' | jq .
```

```json
{ "allowed": false, "checked_at": "1" }
```

### 5. Query relationships

Look up which documents `user:alice` can `view`:

```bash
curl -s -X POST http://localhost:8080/v1/permissions/resources \
  -H 'Content-Type: application/json' \
  -d '{
    "resource_type": "document",
    "permission": "view",
    "subject_type": "user",
    "subject_id": "alice"
  }' | jq .
```

```json
{ "resource_ids": ["readme"], "looked_up_at": "1" }
```

Look up who can `view` `document:readme`:

```bash
curl -s -X POST http://localhost:8080/v1/permissions/subjects \
  -H 'Content-Type: application/json' \
  -d '{
    "resource_type": "document",
    "resource_id": "readme",
    "permission": "view",
    "subject_type": "user"
  }' | jq .
```

```json
{
  "subjects": [
    { "subject_type": "user", "subject_id": "alice" },
    { "subject_type": "user", "subject_id": "bob" }
  ],
  "looked_up_at": "1"
}
```

### Using the SDKs

The SDKs connect to the same server. No API key is needed for the default
in-memory mode:

**TypeScript:**

```typescript
import { KraalzibarClient } from "@kraalzibar/sdk";

const client = new KraalzibarClient({ baseUrl: "http://localhost:8080" });

const { allowed } = await client.checkPermission({
  resourceType: "document",
  resourceId: "readme",
  permission: "view",
  subjectType: "user",
  subjectId: "alice",
});
```

**Go:**

```go
client, _ := kraalzibar.NewClient("localhost:50051", kraalzibar.WithInsecure())
resp, _ := client.CheckPermission(ctx, kraalzibar.CheckPermissionRequest{
    Resource:   kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
    Permission: "view",
    Subject:    kraalzibar.DirectSubject("user", "alice"),
})
fmt.Println(resp.Allowed)
```

**Rust:**

```rust
use kraalzibar_client::KraalzibarClient;

let client = KraalzibarClient::connect("http://localhost:50051").await?;
let result = client.check_permission(
    "document", "readme", "view", "user", "alice", None,
).await?;
```

### Tenant Setup (PostgreSQL)

When running with PostgreSQL, each tenant gets an isolated database schema.
API keys authenticate requests and resolve to a tenant.

**1. Configure the database** in `config.toml`:

```toml
[database]
url = "postgresql://user:pass@localhost:5432/kraalzibar"
```

**2. Run migrations** to create the shared tables:

```bash
kraalzibar-server migrate --config config.toml
```

**3. Provision a tenant** using the CLI:

```bash
kraalzibar-server provision-tenant --name acme --config config.toml
# Output:
#   Tenant provisioned successfully
#     Name:      acme
#     Tenant ID: a1b2c3d4-e5f6-a7b8-c9d0-e1f2a3b4c5d6
```

**4. Create an API key** for the tenant:

```bash
kraalzibar-server create-api-key --tenant-name acme --config config.toml
# Output:
#   API key created successfully
#     Tenant:  acme
#     API Key: kraalzibar_a1b2c3d4_s5t6u7v8w9x0y1z2a3b4c5d6e7f8g9h0
#
#   Store this key securely — it will not be shown again.
```

Keys follow the format `kraalzibar_<8-char-id>_<32-char-secret>`. The secret
is hashed with Argon2 before storage.

**5. Start the server:**

```bash
kraalzibar-server serve --config config.toml
```

**6. Use the API key** in requests via the `Authorization` header:

```bash
curl -X POST http://localhost:8080/v1/permissions/check \
  -H 'Authorization: Bearer kraalzibar_a1b2c3d4_s5t6u7v8w9x0y1z2a3b4c5d6e7f8g9h0' \
  -H 'Content-Type: application/json' \
  -d '{ ... }'
```

Or with the SDKs:

```typescript
const client = new KraalzibarClient({
  baseUrl: "http://localhost:8080",
  apiKey: "kraalzibar_a1b2c3d4_s5t6u7v8w9x0y1z2a3b4c5d6e7f8g9h0",
});
```

## API Overview

### Permissions

- `CheckPermission` — does subject have permission on object?
- `ExpandPermissionTree` — expand the full permission tree for debugging
- `LookupResources` — which resources does a subject have access to?
- `LookupSubjects` — which subjects have access to a resource?

### Relationships

- `WriteRelationships` — create/delete relationship tuples (atomic)
- `ReadRelationships` — query tuples with filters and consistency

### Schema

- `WriteSchema` — update the authorization schema (rejects breaking changes unless `force: true`)
- `ReadSchema` — retrieve the current schema definition

## License

TBD
