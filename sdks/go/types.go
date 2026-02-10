package kraalzibar

import "fmt"

// ObjectRef references a specific object by type and ID.
type ObjectRef struct {
	ObjectType string
	ObjectID   string
}

func (o ObjectRef) String() string {
	return fmt.Sprintf("%s:%s", o.ObjectType, o.ObjectID)
}

// SubjectRef references a subject, optionally with a relation (userset).
type SubjectRef struct {
	SubjectType     string
	SubjectID       string
	SubjectRelation string
}

func (s SubjectRef) String() string {
	if s.SubjectRelation != "" {
		return fmt.Sprintf("%s:%s#%s", s.SubjectType, s.SubjectID, s.SubjectRelation)
	}
	return fmt.Sprintf("%s:%s", s.SubjectType, s.SubjectID)
}

// DirectSubject creates a SubjectRef without a relation.
func DirectSubject(subjectType, subjectID string) SubjectRef {
	return SubjectRef{
		SubjectType: subjectType,
		SubjectID:   subjectID,
	}
}

// UsersetSubject creates a SubjectRef with a relation.
func UsersetSubject(subjectType, subjectID, relation string) SubjectRef {
	return SubjectRef{
		SubjectType:     subjectType,
		SubjectID:       subjectID,
		SubjectRelation: relation,
	}
}
