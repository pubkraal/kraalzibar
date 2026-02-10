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
