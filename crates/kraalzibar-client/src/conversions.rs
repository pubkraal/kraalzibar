use kraalzibar_core::tuple::{ObjectRef, SnapshotToken, SubjectRef, TupleFilter};

use crate::proto::kraalzibar::v1;

pub fn domain_object_to_proto(obj: &ObjectRef) -> v1::ObjectReference {
    v1::ObjectReference {
        object_type: obj.object_type.clone(),
        object_id: obj.object_id.clone(),
    }
}

pub fn proto_object_to_domain(obj: &v1::ObjectReference) -> ObjectRef {
    ObjectRef::new(&obj.object_type, &obj.object_id)
}

pub fn domain_subject_to_proto(subj: &SubjectRef) -> v1::SubjectReference {
    v1::SubjectReference {
        subject_type: subj.subject_type.clone(),
        subject_id: subj.subject_id.clone(),
        subject_relation: subj.subject_relation.clone(),
    }
}

pub fn proto_subject_to_domain(subj: &v1::SubjectReference) -> SubjectRef {
    match &subj.subject_relation {
        Some(rel) => SubjectRef::userset(&subj.subject_type, &subj.subject_id, rel),
        None => SubjectRef::direct(&subj.subject_type, &subj.subject_id),
    }
}

pub fn domain_filter_to_proto(filter: &TupleFilter) -> v1::RelationshipFilter {
    v1::RelationshipFilter {
        object_type: filter.object_type.clone(),
        object_id: filter.object_id.clone(),
        relation: filter.relation.clone(),
        subject_type: filter.subject_type.clone(),
        subject_id: filter.subject_id.clone(),
    }
}

pub fn consistency_to_proto(consistency: &Consistency) -> v1::Consistency {
    match consistency {
        Consistency::FullConsistency => v1::Consistency {
            requirement: Some(v1::consistency::Requirement::FullConsistency(true)),
        },
        Consistency::MinimizeLatency => v1::Consistency {
            requirement: Some(v1::consistency::Requirement::MinimizeLatency(true)),
        },
        Consistency::AtLeastAsFresh(token) => v1::Consistency {
            requirement: Some(v1::consistency::Requirement::AtLeastAsFresh(v1::ZedToken {
                token: token.value().to_string(),
            })),
        },
        Consistency::AtExactSnapshot(token) => v1::Consistency {
            requirement: Some(v1::consistency::Requirement::AtExactSnapshot(
                v1::ZedToken {
                    token: token.value().to_string(),
                },
            )),
        },
    }
}

pub fn zed_token_to_snapshot(token: &v1::ZedToken) -> Option<SnapshotToken> {
    token.token.parse::<u64>().ok().map(SnapshotToken::new)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Consistency {
    FullConsistency,
    #[default]
    MinimizeLatency,
    AtLeastAsFresh(SnapshotToken),
    AtExactSnapshot(SnapshotToken),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consistency_converts_to_proto_full() {
        let proto = consistency_to_proto(&Consistency::FullConsistency);
        assert!(matches!(
            proto.requirement,
            Some(v1::consistency::Requirement::FullConsistency(_))
        ));
    }

    #[test]
    fn consistency_converts_to_proto_minimize_latency() {
        let proto = consistency_to_proto(&Consistency::MinimizeLatency);
        assert!(matches!(
            proto.requirement,
            Some(v1::consistency::Requirement::MinimizeLatency(_))
        ));
    }

    #[test]
    fn consistency_converts_to_proto_at_least_as_fresh() {
        let proto = consistency_to_proto(&Consistency::AtLeastAsFresh(SnapshotToken::new(42)));
        match proto.requirement {
            Some(v1::consistency::Requirement::AtLeastAsFresh(token)) => {
                assert_eq!(token.token, "42");
            }
            _ => panic!("expected AtLeastAsFresh"),
        }
    }

    #[test]
    fn consistency_converts_to_proto_at_exact_snapshot() {
        let proto = consistency_to_proto(&Consistency::AtExactSnapshot(SnapshotToken::new(99)));
        match proto.requirement {
            Some(v1::consistency::Requirement::AtExactSnapshot(token)) => {
                assert_eq!(token.token, "99");
            }
            _ => panic!("expected AtExactSnapshot"),
        }
    }

    #[test]
    fn check_permission_request_converts_to_proto() {
        let obj = ObjectRef::new("document", "readme");
        let subj = SubjectRef::direct("user", "alice");

        let proto_obj = domain_object_to_proto(&obj);
        let proto_subj = domain_subject_to_proto(&subj);

        assert_eq!(proto_obj.object_type, "document");
        assert_eq!(proto_obj.object_id, "readme");
        assert_eq!(proto_subj.subject_type, "user");
        assert_eq!(proto_subj.subject_id, "alice");
        assert!(proto_subj.subject_relation.is_none());
    }

    #[test]
    fn write_relationships_request_converts_to_proto() {
        let obj = ObjectRef::new("document", "readme");
        let subj = SubjectRef::userset("group", "eng", "member");

        let proto_obj = domain_object_to_proto(&obj);
        let proto_subj = domain_subject_to_proto(&subj);

        assert_eq!(proto_subj.subject_relation.as_deref(), Some("member"));

        let back_obj = proto_object_to_domain(&proto_obj);
        let back_subj = proto_subject_to_domain(&proto_subj);

        assert_eq!(back_obj, obj);
        assert_eq!(back_subj, subj);
    }

    #[test]
    fn read_relationships_filter_converts_to_proto() {
        let filter = TupleFilter {
            object_type: Some("document".to_string()),
            object_id: None,
            relation: Some("viewer".to_string()),
            subject_type: None,
            subject_id: None,
            subject_relation: None,
        };

        let proto = domain_filter_to_proto(&filter);

        assert_eq!(proto.object_type.as_deref(), Some("document"));
        assert!(proto.object_id.is_none());
        assert_eq!(proto.relation.as_deref(), Some("viewer"));
        assert!(proto.subject_type.is_none());
        assert!(proto.subject_id.is_none());
    }

    #[test]
    fn zed_token_parses_to_snapshot() {
        let token = v1::ZedToken {
            token: "42".to_string(),
        };
        let snapshot = zed_token_to_snapshot(&token);
        assert_eq!(snapshot, Some(SnapshotToken::new(42)));
    }

    #[test]
    fn zed_token_invalid_returns_none() {
        let token = v1::ZedToken {
            token: "not-a-number".to_string(),
        };
        let snapshot = zed_token_to_snapshot(&token);
        assert!(snapshot.is_none());
    }
}
