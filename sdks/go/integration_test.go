//go:build integration

package kraalzibar

import (
	"context"
	"os"
	"sort"
	"testing"
)

func serverTarget() string {
	if t := os.Getenv("KRAALZIBAR_TARGET"); t != "" {
		return t
	}
	return "localhost:50051"
}

func testClient(t *testing.T) *Client {
	t.Helper()
	c, err := NewClient(serverTarget(), WithInsecure())
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}
	t.Cleanup(func() { _ = c.Close() })
	return c
}

func setupSchema(t *testing.T, c *Client, schema string) {
	t.Helper()
	ctx := context.Background()
	_, err := c.WriteSchema(ctx, schema, true)
	if err != nil {
		t.Fatalf("WriteSchema: %v", err)
	}
}

func touchRelationship(t *testing.T, c *Client, objectType, objectID, relation, subjectType, subjectID string) {
	t.Helper()
	ctx := context.Background()
	_, err := c.WriteRelationships(ctx, WriteRelationshipsRequest{
		Updates: []RelationshipUpdate{{
			Operation: OperationTouch,
			Object:    ObjectRef{ObjectType: objectType, ObjectID: objectID},
			Relation:  relation,
			Subject:   DirectSubject(subjectType, subjectID),
		}},
	})
	if err != nil {
		t.Fatalf("WriteRelationships: %v", err)
	}
}

const basicSchema = `definition user {}

definition document {
  relation viewer: user
  permission can_view = viewer
}`

func TestIntegration_CheckPermissionGranted(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, basicSchema)
	touchRelationship(t, c, "document", "readme", "viewer", "user", "alice")

	resp, err := c.CheckPermission(context.Background(), CheckPermissionRequest{
		Resource:   ObjectRef{ObjectType: "document", ObjectID: "readme"},
		Permission: "can_view",
		Subject:    DirectSubject("user", "alice"),
	})
	if err != nil {
		t.Fatalf("CheckPermission: %v", err)
	}
	if !resp.Allowed {
		t.Error("expected permission to be granted")
	}
}

func TestIntegration_CheckPermissionDenied(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, basicSchema)
	touchRelationship(t, c, "document", "readme", "viewer", "user", "alice")

	resp, err := c.CheckPermission(context.Background(), CheckPermissionRequest{
		Resource:   ObjectRef{ObjectType: "document", ObjectID: "readme"},
		Permission: "can_view",
		Subject:    DirectSubject("user", "bob"),
	})
	if err != nil {
		t.Fatalf("CheckPermission: %v", err)
	}
	if resp.Allowed {
		t.Error("expected permission to be denied")
	}
}

func TestIntegration_WriteAndReadRelationships(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, basicSchema)
	touchRelationship(t, c, "document", "readme", "viewer", "user", "alice")
	touchRelationship(t, c, "document", "readme", "viewer", "user", "bob")

	rels, err := c.ReadRelationships(context.Background(), ReadRelationshipsRequest{
		ObjectType: "document",
		ObjectID:   "readme",
		Relation:   "viewer",
	})
	if err != nil {
		t.Fatalf("ReadRelationships: %v", err)
	}
	if len(rels) != 2 {
		t.Errorf("expected 2 relationships, got %d", len(rels))
	}
}

func TestIntegration_LookupResources(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, basicSchema)
	touchRelationship(t, c, "document", "doc1", "viewer", "user", "alice")
	touchRelationship(t, c, "document", "doc2", "viewer", "user", "alice")
	touchRelationship(t, c, "document", "doc3", "viewer", "user", "bob")

	ids, err := c.LookupResources(context.Background(), LookupResourcesRequest{
		ResourceType: "document",
		Permission:   "can_view",
		Subject:      DirectSubject("user", "alice"),
	})
	if err != nil {
		t.Fatalf("LookupResources: %v", err)
	}
	sort.Strings(ids)
	if len(ids) != 2 || ids[0] != "doc1" || ids[1] != "doc2" {
		t.Errorf("expected [doc1, doc2], got %v", ids)
	}
}

func TestIntegration_LookupSubjects(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, `definition user {}

definition document {
  relation viewer: user
  relation owner: user
  permission can_view = viewer + owner
}`)
	touchRelationship(t, c, "document", "readme", "viewer", "user", "alice")
	touchRelationship(t, c, "document", "readme", "owner", "user", "bob")

	subjects, err := c.LookupSubjects(context.Background(), LookupSubjectsRequest{
		Resource:    ObjectRef{ObjectType: "document", ObjectID: "readme"},
		Permission:  "can_view",
		SubjectType: "user",
	})
	if err != nil {
		t.Fatalf("LookupSubjects: %v", err)
	}
	ids := make([]string, len(subjects))
	for i, s := range subjects {
		ids[i] = s.SubjectID
	}
	sort.Strings(ids)
	if len(ids) != 2 || ids[0] != "alice" || ids[1] != "bob" {
		t.Errorf("expected [alice, bob], got %v", ids)
	}
}

func TestIntegration_SchemaWriteAndRead(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, basicSchema)

	schema, err := c.ReadSchema(context.Background())
	if err != nil {
		t.Fatalf("ReadSchema: %v", err)
	}
	for _, expected := range []string{"definition user", "definition document", "relation viewer", "permission can_view"} {
		if !containsStr(schema, expected) {
			t.Errorf("schema missing %q", expected)
		}
	}
}

func TestIntegration_ComputedPermissionViaUnion(t *testing.T) {
	c := testClient(t)
	setupSchema(t, c, `definition user {}

definition document {
  relation viewer: user
  relation editor: user
  permission can_edit = editor
  permission can_view = viewer + can_edit
}`)
	touchRelationship(t, c, "document", "readme", "editor", "user", "alice")

	resp, err := c.CheckPermission(context.Background(), CheckPermissionRequest{
		Resource:   ObjectRef{ObjectType: "document", ObjectID: "readme"},
		Permission: "can_view",
		Subject:    DirectSubject("user", "alice"),
	})
	if err != nil {
		t.Fatalf("CheckPermission: %v", err)
	}
	if !resp.Allowed {
		t.Error("expected computed permission via union to be granted")
	}
}

func containsStr(s, substr string) bool {
	return len(s) >= len(substr) && (s == substr || len(s) > 0 && containsSubstring(s, substr))
}

func containsSubstring(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
