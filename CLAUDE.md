# Kraalzibar - Project Instructions

## Project

Zanzibar-inspired ReBAC authorization server written in Rust. See `PLAN.md`
for full architecture, requirements, and phased delivery plan.

## Workflow Rules

### TDD Is Mandatory (Red, Green, Refactor)

Every line of production code must be driven by a failing test. No exceptions.

1. **Red**: Write a failing test that describes the desired behavior
2. **Green**: Write the minimum code to make it pass
3. **Refactor**: Assess and improve structure while keeping tests green. Only
   refactor when it adds clear value. Commit after refactoring separately.

If there is no failing test demanding the code, do not write the code.

### Branch Workflow

- Never commit directly to `main`
- Create a feature branch per phase or logical unit of work
  (e.g., `phase-1/core-data-model`, `phase-1/schema-parser`)
- Commit early and often within the branch — each commit should be a complete,
  working change with passing tests
- Use conventional commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`
- When a phase is complete, present the branch for review before merging to
  `main`. Do not merge without explicit approval.

### Commit Discipline

- Run `cargo test` before every commit — all tests must pass
- Run `cargo clippy` before every commit — no warnings
- Run `cargo fmt --check` before every commit — code must be formatted
- Include test changes with the feature they test in the same commit
- Refactoring commits are separate from feature commits

## Rust Conventions

### General

- Rust edition 2024
- Prefer `thiserror` for library errors, `anyhow` for application errors
- Prefer `&str` over `String` in function signatures where ownership is not needed
- Use newtypes for domain identifiers (e.g., `TenantId(Uuid)`, `ObjectId(String)`)
- No `unwrap()` or `expect()` in production code — handle errors explicitly
- `unwrap()` is acceptable in tests
- Prefer iterators and combinators over imperative loops
- No unsafe code without explicit justification

### Testing

- Tests live in the same file as the code they test (`#[cfg(test)]` module)
- Integration tests go in `tests/`
- Test behavior through public APIs — do not test private functions directly
- Use factory functions for test data with sensible defaults
- Aim for 100% behavior coverage, not line coverage

### Project Structure

The project uses a Cargo workspace with multiple crates:

```
crates/
  kraalzibar-core/     # Domain types, schema parser, graph engine
  kraalzibar-storage/  # Storage trait + implementations
  kraalzibar-server/   # gRPC + REST server binary
  kraalzibar-client/   # Rust SDK
```

## Twelve-Factor App

All design and implementation must adhere to the
[twelve-factor methodology](https://12factor.net). In practice:

1. **Codebase**: One repo, many deploys
2. **Dependencies**: Explicitly declared in `Cargo.toml` — no implicit system deps
3. **Config**: All configuration via environment variables (TOML config file is
   a convenience layer that can be overridden by env vars, never hardcoded)
4. **Backing services**: PostgreSQL is an attached resource identified by URL —
   swappable without code changes
5. **Build, release, run**: Strict separation — the binary is built once and
   configured at run time
6. **Processes**: The server is stateless. All persistent state lives in
   PostgreSQL. In-process caches are ephemeral and reconstructable.
7. **Port binding**: The service exports gRPC and HTTP by binding to ports
   specified via config/env
8. **Concurrency**: Scale out via process model (run multiple instances behind
   a load balancer)
9. **Disposability**: Fast startup, graceful shutdown on SIGTERM/SIGINT
10. **Dev/prod parity**: Docker Compose provides a production-like local
    environment
11. **Logs**: Structured JSON log events written to stdout — never to files.
    Log routing is the environment's responsibility.
12. **Admin processes**: One-off tasks (migrations, tenant provisioning) run as
    separate commands using the same codebase and config

## Key Design Decisions

- **Multi-tenancy**: Separate PG schema per tenant, factory pattern for
  tenant-scoped stores
- **Schema lifecycle**: Two-step dry-run flow — `WriteSchema` rejects breaking
  changes unless `force: true`
- **Auth**: API key per tenant (argon2 hashed), resolved in middleware
- **Consistency**: Snapshot tokens per tenant, causal consistency
- **Engine limits**: Max depth 6, schema caps (50 types, 30 relations, 30
  permissions per type), lookup limit 1000

## README Maintenance

Keep `README.md` up to date as the project evolves. When adding new
functionality, update the Getting Started section with build, run, and
configuration instructions as they become available.

## Gotchas and Learnings

- **Rust edition 2024 is valid** with Rust 1.85+. `resolver = "3"` in the
  workspace Cargo.toml is the edition 2024 equivalent of `resolver = "2"`.
- **Native async fn in traits** works in edition 2024 — no `async-trait`
  crate needed. Add `Send + Sync` super-traits on the trait itself.
- **`std::sync::Mutex`** (not tokio Mutex) is correct for the in-memory store
  because critical sections are CPU-bound and short.
- **`TupleFilter.subject_relation` uses `Option<Option<String>>`** — outer
  `None` = don't filter, `Some(None)` = must be direct, `Some(Some(r))` = must
  match relation `r`. This is a key design choice that pervades the storage layer.
- **Pest grammar** does not enforce "no mixed operators" — that's a semantic
  check in the parser, not in the grammar rules.
- **PG sequence `last_value`** starts at 1 but `is_called` is false until first
  `nextval()`. Use `COALESCE(... WHERE is_called = true, 0)` to get 0 for
  fresh sequences.
- **Testcontainers tests** must hold the container in a variable (`_container`)
  for the entire test — dropping it stops the container.
- **sqlx dynamic queries** require building bind parameters as a vec and
  applying them in order. The `bind_idx` counter tracks `$1`, `$2`, etc.
- **`InMemoryStore::new()`** should be `pub` — it's needed by tests across
  module boundaries (e.g., GC tests use in-memory store directly).
