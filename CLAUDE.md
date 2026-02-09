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

## Key Design Decisions

- **Multi-tenancy**: Separate PG schema per tenant, factory pattern for
  tenant-scoped stores
- **Schema lifecycle**: Two-step dry-run flow — `WriteSchema` rejects breaking
  changes unless `force: true`
- **Auth**: API key per tenant (argon2 hashed), resolved in middleware
- **Consistency**: Snapshot tokens per tenant, causal consistency
- **Engine limits**: Max depth 6, schema caps (50 types, 30 relations, 30
  permissions per type), lookup limit 1000

## Gotchas and Learnings

Update this section as the project progresses with things you wish you knew
at the start.
