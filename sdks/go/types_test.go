package kraalzibar

import "testing"

func TestObjectRef_String(t *testing.T) {
	obj := ObjectRef{ObjectType: "document", ObjectID: "readme"}
	if got := obj.String(); got != "document:readme" {
		t.Errorf("ObjectRef.String() = %q, want %q", got, "document:readme")
	}
}

func TestSubjectRef_String(t *testing.T) {
	direct := DirectSubject("user", "alice")
	if got := direct.String(); got != "user:alice" {
		t.Errorf("SubjectRef.String() = %q, want %q", got, "user:alice")
	}
}

func TestSubjectRef_WithRelation(t *testing.T) {
	userset := UsersetSubject("group", "eng", "member")
	if got := userset.String(); got != "group:eng#member" {
		t.Errorf("SubjectRef.String() = %q, want %q", got, "group:eng#member")
	}
}
