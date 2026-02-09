# Kraalzibar

A Zanzibar-inspired Relationship-Based Access Control (ReBAC) authorization
server written in Rust.

Kraalzibar stores relationship tuples, evaluates permission checks via graph
traversal, and exposes gRPC + REST APIs. Each tenant gets a fully isolated
authorization schema and data space.

## Status

Early development — not yet usable.

## Features (Planned)

- **Relationship tuples**: store and query `object#relation@subject` relationships
- **Schema DSL**: define types, relations, and computed permissions per tenant
- **Permission checks**: evaluate access via recursive graph traversal
- **Multi-tenancy**: isolated PostgreSQL schema per tenant
- **gRPC + REST APIs**: dual protocol support
- **Snapshot consistency**: tokens to prevent the new-enemy problem

## Requirements

- Rust 1.92+ (edition 2024)
- PostgreSQL 16+

## Getting Started

### Build

```bash
cargo build
```

### Test

```bash
# Unit and in-memory tests
cargo test

# PostgreSQL integration tests (requires Docker)
docker compose -f docker/docker-compose.yml up -d
cargo test -- --ignored
```

### Project Structure

```
crates/
  kraalzibar-core/      # Domain types, schema DSL parser, validation
  kraalzibar-storage/   # Storage traits, in-memory + PostgreSQL backends
docker/
  docker-compose.yml    # PostgreSQL 16 for local development
```

## License

TBD
