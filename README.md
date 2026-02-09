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

_Installation and run instructions will be added as the project matures._

## License

TBD
