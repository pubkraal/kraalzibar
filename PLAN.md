# Kraalzibar: Web-Based Zanzibar-Inspired Authorization System

## Vision

Kraalzibar is a Zanzibar-inspired Relationship-Based Access Control (ReBAC)
system. It provides a server that stores relationship tuples, evaluates
permission checks via graph traversal, and exposes a gRPC + REST API. Client
SDKs in Go, Rust, and TypeScript make integration straightforward.

The server is written in Rust for performance and safety.

---

## Core Concepts

The system models authorization as a graph of relationships between objects and
subjects:

- **Tuple**: `object#relation@subject` (e.g., `object:readme#viewer@user:john`)
- **Namespace/Type**: a category of object (e.g., `object`, `category`, `user`)
- **Relation**: a named edge type within a namespace (e.g., `viewer`, `editor`, `owner`)
- **Permission**: a computed relation derived from unions, intersections, exclusions, and
  indirection (arrow/tuple-to-userset)
- **Consistency Token**: an opaque snapshot token returned by writes, used in reads/checks
  to prevent the new-enemy problem

---

## Architecture Overview

```
                          +-------------------+
                          |   Schema (DSL)    |
                          |  parsed at boot   |
                          |  or via API       |
                          +---------+---------+
                                    |
         +-----------+    +---------v---------+    +-------------+
         |   SDKs    |--->|   Kraalzibar      |--->|  Storage    |
         | Go / Rust |    |   Server (Rust)   |    |  Backend    |
         | TypeScript |   |                   |    | (Postgres)  |
         +-----------+    |  gRPC + REST API  |    +-------------+
                          |  Graph Engine     |
                          |  Cache Layer      |
                          +-------------------+
```

---

## Requirements

### Multi-Tenancy

- **Isolation model**: Separate PostgreSQL schema per tenant. Each tenant gets
  its own set of tables (`relation_tuples`, `schema_versions`, etc.) within a
  dedicated PG schema namespace (e.g., `tenant_<uuid>`).
- **Tenant definition**: A tenant is a workspace with its own authorization
  schema (DSL), relationship tuples, and users/assets. Flat hierarchy — no
  sub-tenants.
- **Cross-tenant access**: Not supported. A single API request is always scoped
  to exactly one tenant, determined by the API key.
- **Tenant registry**: Stored in a shared `public` PG schema (tenant metadata,
  API key hashes).
- **Scale target**: Dozens of tenants, hundreds of users per tenant, thousands
  of assets per tenant.

### Authorization Schema Lifecycle

- **Atomic changes**: Schema updates are all-or-nothing. A `WriteSchema` call
  either fully applies the new schema or fails entirely.
- **Breaking change warnings (two-step flow)**: The system detects breaking
  changes (removed types or relations that have existing tuples, changed
  subject type constraints) and returns them as warnings. By default, a
  `WriteSchema` call with breaking changes is **rejected**. The caller must
  explicitly pass `force: true` to apply a schema with breaking changes. This
  enables a dry-run workflow: call `WriteSchema` without `force`, inspect
  warnings, then call again with `force: true` if acceptable.
- **Breaking change definitions**:
  - Removing a type that has existing tuples → breaking
  - Removing a relation that has existing tuples → breaking
  - Changing allowed subject types for a relation when existing tuples would
    violate the new constraint → breaking
  - Renaming a type or relation → breaking (effectively remove + add)
  - Adding new types, relations, or permissions → safe
  - Changing a permission's rewrite rule → safe (no stored data affected)
- **Orphan cleanup**: When a schema change removes types or relations, orphaned
  tuples are marked for deletion and cleaned up asynchronously by a background
  garbage collector task.
- **Version history**: Out of scope for initial implementation.

### Authentication

- **Mechanism**: API key per tenant, passed via `Authorization: Bearer <key>`
  header.
- **Key storage**: API keys are stored as hashed values (argon2) in the shared
  `public` schema. The raw key is returned once on creation and never stored.
- **Key-to-tenant mapping**: Each API key maps to exactly one tenant. All
  requests authenticated with that key operate within that tenant's PG schema.
- **Callers**: Backend services only (service-to-service). No browser or
  end-user direct access.
- **Future scope**: OAuth2/OIDC support, tiered permissions (admin vs. normal).

### Consistency Model

- **Snapshot tokens**: Write operations return an opaque snapshot token.
  Read/check operations accept an optional `at_least_as_fresh` token to ensure
  they see at least that write.
- **Default behavior**: Without a token, reads use the latest available snapshot
  (minimize latency mode).
- **Guarantee level**: Causal consistency within a tenant. No cross-tenant
  consistency requirements.
- **Token scope**: Tokens are per-tenant. A token from tenant A is meaningless
  in tenant B.

### Security & Observability

- **Deployment model**: Shared multi-tenant deployment.
- **Isolation severity**: Standard — data leaks between tenants are bugs to
  fix, not regulatory catastrophes.
- **Audit logging**: Immutable structured logs to stdout (12-factor app). All
  schema changes and relationship writes are logged with tenant context. Logs
  must support filtering by tenant ID, operation type, and timestamp.
- **Rate limiting**: Handled externally by the load balancer. Not implemented
  in the service itself.

### Graph Engine Limits

- **Max traversal depth**: Default 6, configurable per deployment. Recommended
  range: 5–8.
- **Schema size limits**: Defaults with manually adjustable hard limits:
  - Max types per schema: 50
  - Max relations per type: 30
  - Max permissions per type: 30
- **Lookup operation limits**: Hard limit on results returned by
  `LookupResources` and `LookupSubjects` (default: 1000, adjustable).
- **Concurrent branches**: Max concurrent graph branches per check request
  (default: 10).

### Operations (Initial Scope)

- **Tenant onboarding**: Out of scope for the service API. Handled by a
  separate admin binary or manual process.
- **Tenant suspension/deletion**: Manual intervention only.

---

## Phase 1: Core Data Model & Storage

**Goal**: Define the tuple data model, schema types, and storage layer.

### 1.1 Tuple Representation

Define the core relationship tuple struct:

```
object_type:  String    (namespace, e.g. "object")
object_id:    String    (e.g. "readme")
relation:     String    (e.g. "viewer")
subject_type: String    (e.g. "user")
subject_id:   String    (e.g. "john")
subject_relation: Option<String>  (e.g. Some("member") for usersets, None for direct)
```

### 1.2 Schema / Namespace Configuration

Define a DSL for declaring types, relations, and permissions. Syntax inspired by
SpiceDB but kept minimal:

```
definition user {}

definition group {
    relation member: user | group#member
}

definition category {
    relation owner: user
    relation editor: user | group#member
    relation viewer: user | group#member

    permission can_edit = owner + editor
    permission can_view = can_edit + viewer
}

definition object {
    relation parent: category
    relation owner: user
    relation editor: user | group#member
    relation viewer: user | group#member

    permission can_edit = owner + editor + parent->can_edit
    permission can_view = can_edit + viewer + parent->can_view
}
```

Operators:
- `+` union (OR)
- `&` intersection (AND)
- `-` exclusion (BUT NOT)
- `->` arrow / tuple-to-userset (follow relation, then evaluate)

#### Schema Write Flow

When a tenant calls `WriteSchema`:

1. Parse the new DSL definition
2. Validate against schema size limits (max types, relations, permissions)
3. Compare against the current schema to detect breaking changes:
   - Removed types with existing tuples
   - Removed relations with existing tuples
   - Changed subject type constraints that invalidate existing tuples
4. If breaking changes detected and `force` is not set → reject with warnings
5. If no breaking changes, or `force: true` → apply atomically (single tx)
6. Return response (with warnings if `force` was used)
7. Enqueue orphaned tuple cleanup for the background GC task

The GC task runs periodically and marks orphaned tuples (those referencing
types or relations no longer in the schema) with `deleted_tx_id` set to the
current transaction ID.

### 1.3 Storage Backend

Start with PostgreSQL. Abstract behind a trait so other backends can be added.

The database uses two layers:
- **Shared schema** (`public`): tenant registry and API key storage
- **Per-tenant schemas** (`tenant_<uuid>`): relationship tuples and schema
  definitions, fully isolated per tenant

#### Shared Schema (public)

```sql
CREATE TABLE tenants (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL UNIQUE,
    pg_schema   TEXT NOT NULL UNIQUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id   UUID NOT NULL REFERENCES tenants(id),
    key_hash    TEXT NOT NULL UNIQUE,  -- argon2 hash
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at  TIMESTAMPTZ
);
```

#### Per-Tenant Schema (tenant_<uuid>)

Created dynamically when a tenant is provisioned. Each tenant gets an identical
set of tables in their own PG schema namespace:

```sql
CREATE SCHEMA tenant_<uuid>;

CREATE TABLE tenant_<uuid>.relation_tuples (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    object_type     TEXT NOT NULL,
    object_id       TEXT NOT NULL,
    relation        TEXT NOT NULL,
    subject_type    TEXT NOT NULL,
    subject_id      TEXT NOT NULL,
    subject_relation TEXT,
    created_tx_id   BIGINT NOT NULL,
    deleted_tx_id   BIGINT NOT NULL DEFAULT 9223372036854775807,
    UNIQUE(object_type, object_id, relation, subject_type, subject_id,
           subject_relation, deleted_tx_id)
);

CREATE INDEX idx_tuples_lookup ON tenant_<uuid>.relation_tuples
    (object_type, object_id, relation, deleted_tx_id);

CREATE INDEX idx_tuples_reverse ON tenant_<uuid>.relation_tuples
    (subject_type, subject_id, subject_relation, deleted_tx_id);

CREATE TABLE tenant_<uuid>.schema_definitions (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    definition  TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE SEQUENCE tenant_<uuid>.tx_id_seq;
```

The `deleted_tx_id` column enables MVCC-style snapshot reads:
- Active tuples have `deleted_tx_id = MAX_BIGINT`
- Deleted tuples get `deleted_tx_id = <tx_id of deletion>`
- Reads at a given snapshot filter:
  `created_tx_id <= snapshot AND deleted_tx_id > snapshot`

Each tenant has its own `tx_id_seq` sequence — snapshot tokens are scoped to a
tenant and carry no cross-tenant meaning.

### 1.4 Storage Trait

The store trait is tenant-unaware. A tenant-scoped store instance is created
per request via a factory, which sets the PG `search_path` to the tenant's
schema:

```rust
trait RelationshipStore {
    async fn write(&self, writes: Vec<TupleWrite>, deletes: Vec<TupleFilter>) -> Result<SnapshotToken>;
    async fn read(&self, filter: TupleFilter, snapshot: Option<SnapshotToken>) -> Result<Vec<Tuple>>;
    async fn snapshot(&self) -> Result<SnapshotToken>;
}

trait StoreFactory {
    fn for_tenant(&self, tenant_id: &TenantId) -> impl RelationshipStore;
}
```

### Deliverables - Phase 1

- [ ] `Tuple` and related domain types
- [ ] Schema DSL parser (using `nom` or `pest`)
- [ ] In-memory schema representation (types, relations, permissions with rewrite rules)
- [ ] Schema validation (size limits, breaking change detection)
- [ ] PostgreSQL storage implementation (shared + per-tenant schemas)
- [ ] `StoreFactory` for tenant-scoped store creation
- [ ] In-memory storage implementation (for tests)
- [ ] Snapshot token (consistency token) generation and validation
- [ ] Background GC task for orphaned tuple cleanup
- [ ] Database migrations (shared schema + per-tenant template)

---

## Phase 2: Graph Engine (Check, Expand)

**Goal**: Implement the permission evaluation engine.

### 2.1 Check Algorithm

Given `Check(object_type, object_id, permission, subject_type, subject_id)`:

1. Look up the permission's rewrite rule from the schema
2. Evaluate the rule recursively:
   - **this/self**: query storage for a direct tuple match
   - **computed_userset**: recurse with a different relation on the same object
   - **tuple_to_userset (arrow)**: query storage for tuples on the tupleset
     relation, then for each result, recurse to check the computed relation
     on the target object
   - **union**: return `true` if any child returns `true` (short-circuit)
   - **intersection**: return `true` only if all children return `true`
     (short-circuit on first `false`)
   - **exclusion**: return `true` if the base returns `true` and the subtract
     returns `false`
3. All storage reads use the provided snapshot token for consistency

### 2.2 Optimizations

- **Short-circuit evaluation**: stop evaluating union branches on first `true`
- **Concurrent branch evaluation**: evaluate independent branches (e.g., children
  of a union) concurrently using `tokio::spawn` / `tokio::select!`
- **Cycle detection**: track visited `(object, relation)` pairs per check to
  prevent infinite loops from circular group memberships
- **Depth limiting**: configurable max recursion depth (default: 6)
- **Request-level memoization**: cache intermediate results within a single
  check request to avoid redundant sub-checks

### 2.3 Expand

Returns the full userset tree for `(object, relation)` as a tree structure.
Same traversal as Check but returns the full tree instead of a boolean.
Primarily for debugging and explainability.

### Deliverables - Phase 2

- [ ] `CheckEngine` with recursive graph traversal
- [ ] Short-circuit evaluation for union/intersection/exclusion
- [ ] Concurrent branch evaluation
- [ ] Cycle detection and depth limiting
- [ ] Request-level memoization
- [ ] `ExpandEngine` returning a userset tree
- [ ] Comprehensive tests for graph traversal edge cases

---

## Phase 3: Server & API

**Goal**: Expose the system via gRPC and REST APIs.

### 3.1 API Design

Define APIs via Protocol Buffers (`.proto` files). Use `tonic` for gRPC and
provide a REST gateway (either via `tonic-web` + grpc-web, or a separate
`axum` REST layer that calls the same service).

#### RPCs

```protobuf
service KraalzibarService {
    // Write relationship tuples
    rpc WriteRelationships(WriteRelationshipsRequest)
        returns (WriteRelationshipsResponse);

    // Read stored relationship tuples
    rpc ReadRelationships(ReadRelationshipsRequest)
        returns (stream ReadRelationshipsResponse);

    // Check a permission
    rpc CheckPermission(CheckPermissionRequest)
        returns (CheckPermissionResponse);

    // Expand a permission tree (for debugging)
    rpc ExpandPermissionTree(ExpandPermissionTreeRequest)
        returns (ExpandPermissionTreeResponse);

    // List objects a subject can access
    rpc LookupResources(LookupResourcesRequest)
        returns (stream LookupResourcesResponse);

    // List subjects with a permission on an object
    rpc LookupSubjects(LookupSubjectsRequest)
        returns (stream LookupSubjectsResponse);

    // Watch for relationship changes
    rpc Watch(WatchRequest) returns (stream WatchResponse);

    // Write a schema
    rpc WriteSchema(WriteSchemaRequest) returns (WriteSchemaResponse);

    // Read the current schema
    rpc ReadSchema(ReadSchemaRequest) returns (ReadSchemaResponse);
}
```

#### Consistency specification

Every read/check request accepts an optional consistency parameter:

```protobuf
message Consistency {
    oneof requirement {
        bool full_consistency = 1;
        bool minimize_latency = 2;
        ZedToken at_least_as_fresh = 3;
        ZedToken at_exact_snapshot = 4;
    }
}

message ZedToken {
    string token = 1;
}
```

### 3.2 REST API

Provide a JSON REST API alongside gRPC. Routes:

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/relationships/write` | Write tuples |
| POST | `/v1/relationships/read` | Read tuples |
| POST | `/v1/permissions/check` | Check permission |
| POST | `/v1/permissions/expand` | Expand permission tree |
| POST | `/v1/permissions/resources` | Lookup resources |
| POST | `/v1/permissions/subjects` | Lookup subjects |
| GET  | `/v1/watch` | Watch (SSE stream) |
| POST | `/v1/schema` | Write schema |
| GET  | `/v1/schema` | Read schema |
| GET  | `/healthz` | Health check |

All POST for reads/checks (rather than GET) because request bodies carry
structured query parameters that don't map well to query strings.

### 3.3 Server Infrastructure

- **Framework**: `tonic` for gRPC, `axum` for REST
- **Configuration**: environment variables + config file (TOML)
- **Logging**: `tracing` crate with structured logging
- **Metrics**: Prometheus metrics via `metrics` crate
- **Health checks**: readiness and liveness probes
- **Graceful shutdown**: handle SIGTERM/SIGINT

### 3.4 Authentication & Multi-tenancy

- API key authentication via `Authorization: Bearer <key>` header
- Key is looked up in the shared `public.api_keys` table (hashed with argon2)
- On successful auth, the middleware resolves the tenant and sets the PG
  `search_path` to the tenant's schema for the duration of the request
- All subsequent storage operations are automatically scoped to that tenant
- Future: OAuth2/OIDC token validation, tiered permissions

#### Request Flow

```
1. Request arrives with Authorization header
2. Middleware hashes the key, looks up in public.api_keys
3. If valid and not revoked → resolve tenant_id → resolve pg_schema
4. Set search_path to tenant's PG schema
5. Proceed to handler (all storage ops are now tenant-scoped)
6. On error/invalid key → 401 Unauthorized
```

### Deliverables - Phase 3

- [ ] Protocol Buffer definitions
- [ ] `tonic` gRPC server implementing all RPCs
- [ ] `axum` REST server with matching routes
- [ ] Request validation and error handling
- [ ] Consistency parameter handling
- [ ] Streaming responses for Read, LookupResources, LookupSubjects, Watch
- [ ] API key authentication middleware (argon2 hash lookup, tenant resolution)
- [ ] Tenant-scoped request pipeline (API key → tenant → PG schema)
- [ ] Structured audit logging (schema writes, relationship writes, auth events)
- [ ] Server configuration (env vars + TOML)
- [ ] Prometheus metrics (per-RPC, per-tenant)
- [ ] Health check endpoints
- [ ] Docker image / Dockerfile

---

## Phase 4: Caching & Performance

**Goal**: Add caching layers for production-grade performance.

### 4.1 Cache Layers

1. **Request-level**: already implemented in Phase 2 (memoization within a
   single Check)
2. **Application-level check cache**: LRU cache keyed on
   `(object, relation, subject, snapshot_token)`. Safe because a snapshot is
   immutable.
3. **Schema cache**: cache parsed schema in memory, invalidate on schema write
4. **Tuple read cache**: optional LRU cache for hot tuple lookups

### 4.2 Connection Pooling

Use `deadpool-postgres` or `sqlx` connection pool with configurable size.

### 4.3 Benchmarking

- Criterion benchmarks for the graph engine
- Load tests with realistic tuple graphs (e.g., 1M tuples, hierarchical
  category hierarchy)
- Target: < 5ms p99 for Check on a warm cache with moderate graph depth

### Deferred TODOs from Phase 3 Code Review

These were identified during the Phase 3 PR review and deferred to later phases:

#### Security & Auth
- [ ] Wire auth middleware — replace hardcoded nil tenant ID with real API key
  authentication (Phase 3.4 scope, currently stubbed)
- [ ] Use env var for DB password in docker-compose instead of hardcoded value
- [ ] Consider higher entropy for API key secret (currently 32 bytes / 256 bits)
- [ ] Custom `Debug` impl for `DatabaseConfig` to redact credentials from logs

#### Performance & Correctness
- [ ] Optimize `lookup_resources` — currently fetches all tuples matching the
  type, then filters. Replace with cursor-based pagination or indexed query
- [ ] Return snapshot tokens in gRPC `ExpandPermissionTree`, `ReadRelationships`,
  and `WriteRelationships` responses (currently only `CheckPermission` returns
  `checked_at`)

#### Code Quality
- [ ] Fix unsafe `env::set_var`/`remove_var` in config tests — use `serial_test`
  crate or refactor config loading to accept env as parameter
- [ ] Align REST/gRPC error status for breaking schema changes — REST returns
  409 Conflict, gRPC returns FAILED_PRECONDITION. Pick one semantic and align
- [ ] Add integration tests for the server crate (end-to-end gRPC/REST against
  a real running server)

### Deliverables - Phase 4

- [ ] Application-level check result cache
- [ ] Schema cache with invalidation
- [ ] Connection pool tuning
- [ ] Criterion benchmarks
- [ ] Load test suite

---

## Phase 5: SDKs

**Goal**: Provide idiomatic client SDKs in Go, Rust, and TypeScript.

### 5.1 SDK Design Principles

All SDKs share the same design philosophy:
- **Generated from protobuf**: use the `.proto` files as the source of truth
  for types and service definitions
- **Thin typed wrappers**: provide idiomatic convenience over raw gRPC stubs
- **Connection management**: handle gRPC channel creation, retries, deadlines
- **Consistency helpers**: make it easy to pass and store consistency tokens

### 5.2 Go SDK

**Module**: `github.com/kraalzibar/kraalzibar-go`

Generate gRPC stubs with `protoc-gen-go` and `protoc-gen-go-grpc`. Wrap with
an idiomatic client:

```go
package kraalzibar

type Client struct { ... }

func NewClient(target string, opts ...ClientOption) (*Client, error)

func (c *Client) CheckPermission(ctx context.Context, req *CheckPermissionRequest) (*CheckPermissionResponse, error)
func (c *Client) WriteRelationships(ctx context.Context, req *WriteRelationshipsRequest) (*WriteRelationshipsResponse, error)
func (c *Client) ReadRelationships(ctx context.Context, req *ReadRelationshipsRequest) (ReadRelationshipsStream, error)
func (c *Client) LookupResources(ctx context.Context, req *LookupResourcesRequest) (LookupResourcesStream, error)
func (c *Client) LookupSubjects(ctx context.Context, req *LookupSubjectsRequest) (LookupSubjectsStream, error)
func (c *Client) WriteSchema(ctx context.Context, schema string) (*WriteSchemaResponse, error)
func (c *Client) ReadSchema(ctx context.Context) (string, error)

type ClientOption func(*clientConfig)
func WithInsecure() ClientOption
func WithAPIKey(key string) ClientOption
func WithDeadline(d time.Duration) ClientOption
```

**Testing**: integration tests against a real or containerized server.

### 5.3 Rust SDK

**Crate**: `kraalzibar-client`

Generate gRPC stubs with `tonic-build`. Provide an async client:

```rust
pub struct KraalzibarClient { ... }

impl KraalzibarClient {
    pub async fn connect(endpoint: &str, options: ClientOptions) -> Result<Self>;
    pub async fn check_permission(&self, request: CheckPermissionRequest) -> Result<CheckPermissionResponse>;
    pub async fn write_relationships(&self, request: WriteRelationshipsRequest) -> Result<WriteRelationshipsResponse>;
    pub async fn read_relationships(&self, request: ReadRelationshipsRequest) -> Result<impl Stream<Item = Result<Tuple>>>;
    pub async fn lookup_resources(&self, request: LookupResourcesRequest) -> Result<impl Stream<Item = Result<ObjectReference>>>;
    pub async fn lookup_subjects(&self, request: LookupSubjectsRequest) -> Result<impl Stream<Item = Result<SubjectReference>>>;
    pub async fn write_schema(&self, schema: &str) -> Result<WriteSchemaResponse>;
    pub async fn read_schema(&self) -> Result<String>;
}
```

### 5.4 TypeScript SDK

**Package**: `@kraalzibar/client`

Use `@grpc/grpc-js` or `nice-grpc` for gRPC, or provide a REST-only client for
simpler environments (browsers, edge runtimes):

```typescript
type KraalzibarClient = {
    checkPermission(request: CheckPermissionRequest): Promise<CheckPermissionResponse>;
    writeRelationships(request: WriteRelationshipsRequest): Promise<WriteRelationshipsResponse>;
    readRelationships(request: ReadRelationshipsRequest): AsyncIterable<Tuple>;
    lookupResources(request: LookupResourcesRequest): AsyncIterable<ObjectReference>;
    lookupSubjects(request: LookupSubjectsRequest): AsyncIterable<SubjectReference>;
    writeSchema(schema: string): Promise<WriteSchemaResponse>;
    readSchema(): Promise<string>;
};

const createClient: (options: ClientOptions) => KraalzibarClient;

type ClientOptions = {
    target: string;
    apiKey?: string;
    transport?: "grpc" | "rest";
};
```

The TypeScript SDK supports both gRPC (for Node.js server environments) and
REST (for browsers and edge runtimes).

### 5.5 Shared Test Suite

Write a transport-agnostic integration test suite as a set of scenarios
(in a YAML or JSON fixture format) that all SDKs execute against a running
server. This ensures behavioral parity across SDKs.

### Deliverables - Phase 5

- [ ] Protobuf code generation setup for Go, Rust, TypeScript
- [ ] Go SDK with idiomatic wrapper, tests, and README
- [ ] Rust SDK with async client, tests, and README
- [ ] TypeScript SDK with gRPC + REST transports, tests, and README
- [ ] Shared integration test scenarios
- [ ] CI pipeline running SDK tests against a containerized server

---

## Phase 6: Operations & Deployment

**Goal**: Make the system production-ready to deploy.

### 6.1 Deployment

- **Docker**: multi-stage Dockerfile producing a minimal image
- **Docker Compose**: server + PostgreSQL for local development
- **Helm chart**: for Kubernetes deployment (optional, later)
- **Database migrations**: embedded in the binary, run on startup with a
  `migrate` subcommand

### 6.2 Observability

- **Logging**: structured JSON logs to stdout via `tracing` + `tracing-subscriber`
  (12-factor app). All log entries include `tenant_id` where applicable.
- **Audit logging**: immutable structured log events for:
  - Schema writes (includes tenant_id, before/after hash, warnings)
  - Relationship writes (includes tenant_id, operation count, snapshot token)
  - Authentication events (success/failure, key ID, tenant_id)
  - Audit logs are written to stdout as structured JSON, filterable by
    `tenant_id`, `operation`, and `timestamp`
- **Metrics**: Prometheus endpoint at `/metrics`
  - Request count, latency histograms (by RPC, by tenant)
  - Check cache hit/miss rates
  - Storage query latency
  - Active connections
- **Tracing**: OpenTelemetry integration for distributed tracing

### 6.3 Configuration

```toml
[server]
grpc_port = 50051
http_port = 8080
max_request_size = "4MB"

[storage]
backend = "postgres"
connection_string = "postgres://..."
pool_size = 20

[cache]
check_cache_size = 10000
schema_cache_ttl = "5m"

[engine]
max_depth = 6
concurrent_branches = 10
max_types_per_schema = 50
max_relations_per_type = 30
max_permissions_per_type = 30
lookup_result_limit = 1000
```

### Deliverables - Phase 6

- [ ] Dockerfile (multi-stage)
- [ ] docker-compose.yml (server + Postgres)
- [ ] Embedded database migrations
- [ ] OpenTelemetry tracing integration
- [ ] Prometheus metrics
- [ ] Configuration file support

---

## Project Structure

```
kraalzibar/
├── Cargo.toml                    # Workspace root
├── PLAN.md
├── proto/
│   └── kraalzibar/
│       └── v1/
│           ├── core.proto        # Shared types (ObjectRef, SubjectRef, Tuple, etc.)
│           ├── permission.proto  # Check, Expand, Lookup RPCs
│           ├── relationship.proto# Write, Read RPCs
│           └── schema.proto      # WriteSchema, ReadSchema RPCs
├── crates/
│   ├── kraalzibar-core/          # Domain types, schema parser, graph engine
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── tuple.rs          # Tuple types
│   │       ├── schema/
│   │       │   ├── mod.rs
│   │       │   ├── parser.rs     # DSL parser
│   │       │   └── types.rs      # Schema AST types
│   │       └── engine/
│   │           ├── mod.rs
│   │           ├── check.rs      # Check algorithm
│   │           └── expand.rs     # Expand algorithm
│   ├── kraalzibar-storage/       # Storage trait + implementations
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── postgres.rs
│   │       └── memory.rs
│   ├── kraalzibar-server/        # gRPC + REST server binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs
│   │       ├── grpc.rs
│   │       ├── rest.rs
│   │       ├── config.rs
│   │       └── auth.rs
│   └── kraalzibar-client/        # Rust SDK
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs
├── sdks/
│   ├── go/                       # Go SDK
│   │   ├── go.mod
│   │   ├── client.go
│   │   └── client_test.go
│   └── typescript/               # TypeScript SDK
│       ├── package.json
│       ├── tsconfig.json
│       ├── src/
│       │   └── index.ts
│       └── tests/
│           └── client.test.ts
├── tests/
│   └── integration/              # Shared integration test fixtures
│       └── scenarios.yaml
├── docker/
│   ├── Dockerfile
│   └── docker-compose.yml
└── migrations/
    └── 001_initial.sql
```

---

## Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Server language | Rust | Performance, safety, existing project setup |
| gRPC framework | `tonic` | De facto Rust gRPC, excellent async support |
| REST framework | `axum` | Same Tokio runtime as tonic, ergonomic |
| Schema parser | `pest` or `nom` | Rust parser combinator libraries |
| PostgreSQL driver | `sqlx` | Async, compile-time checked queries |
| Serialization | Protocol Buffers | Language-agnostic, gRPC native |
| Caching | `moka` | High-performance concurrent cache for Rust |
| Logging | `tracing` | Structured, async-aware logging |
| Metrics | `metrics` + `metrics-exporter-prometheus` | Prometheus-compatible |
| Go SDK codegen | `protoc-gen-go-grpc` | Standard Go gRPC tooling |
| TS SDK codegen | `ts-proto` or `nice-grpc` | TypeScript-first protobuf codegen |
| CI | GitHub Actions | Standard for open-source |

---

## Implementation Order

The phases above are presented in dependency order. Within each phase, work
follows TDD: write a failing test, make it pass, refactor if valuable, commit.

1. **Phase 1** - Data model + storage (~foundation)
2. **Phase 2** - Graph engine (~the brain)
3. **Phase 3** - Server + API (~the interface)
4. **Phase 4** - Caching + performance (~production readiness)
5. **Phase 5** - SDKs (~developer experience)
6. **Phase 6** - Operations + deployment (~ship it)

Phases 4 and 5 can partially overlap since the SDKs can be built against the
Phase 3 server while caching is added in parallel.

---

## Open Questions & Future Considerations

These are explicitly out of scope for the initial implementation but worth
noting for later:

- **Schema version history**: ability to view and roll back to previous schema
  versions per tenant.
- **OAuth2/OIDC authentication**: token-based auth alongside API keys.
- **Permission tiers**: admin vs. normal API key privileges (e.g., only admins
  can write schemas).
- **Tenant self-service onboarding**: API for creating/managing tenants
  (currently manual/admin binary).
- **Tenant suspension and deletion**: soft delete with retention, crypto-shredding.
- **Caveats/Conditions**: conditional relationships with runtime context
  (e.g., IP allowlists, time-based access). SpiceDB supports this; consider
  adding after the core is stable.
- **Global distribution**: CockroachDB or Spanner backend for multi-region.
- **Watch API**: real-time streaming of tuple changes. Important for cache
  invalidation in distributed deployments.
- **Admin UI**: web interface for managing schemas and browsing tuples.
- **Playground**: interactive tool for testing schemas and permissions (similar
  to the Authzed playground).
- **Rate limiting in-service**: per-API-key rate limiting (currently external
  via load balancer).
- **Batched checks**: check multiple permissions in a single request.
- **RBAC bridge**: convenience layer mapping traditional roles to ReBAC tuples.
