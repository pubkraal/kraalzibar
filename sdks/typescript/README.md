# Kraalzibar TypeScript SDK

TypeScript client for the [Kraalzibar](https://github.com/pubkraal/kraalzibar) authorization server. Communicates over REST with zero runtime dependencies.

## Installation

```bash
npm install @kraalzibar/client
```

## Quick Start

```typescript
import { createClient } from "@kraalzibar/client";

const client = createClient({ target: "http://localhost:8080" });

// Write schema
await client.writeSchema(`definition user {}

definition document {
  relation viewer: user
  relation editor: user
  permission view = viewer + editor
  permission edit = editor
}`);

// Create a relationship
await client.writeRelationships({
  updates: [
    {
      operation: "touch",
      resource_type: "document",
      resource_id: "readme",
      relation: "editor",
      subject_type: "user",
      subject_id: "alice",
    },
  ],
});

// Check permission
const { allowed } = await client.checkPermission({
  resource_type: "document",
  resource_id: "readme",
  permission: "view",
  subject_type: "user",
  subject_id: "alice",
});

console.log(`alice can view: ${allowed}`);
```

## Authentication

Pass an API key to authenticate with a production server:

```typescript
const client = createClient({
  target: "http://server:8080",
  apiKey: "kraalzibar_abcd1234_secretsecretsecretsecretsecretsec",
});
```

The API key is sent as a `Bearer` token in the `Authorization` header on every request.

## Client Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `target` | `string` | (required) | Base URL of the REST API |
| `apiKey` | `string` | `undefined` | API key for authentication |
| `timeout` | `number` | `30000` | Per-request timeout in milliseconds |
| `fetch` | `typeof fetch` | `globalThis.fetch` | Custom fetch implementation (for testing) |

```typescript
import { createClient } from "@kraalzibar/client";

const client = createClient({
  target: "http://localhost:8080",
  apiKey: "kraalzibar_...",
  timeout: 10_000,
});
```

## API Reference

### `createClient`

```typescript
function createClient(options: ClientOptions): KraalzibarClient
```

Creates a new client. Throws if `target` is empty.

### `checkPermission`

```typescript
function checkPermission(request: CheckPermissionRequest): Promise<CheckPermissionResponse>
```

Checks whether a subject has a permission on a resource.

```typescript
const { allowed, checked_at } = await client.checkPermission({
  resource_type: "document",
  resource_id: "readme",
  permission: "view",
  subject_type: "user",
  subject_id: "alice",
  consistency: { type: "full" },
});
```

### `writeRelationships`

```typescript
function writeRelationships(request: WriteRelationshipsRequest): Promise<WriteRelationshipsResponse>
```

Creates or deletes relationships. Returns a `written_at` snapshot token.

```typescript
const { written_at } = await client.writeRelationships({
  updates: [
    {
      operation: "touch",
      resource_type: "document",
      resource_id: "readme",
      relation: "viewer",
      subject_type: "user",
      subject_id: "bob",
    },
    {
      operation: "delete",
      resource_type: "document",
      resource_id: "readme",
      relation: "viewer",
      subject_type: "user",
      subject_id: "charlie",
    },
  ],
});
```

### `readRelationships`

```typescript
function readRelationships(request: ReadRelationshipsRequest): Promise<ReadRelationshipsResponse>
```

Reads relationships matching the given filter. All filter fields are optional.

```typescript
const { relationships } = await client.readRelationships({
  filter: {
    resource_type: "document",
    resource_id: "readme",
    relation: "viewer",
  },
});
```

### `lookupResources`

```typescript
function lookupResources(request: LookupResourcesRequest): Promise<LookupResourcesResponse>
```

Finds all resources of a given type that the subject has a permission on.

```typescript
const { resource_ids } = await client.lookupResources({
  resource_type: "document",
  permission: "view",
  subject_type: "user",
  subject_id: "alice",
  limit: 100,
});
```

### `lookupSubjects`

```typescript
function lookupSubjects(request: LookupSubjectsRequest): Promise<LookupSubjectsResponse>
```

Finds all subjects of a given type with a permission on a resource.

```typescript
const { subjects } = await client.lookupSubjects({
  resource_type: "document",
  resource_id: "readme",
  permission: "view",
  subject_type: "user",
});
```

### `writeSchema`

```typescript
function writeSchema(schema: string, options?: WriteSchemaOptions): Promise<WriteSchemaResponse>
```

Writes or updates the authorization schema. Pass `{ force: true }` to allow breaking changes.

```typescript
await client.writeSchema(schemaText, { force: true });
```

### `readSchema`

```typescript
function readSchema(): Promise<string>
```

Reads the current authorization schema as a string.

## Types

### `RelationshipUpdate`

```typescript
type RelationshipUpdate = {
  operation: "touch" | "delete";
  resource_type: string;
  resource_id: string;
  relation: string;
  subject_type: string;
  subject_id: string;
  subject_relation?: string;
};
```

### `ConsistencyRequest`

```typescript
type ConsistencyRequest =
  | { type: "full" }
  | { type: "minimize_latency" }
  | { type: "at_least_as_fresh"; token: string }
  | { type: "at_exact_snapshot"; token: string };
```

## Error Handling

All methods throw `KraalzibarError` on failure:

```typescript
import { KraalzibarError } from "@kraalzibar/client";

try {
  await client.checkPermission(request);
} catch (error) {
  if (error instanceof KraalzibarError) {
    switch (error.code) {
      case "UNAUTHENTICATED":
        console.error("Invalid or missing API key");
        break;
      case "INVALID_ARGUMENT":
        console.error("Bad request:", error.message);
        break;
      default:
        console.error(`Error [${error.code}]: ${error.message}`);
    }
  }
}
```

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `INVALID_ARGUMENT` | 400 | Invalid request parameters |
| `UNAUTHENTICATED` | 401 | Missing or invalid API key |
| `NOT_FOUND` | 404 | Resource not found |
| `FAILED_PRECONDITION` | 412 | Precondition failed (e.g., breaking schema change) |
| `RESOURCE_EXHAUSTED` | 422 | Resource limit exceeded |
| `INTERNAL` | 500 | Internal server error |
| `UNKNOWN` | Other | Unmapped HTTP status |

## Testing

### Unit Tests

```bash
cd sdks/typescript
npm test
```

### Integration Tests

Integration tests require a running kraalzibar server:

```bash
# Start the server (dev mode)
cargo run --release &

# Run integration tests
cd sdks/typescript
INTEGRATION=true npm run test:integration
```
