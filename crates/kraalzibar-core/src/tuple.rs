use std::fmt;

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(Uuid);

impl TenantId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl From<Uuid> for TenantId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub object_type: String,
    pub object_id: String,
}

impl ObjectRef {
    pub fn new(object_type: impl Into<String>, object_id: impl Into<String>) -> Self {
        Self {
            object_type: object_type.into(),
            object_id: object_id.into(),
        }
    }
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.object_type, self.object_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubjectRef {
    pub subject_type: String,
    pub subject_id: String,
    pub subject_relation: Option<String>,
}

impl SubjectRef {
    pub fn direct(subject_type: impl Into<String>, subject_id: impl Into<String>) -> Self {
        Self {
            subject_type: subject_type.into(),
            subject_id: subject_id.into(),
            subject_relation: None,
        }
    }

    pub fn userset(
        subject_type: impl Into<String>,
        subject_id: impl Into<String>,
        relation: impl Into<String>,
    ) -> Self {
        Self {
            subject_type: subject_type.into(),
            subject_id: subject_id.into(),
            subject_relation: Some(relation.into()),
        }
    }
}

impl fmt::Display for SubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.subject_type, self.subject_id)?;
        if let Some(ref rel) = self.subject_relation {
            write!(f, "#{rel}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tuple {
    pub object: ObjectRef,
    pub relation: String,
    pub subject: SubjectRef,
}

impl Tuple {
    pub fn new(object: ObjectRef, relation: impl Into<String>, subject: SubjectRef) -> Self {
        Self {
            object,
            relation: relation.into(),
            subject,
        }
    }
}

impl fmt::Display for Tuple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}@{}", self.object, self.relation, self.subject)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TupleWrite {
    pub object: ObjectRef,
    pub relation: String,
    pub subject: SubjectRef,
}

impl TupleWrite {
    pub fn new(object: ObjectRef, relation: impl Into<String>, subject: SubjectRef) -> Self {
        Self {
            object,
            relation: relation.into(),
            subject,
        }
    }
}

impl From<TupleWrite> for Tuple {
    fn from(write: TupleWrite) -> Self {
        Self {
            object: write.object,
            relation: write.relation,
            subject: write.subject,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TupleFilter {
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    pub relation: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
    pub subject_relation: Option<Option<String>>,
}

impl TupleFilter {
    pub fn matches(&self, tuple: &Tuple) -> bool {
        if let Some(ref ot) = self.object_type
            && ot != &tuple.object.object_type
        {
            return false;
        }
        if let Some(ref oi) = self.object_id
            && oi != &tuple.object.object_id
        {
            return false;
        }
        if let Some(ref r) = self.relation
            && r != &tuple.relation
        {
            return false;
        }
        if let Some(ref st) = self.subject_type
            && st != &tuple.subject.subject_type
        {
            return false;
        }
        if let Some(ref si) = self.subject_id
            && si != &tuple.subject.subject_id
        {
            return false;
        }
        if let Some(ref sr) = self.subject_relation
            && sr != &tuple.subject.subject_relation
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotToken(u64);

impl SnapshotToken {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for SnapshotToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- TenantId ---

    #[test]
    fn tenant_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let tenant_id = TenantId::from(uuid);

        assert_eq!(*tenant_id.as_uuid(), uuid);
    }

    #[test]
    fn tenant_id_display_matches_uuid() {
        let uuid = Uuid::new_v4();
        let tenant_id = TenantId::new(uuid);

        assert_eq!(tenant_id.to_string(), uuid.to_string());
    }

    #[test]
    fn tenant_id_equality() {
        let uuid = Uuid::new_v4();
        let a = TenantId::new(uuid);
        let b = TenantId::new(uuid);

        assert_eq!(a, b);
    }

    #[test]
    fn tenant_id_clone() {
        let tenant_id = TenantId::new(Uuid::new_v4());
        let cloned = tenant_id.clone();

        assert_eq!(tenant_id, cloned);
    }

    #[test]
    fn tenant_id_hash_consistent_with_equality() {
        use std::collections::HashSet;

        let uuid = Uuid::new_v4();
        let a = TenantId::new(uuid);
        let b = TenantId::new(uuid);

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    // --- ObjectRef ---

    #[test]
    fn object_ref_display() {
        let obj = ObjectRef::new("document", "readme");

        assert_eq!(obj.to_string(), "document:readme");
    }

    #[test]
    fn object_ref_equality() {
        let a = ObjectRef::new("document", "readme");
        let b = ObjectRef::new("document", "readme");

        assert_eq!(a, b);
    }

    // --- SubjectRef ---

    #[test]
    fn direct_subject_display() {
        let subject = SubjectRef::direct("user", "john");

        assert_eq!(subject.to_string(), "user:john");
        assert_eq!(subject.subject_relation, None);
    }

    #[test]
    fn userset_subject_display() {
        let subject = SubjectRef::userset("group", "engineering", "member");

        assert_eq!(subject.to_string(), "group:engineering#member");
        assert_eq!(subject.subject_relation, Some("member".to_string()));
    }

    // --- Tuple ---

    #[test]
    fn tuple_display_direct_subject() {
        let tuple = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        assert_eq!(tuple.to_string(), "object:readme#viewer@user:john");
    }

    #[test]
    fn tuple_display_userset_subject() {
        let tuple = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::userset("group", "engineering", "member"),
        );

        assert_eq!(
            tuple.to_string(),
            "object:readme#viewer@group:engineering#member"
        );
    }

    // --- TupleWrite ---

    #[test]
    fn tuple_write_holds_same_fields_as_tuple() {
        let write = TupleWrite::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        assert_eq!(write.object, ObjectRef::new("object", "readme"));
        assert_eq!(write.relation, "viewer");
        assert_eq!(write.subject, SubjectRef::direct("user", "john"));
    }

    // --- From<TupleWrite> for Tuple ---

    #[test]
    fn tuple_from_tuple_write() {
        let write = TupleWrite::new(
            ObjectRef::new("doc", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        let tuple: Tuple = write.into();

        assert_eq!(tuple.object, ObjectRef::new("doc", "readme"));
        assert_eq!(tuple.relation, "viewer");
        assert_eq!(tuple.subject, SubjectRef::direct("user", "john"));
    }

    #[test]
    fn tuple_hash_consistent_with_equality() {
        use std::collections::HashSet;

        let a = Tuple::new(
            ObjectRef::new("doc", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );
        let b = Tuple::new(
            ObjectRef::new("doc", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
    }

    // --- TupleFilter ---

    #[test]
    fn empty_filter_matches_everything() {
        let filter = TupleFilter::default();
        let tuple = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        assert!(filter.matches(&tuple));
    }

    #[test]
    fn filter_by_object_type() {
        let filter = TupleFilter {
            object_type: Some("object".to_string()),
            ..Default::default()
        };
        let matching = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );
        let non_matching = Tuple::new(
            ObjectRef::new("category", "docs"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        assert!(filter.matches(&matching));
        assert!(!filter.matches(&non_matching));
    }

    #[test]
    fn filter_by_subject_relation_none_matches_direct_only() {
        let filter = TupleFilter {
            subject_relation: Some(None),
            ..Default::default()
        };
        let direct = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );
        let userset = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::userset("group", "eng", "member"),
        );

        assert!(filter.matches(&direct));
        assert!(!filter.matches(&userset));
    }

    #[test]
    fn filter_by_subject_relation_some_matches_userset() {
        let filter = TupleFilter {
            subject_relation: Some(Some("member".to_string())),
            ..Default::default()
        };
        let matching = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::userset("group", "eng", "member"),
        );
        let non_matching_direct = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );
        let non_matching_relation = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::userset("group", "eng", "admin"),
        );

        assert!(filter.matches(&matching));
        assert!(!filter.matches(&non_matching_direct));
        assert!(!filter.matches(&non_matching_relation));
    }

    #[test]
    fn filter_all_fields() {
        let filter = TupleFilter {
            object_type: Some("object".to_string()),
            object_id: Some("readme".to_string()),
            relation: Some("viewer".to_string()),
            subject_type: Some("user".to_string()),
            subject_id: Some("john".to_string()),
            subject_relation: Some(None),
        };
        let matching = Tuple::new(
            ObjectRef::new("object", "readme"),
            "viewer",
            SubjectRef::direct("user", "john"),
        );

        assert!(filter.matches(&matching));
    }

    // --- SnapshotToken ---

    #[test]
    fn snapshot_token_value_round_trip() {
        let token = SnapshotToken::new(42);

        assert_eq!(token.value(), 42);
    }

    #[test]
    fn snapshot_token_display() {
        let token = SnapshotToken::new(42);

        assert_eq!(token.to_string(), "42");
    }

    #[test]
    fn snapshot_token_ordering() {
        let a = SnapshotToken::new(1);
        let b = SnapshotToken::new(2);

        assert!(a < b);
        assert!(b > a);
        assert_eq!(SnapshotToken::new(3), SnapshotToken::new(3));
    }
}
