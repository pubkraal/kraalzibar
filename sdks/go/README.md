# Kraalzibar Go SDK

Go client for the [Kraalzibar](https://github.com/pubkraal/kraalzibar) authorization server. Communicates over gRPC.

## Installation

```bash
go get github.com/pubkraal/kraalzibar/sdks/go
```

Requires Go 1.24+.

## Quick Start

```go
package main

import (
	"context"
	"fmt"
	"log"

	kraalzibar "github.com/pubkraal/kraalzibar/sdks/go"
)

func main() {
	client, err := kraalzibar.NewClient("localhost:50051", kraalzibar.WithInsecure())
	if err != nil {
		log.Fatal(err)
	}
	defer client.Close()

	ctx := context.Background()

	// Write an authorization schema
	_, err = client.WriteSchema(ctx, `definition user {}

definition document {
  relation viewer: user
  relation editor: user
  permission view = viewer + editor
  permission edit = editor
}`, false)
	if err != nil {
		log.Fatal(err)
	}

	// Create a relationship: alice is an editor of document:readme
	_, err = client.WriteRelationships(ctx, kraalzibar.WriteRelationshipsRequest{
		Updates: []kraalzibar.RelationshipUpdate{{
			Operation: kraalzibar.OperationTouch,
			Object:    kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
			Relation:  "editor",
			Subject:   kraalzibar.DirectSubject("user", "alice"),
		}},
	})
	if err != nil {
		log.Fatal(err)
	}

	// Check permission
	resp, err := client.CheckPermission(ctx, kraalzibar.CheckPermissionRequest{
		Resource:   kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
		Permission: "view",
		Subject:    kraalzibar.DirectSubject("user", "alice"),
	})
	if err != nil {
		log.Fatal(err)
	}
	fmt.Printf("alice can view document:readme = %v\n", resp.Allowed)
}
```

## Server Setup

### Dev Mode (In-Memory)

Start the server with no configuration for development:

```bash
cargo run --release
```

This runs with in-memory storage and no authentication. Data is lost on restart.

### Production Mode (PostgreSQL)

Configure a database URL to enable persistent storage and API key authentication:

```bash
# Run migrations
kraalzibar-server migrate --config config.toml

# Provision a tenant
kraalzibar-server provision-tenant --name acme --config config.toml

# Create an API key for the tenant
kraalzibar-server create-api-key --tenant-name acme --config config.toml

# Start the server
kraalzibar-server serve --config config.toml
```

The `create-api-key` command prints the full API key once. Store it securely.

## Client Options

```go
// Connect without TLS (development)
client, err := kraalzibar.NewClient("localhost:50051", kraalzibar.WithInsecure())

// Connect with an API key (production)
client, err := kraalzibar.NewClient("server:50051",
    kraalzibar.WithAPIKey("kraalzibar_abcd1234_secretsecretsecretsecretsecretsec"),
)

// Custom timeout (default: 30s)
client, err := kraalzibar.NewClient("server:50051",
    kraalzibar.WithTimeout(10 * time.Second),
)

// Additional gRPC dial options
client, err := kraalzibar.NewClient("server:50051",
    kraalzibar.WithDialOptions(grpc.WithBlock()),
)
```

| Option | Description |
|--------|-------------|
| `WithInsecure()` | Disable TLS for the gRPC connection |
| `WithAPIKey(key)` | Set the Bearer token for authentication |
| `WithTimeout(d)` | Set per-request timeout (default 30s) |
| `WithDialOptions(opts...)` | Append additional gRPC dial options |

## API Reference

### NewClient

```go
func NewClient(target string, opts ...ClientOption) (*Client, error)
```

Creates a new client connected to the given gRPC target address.

### Close

```go
func (c *Client) Close() error
```

Closes the underlying gRPC connection.

### CheckPermission

```go
func (c *Client) CheckPermission(ctx context.Context, req CheckPermissionRequest) (*CheckPermissionResponse, error)
```

Checks whether a subject has a permission on a resource.

```go
resp, err := client.CheckPermission(ctx, kraalzibar.CheckPermissionRequest{
    Resource:   kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
    Permission: "view",
    Subject:    kraalzibar.DirectSubject("user", "alice"),
})
if err != nil {
    log.Fatal(err)
}
fmt.Println(resp.Allowed) // true or false
```

### WriteRelationships

```go
func (c *Client) WriteRelationships(ctx context.Context, req WriteRelationshipsRequest) (*WriteRelationshipsResponse, error)
```

Creates or deletes relationships. Returns a snapshot token in `WrittenAt`.

```go
resp, err := client.WriteRelationships(ctx, kraalzibar.WriteRelationshipsRequest{
    Updates: []kraalzibar.RelationshipUpdate{
        {
            Operation: kraalzibar.OperationTouch,
            Object:    kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
            Relation:  "viewer",
            Subject:   kraalzibar.DirectSubject("user", "bob"),
        },
        {
            Operation: kraalzibar.OperationDelete,
            Object:    kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
            Relation:  "viewer",
            Subject:   kraalzibar.DirectSubject("user", "charlie"),
        },
    },
})
```

### ReadRelationships

```go
func (c *Client) ReadRelationships(ctx context.Context, req ReadRelationshipsRequest) ([]Relationship, error)
```

Reads relationships matching the given filter. All filter fields are optional.

```go
rels, err := client.ReadRelationships(ctx, kraalzibar.ReadRelationshipsRequest{
    ObjectType: "document",
    ObjectID:   "readme",
    Relation:   "viewer",
})
for _, rel := range rels {
    fmt.Printf("%s#%s@%s\n", rel.Object, rel.Relation, rel.Subject)
}
```

### LookupResources

```go
func (c *Client) LookupResources(ctx context.Context, req LookupResourcesRequest) ([]string, error)
```

Finds all resources of a given type that the subject has a permission on. Returns resource IDs.

```go
ids, err := client.LookupResources(ctx, kraalzibar.LookupResourcesRequest{
    ResourceType: "document",
    Permission:   "view",
    Subject:      kraalzibar.DirectSubject("user", "alice"),
    Limit:        100,
})
```

### LookupSubjects

```go
func (c *Client) LookupSubjects(ctx context.Context, req LookupSubjectsRequest) ([]SubjectRef, error)
```

Finds all subjects of a given type with a permission on a resource.

```go
subjects, err := client.LookupSubjects(ctx, kraalzibar.LookupSubjectsRequest{
    Resource:    kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"},
    Permission:  "view",
    SubjectType: "user",
})
for _, s := range subjects {
    fmt.Println(s.SubjectID)
}
```

### WriteSchema

```go
func (c *Client) WriteSchema(ctx context.Context, schema string, force bool) (*WriteSchemaResponse, error)
```

Writes or updates the authorization schema. Set `force` to `true` to allow breaking changes (e.g., removing a relation that has existing tuples).

```go
resp, err := client.WriteSchema(ctx, `definition user {}

definition document {
  relation viewer: user
  permission view = viewer
}`, false)
```

### ReadSchema

```go
func (c *Client) ReadSchema(ctx context.Context) (string, error)
```

Reads the current authorization schema as a string.

```go
schema, err := client.ReadSchema(ctx)
fmt.Println(schema)
```

## Types

### ObjectRef

References a specific object by type and ID.

```go
type ObjectRef struct {
    ObjectType string
    ObjectID   string
}

ref := kraalzibar.ObjectRef{ObjectType: "document", ObjectID: "readme"}
fmt.Println(ref) // "document:readme"
```

### SubjectRef

References a subject, optionally with a relation (userset).

```go
type SubjectRef struct {
    SubjectType     string
    SubjectID       string
    SubjectRelation string  // empty for direct subjects
}
```

Use the constructor functions:

```go
// Direct subject: user:alice
alice := kraalzibar.DirectSubject("user", "alice")

// Userset subject: team:engineering#member
members := kraalzibar.UsersetSubject("team", "engineering", "member")
```

### RelationshipUpdate

Describes a single relationship write or delete.

```go
type RelationshipUpdate struct {
    Operation RelationshipOperation
    Object    ObjectRef
    Relation  string
    Subject   SubjectRef
}
```

### RelationshipOperation

| Constant | Value | Description |
|----------|-------|-------------|
| `OperationTouch` | 1 | Create or update the relationship |
| `OperationDelete` | 2 | Delete the relationship |

## Error Handling

All methods return `*Error` on failure with a `Code` and `Message`:

```go
resp, err := client.CheckPermission(ctx, req)
if err != nil {
    var kErr *kraalzibar.Error
    if errors.As(err, &kErr) {
        switch kErr.Code {
        case kraalzibar.CodeInvalidArgument:
            // bad request
        case kraalzibar.CodeUnavailable:
            // server unreachable
        case kraalzibar.CodeTimeout:
            // deadline exceeded
        default:
            log.Printf("kraalzibar error: %v", kErr)
        }
    }
}
```

| Code | Description |
|------|-------------|
| `CodeUnknown` | Unknown or unmapped error |
| `CodeInvalidArgument` | Invalid request parameters |
| `CodeNotFound` | Resource not found |
| `CodePermissionDenied` | Insufficient permissions |
| `CodeFailedPrecondition` | Precondition failed (e.g., breaking schema change) |
| `CodeInternal` | Internal server error |
| `CodeTimeout` | Request deadline exceeded |
| `CodeUnavailable` | Server unavailable |

## Testing

### Unit Tests

```bash
cd sdks/go
go test ./...
```

### Integration Tests

Integration tests require a running kraalzibar server and use the `integration` build tag:

```bash
# Start the server (dev mode)
cargo run --release &

# Run integration tests (default target: localhost:50051)
cd sdks/go
go test -tags integration ./...

# Or specify a custom target
KRAALZIBAR_TARGET=server:50051 go test -tags integration ./...
```
