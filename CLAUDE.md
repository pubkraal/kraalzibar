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
- After merging a PR, delete the feature branch (both local and remote).

### Automated PR Review

After every `git push` to a branch that has an open GitHub PR, **automatically
run the pr-reviewer agent** to review the PR. This is mandatory and must not be
skipped. The review should check for TDD compliance, code quality, test
coverage, and adherence to project conventions.

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
- **Recursive async functions in Rust** cannot use `async fn` directly due to
  E0733 (opaque type of infinite size). Return
  `Pin<Box<dyn Future<Output = ...> + Send + 'a>>` with `Box::pin(async move { ... })`.
- **`TupleReader` trait lives in `kraalzibar-core`** (not storage) to avoid
  circular dependency. The engine is generic over `T: TupleReader`; storage
  crates implement the trait. For tests, a simple `TestStore` with `Vec<Tuple>`
  suffices.
- **Clippy `too_many_arguments`** triggers at 7+ params. Bundle related params
  into context structs (e.g., `CheckContext`, `ExpandContext`) early rather than
  accumulating individual parameters.
- **Permissions shadow relations** in `This(name)` resolution — check
  permissions first, then relations. This matches Zanzibar/SpiceDB semantics.
- **Cycles in the permission graph return `false`** (not errors) — matching
  Zanzibar semantics. Use a per-branch `HashSet<(String, String, String)>`
  visited set. Clone the set when branching.
- **Context structs must own Strings** (not borrow) when used in recursive
  async evaluation, because each branch needs its own depth counter and visited
  set that can be cloned independently.
- **`StoreTupleReader` adapter** bridges `RelationshipStore` → `TupleReader` in
  the server crate. Maps `StorageError` → `CheckError::StorageError`. Lives in
  `crates/kraalzibar-server/src/adapter.rs`.
- **`AuthzService<F: StoreFactory>`** is the shared service layer. Both gRPC and
  REST handlers call it. Each request creates a fresh store from the factory,
  loads the schema (from cache or storage), builds an engine, and evaluates.
- **`EngineConfig` needs `Clone`** since the service creates engines per-request.
  Added `#[derive(Clone)]` in Phase 3.
- **`lookup_resources`** scans all tuples for a type, deduplicates object IDs,
  then checks each one. Default limit is 1000. This is O(n) and correct but
  slow for large datasets — optimize in later phases.
- **Never expose storage errors to clients.** Use generic "internal server error"
  message in both gRPC Status and REST JSON responses. Log the real error
  server-side with `tracing::error!`.
- **Body and message size limits** must be set explicitly. axum:
  `DefaultBodyLimit::max(4MB)`. tonic: `.max_decoding_message_size(4MB)` on
  each service server. Without these, clients can OOM the server.
- **Input validation at API boundaries**: validate identifiers are non-empty and
  under 256 chars, enforce batch size limits (1000 for writes), and clamp
  user-provided query limits to a safe max (e.g., `min(requested, 10_000)`).
- **`Vec::dedup()` only removes consecutive duplicates** — always sort first.
  This bit `lookup_subjects` where results from different tree branches could
  interleave.
- **Pattern-match with wildcards on proto booleans**: use `FullConsistency(_)`
  not `FullConsistency(true)`, since protobuf can send `false`.
- **Clippy `result_large_err`**: `tonic::Status` is 176 bytes, triggering the
  lint on `Result<_, tonic::Status>`. Suppress with
  `#[allow(clippy::result_large_err)]` on affected conversion functions.
- **Config test env var races**: tests that set env vars like
  `KRAALZIBAR_GRPC_PORT` can race with other tests. Use unique var names or
  the `serial_test` crate. Known flaky test: `load_from_toml_file`.
- **Refactoring can create dead code**: after replacing `ErrorResponse` struct
  with `serde_json::json!()` in handlers, clippy caught the now-unused type.
  Always run clippy after refactors.
- **axum `DefaultBodyLimit`** requires importing from `axum::extract`. Add it
  as a `.layer()` on the router, not on individual routes.
- **moka `Cache::try_get_with`** requires closures that return `Result`. For
  simple insertions, use `cache.get()` + `cache.insert()` manually instead.
- **Criterion benchmarks with tokio**: Use
  `tokio::runtime::Builder::new_current_thread().enable_all().build()` — the
  `rt` feature alone doesn't include `Runtime::new()`.
- **`#[cfg(test)]` in bench files**: Test modules inside bench files are not
  compiled. Validate factory helpers implicitly through benchmark execution.
- **Optional metrics with `Option<Arc<Metrics>>`**: Use this pattern to add
  observability without breaking existing tests. The `with_metrics()` builder
  method keeps the API backward-compatible.
- **Collapsible `if let` with `&&` (let chains)**: Rust 2024 edition supports
  `if let Some(v) = expr && let Ok(n) = v.parse() { ... }`. Clippy enforces
  this over nested `if let` blocks.
- **Cache invalidation strategy**: Schema cache uses TTL (30s default) + local
  invalidation on `write_schema`. Check cache uses snapshot-keyed entries
  (naturally immutable) + `invalidate_all()` on writes. Cross-server staleness
  for schema is bounded by TTL (same approach as SpiceDB).
- **Check cache only caches snapshot-keyed requests**: `MinimizeLatency`
  (snapshot=None) skips the cache because results can change between calls.
  Only `AtExactSnapshot` and `FullConsistency` (which resolves to a concrete
  snapshot) produce cacheable results.
- **`protoc-gen-go` vs `protoc-gen-go-grpc` package name mismatch**: Old
  versions of `protoc-gen-go-grpc` generate a different Go package name
  (e.g., `kraalzibar_v1`) than `protoc-gen-go` (`v1`). Upgrade both plugins
  to latest (`go install ...@latest`) before generating.
- **Always add `.gitignore` before `git add`**: In TypeScript projects,
  `node_modules/` will be committed if no `.gitignore` is in place. Add
  `.gitignore` first, then `git add`.
- **In-process gRPC test server pattern**: Bind `TcpListener` to `127.0.0.1:0`,
  get the random port from `local_addr()`, wrap in `TcpListenerStream`, pass to
  `tonic::transport::Server::serve_with_incoming()`. No Docker needed for Rust
  integration tests.
- **Clippy `derivable_impls`**: If a `Default` impl just selects a specific enum
  variant, use `#[derive(Default)]` with `#[default]` attribute on the variant
  instead of a manual `impl Default`.
- **`OpenTelemetryLayer` is typed to bare `Registry`**: When building a layered
  tracing subscriber, the OTel layer must be `.with()`'d directly onto the
  `Registry` before any other layers (like `EnvFilter`). If added after
  `EnvFilter`, the type mismatch (`Layer<Layered<EnvFilter, Registry>>` vs
  `Layer<Registry>`) produces confusing compile errors. Layer order:
  `Registry → OTel → EnvFilter → fmt`.
- **OTel test with real OTLP connection hangs on drop**: Creating a real
  `TracerProvider` with OTLP export in tests spawns background threads that
  hang on `drop()`/`shutdown()` when no collector is running. Only test the
  disabled path (`init_telemetry` returns `None`); skip tests that create real
  providers.
- **Hand-rolled Prometheus histograms**: Store f64 sum as `AtomicU64` using
  `f64::to_bits()`/`f64::from_bits()`. Use `fetch_update` with
  `Ordering::Relaxed` to atomically add to the sum. Bucket boundaries are a
  `const` array; `+Inf` bucket always equals `_count`.
- **Capturing tracing events in tests**: Create a custom `Layer` that stores
  events in `Arc<Mutex<Vec<CapturedEvent>>>`. Use
  `tracing::subscriber::set_default()` (not `set_global_default`) which returns
  a guard — this avoids conflicts between parallel tests.
- **Capturing tracing spans in tests**: Similar pattern but implement
  `on_new_span` instead of `on_event`. Use `span.extensions_mut().insert()` to
  tag spans, then read back in `on_close`.
- **Feature-gated optional deps pattern**: Use `dep:crate_name` syntax in
  `[features]` to avoid auto-creating a feature per optional dep. Wrap code
  with `#[cfg(feature = "telemetry")]` and provide stub functions for the
  `#[cfg(not(feature = "telemetry"))]` case.
- **`std::mem::forget` for long-lived providers**: OTel `TracerProvider` must
  live for the process lifetime. Leaking it with `std::mem::forget` is
  intentional — the OS reclaims resources on exit. This avoids blocking
  shutdown while the OTLP exporter drains.
