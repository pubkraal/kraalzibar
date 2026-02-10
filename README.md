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

```bash
# With default config (in-memory storage, pretty logs)
cargo run --release

# With a TOML config file
cargo run --release -- --config config.toml

# With environment variable overrides
KRAALZIBAR_GRPC_PORT=50051 KRAALZIBAR_REST_PORT=8080 cargo run --release
```

### CLI Subcommands

```bash
# Start the server (default)
kraalzibar-server serve

# Run database migrations
kraalzibar-server migrate --config config.toml
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

The server currently runs with in-memory storage and a single implicit tenant.
No authentication is required — you can start making API calls immediately.

> **Note:** Data is stored in memory and lost when the server restarts.
> PostgreSQL-backed persistent storage with per-tenant API key authentication
> is implemented in the storage and auth libraries but not yet wired into the
> server binary.

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
client, _ := kraalzibar.NewClient("localhost:50051")
allowed, _, _ := client.CheckPermission(ctx,
    "document", "readme", "view", "user", "alice")
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

> **Status:** The PostgreSQL storage backend and API key authentication are
> fully implemented in the library crates (`kraalzibar-storage`,
> `kraalzibar-server/auth`) but the server binary does not yet wire them up —
> it always uses in-memory storage with a single implicit tenant. The
> information below documents how the system is designed to work once the
> server startup is updated.

When running with PostgreSQL, each tenant gets an isolated database schema.
API keys authenticate requests and resolve to a tenant.

**1. Run database migrations** to create the shared `tenants` and `api_keys`
tables:

```bash
kraalzibar-server migrate --config config.toml
```

**2. Provision a tenant** by inserting into the `tenants` table and creating
its isolated schema. This is currently a database-level operation (an admin
CLI is planned):

```sql
-- Generate a tenant UUID
INSERT INTO tenants (id, name, pg_schema)
VALUES (
  'a1b2c3d4-e5f6-a7b8-c9d0-e1f2a3b4c5d6',
  'acme-corp',
  'tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6'
);
```

The `pg_schema` value must follow the format `tenant_<32-hex-chars>` (the
UUID with hyphens removed).

**3. Create the tenant's schema** with its tables, indexes, and sequence.
This is handled by `PostgresStoreFactory::provision_tenant()` in code, or
manually:

```sql
CREATE SCHEMA tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6;

CREATE TABLE tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6.relation_tuples (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    object_type     TEXT NOT NULL,
    object_id       TEXT NOT NULL,
    relation        TEXT NOT NULL,
    subject_type    TEXT NOT NULL,
    subject_id      TEXT NOT NULL,
    subject_relation TEXT,
    created_tx_id   BIGINT NOT NULL,
    deleted_tx_id   BIGINT NOT NULL DEFAULT 9223372036854775807,
    UNIQUE NULLS NOT DISTINCT (object_type, object_id, relation,
           subject_type, subject_id, subject_relation, deleted_tx_id)
);

CREATE TABLE tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6.schema_definitions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    definition  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE SEQUENCE tenant_a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6.tx_id_seq;
```

**4. Generate an API key.** Keys follow the format
`kraalzibar_<8-char-id>_<32-char-secret>`. The secret is hashed with Argon2
before storage. You can generate one using the Rust library:

```rust
use kraalzibar_server::auth::{generate_api_key, hash_secret};

let (full_key, secret) = generate_api_key();
let key_hash = hash_secret(&secret).unwrap();

// Store key_hash in the api_keys table, give full_key to the tenant
println!("API Key: {full_key}");
```

Then insert the hashed key:

```sql
INSERT INTO api_keys (tenant_id, key_hash)
VALUES (
  'a1b2c3d4-e5f6-a7b8-c9d0-e1f2a3b4c5d6',
  '$argon2id$v=19$m=19456,t=2,p=1$...'  -- output from hash_secret()
);
```

**5. Use the API key** in requests via the `Authorization` header:

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
