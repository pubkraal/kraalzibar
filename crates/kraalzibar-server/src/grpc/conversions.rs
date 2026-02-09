use kraalzibar_core::tuple::{
    ObjectRef, SnapshotToken, SubjectRef, Tuple, TupleFilter, TupleWrite,
};

use crate::proto::kraalzibar::v1;
use crate::service::Consistency;

pub fn proto_object_to_domain(obj: &v1::ObjectReference) -> ObjectRef {
    ObjectRef::new(&obj.object_type, &obj.object_id)
}

pub fn domain_object_to_proto(obj: &ObjectRef) -> v1::ObjectReference {
    v1::ObjectReference {
        object_type: obj.object_type.clone(),
        object_id: obj.object_id.clone(),
    }
}

pub fn proto_subject_to_domain(subj: &v1::SubjectReference) -> SubjectRef {
    match &subj.subject_relation {
        Some(rel) => SubjectRef::userset(&subj.subject_type, &subj.subject_id, rel),
        None => SubjectRef::direct(&subj.subject_type, &subj.subject_id),
    }
}

pub fn domain_subject_to_proto(subj: &SubjectRef) -> v1::SubjectReference {
    v1::SubjectReference {
        subject_type: subj.subject_type.clone(),
        subject_id: subj.subject_id.clone(),
        subject_relation: subj.subject_relation.clone(),
    }
}

pub fn domain_tuple_to_proto(tuple: &Tuple) -> v1::Relationship {
    v1::Relationship {
        object: Some(domain_object_to_proto(&tuple.object)),
        relation: tuple.relation.clone(),
        subject: Some(domain_subject_to_proto(&tuple.subject)),
    }
}

#[allow(clippy::result_large_err)]
pub fn proto_relationship_to_write(rel: &v1::Relationship) -> Result<TupleWrite, tonic::Status> {
    let object = rel
        .object
        .as_ref()
        .map(proto_object_to_domain)
        .ok_or_else(|| tonic::Status::invalid_argument("relationship object is required"))?;
    let subject = rel
        .subject
        .as_ref()
        .map(proto_subject_to_domain)
        .ok_or_else(|| tonic::Status::invalid_argument("relationship subject is required"))?;

    Ok(TupleWrite::new(object, &rel.relation, subject))
}

pub fn proto_filter_to_domain(filter: &v1::RelationshipFilter) -> TupleFilter {
    TupleFilter {
        object_type: filter.object_type.clone(),
        object_id: filter.object_id.clone(),
        relation: filter.relation.clone(),
        subject_type: filter.subject_type.clone(),
        subject_id: filter.subject_id.clone(),
        subject_relation: None,
    }
}

#[allow(clippy::result_large_err)]
pub fn proto_consistency_to_domain(
    consistency: Option<&v1::Consistency>,
) -> Result<Consistency, tonic::Status> {
    match consistency.and_then(|c| c.requirement.as_ref()) {
        Some(v1::consistency::Requirement::FullConsistency(_)) => Ok(Consistency::FullConsistency),
        Some(v1::consistency::Requirement::AtLeastAsFresh(token)) => {
            Ok(Consistency::AtLeastAsFresh(zed_token_to_snapshot(token)?))
        }
        Some(v1::consistency::Requirement::AtExactSnapshot(token)) => {
            Ok(Consistency::AtExactSnapshot(zed_token_to_snapshot(token)?))
        }
        _ => Ok(Consistency::MinimizeLatency),
    }
}

pub fn snapshot_to_zed_token(snapshot: Option<SnapshotToken>) -> Option<v1::ZedToken> {
    snapshot.map(|s| v1::ZedToken {
        token: s.value().to_string(),
    })
}

#[allow(clippy::result_large_err)]
fn zed_token_to_snapshot(token: &v1::ZedToken) -> Result<SnapshotToken, tonic::Status> {
    let value: u64 = token
        .token
        .parse()
        .map_err(|_| tonic::Status::invalid_argument("invalid snapshot token format"))?;
    Ok(SnapshotToken::new(value))
}

pub fn domain_expand_tree_to_proto(
    tree: &kraalzibar_core::engine::ExpandTree,
) -> v1::PermissionExpansionTree {
    use kraalzibar_core::engine::ExpandTree;
    use v1::permission_expansion_tree::*;

    match tree {
        ExpandTree::Leaf { subject } => v1::PermissionExpansionTree {
            node: Some(Node::Leaf(LeafNode {
                subject: Some(domain_subject_to_proto(subject)),
            })),
        },
        ExpandTree::This { subjects, .. } => {
            let children: Vec<_> = subjects.iter().map(domain_expand_tree_to_proto).collect();
            v1::PermissionExpansionTree {
                node: Some(Node::Intermediate(IntermediateNode {
                    operation: intermediate_node::Operation::Union as i32,
                    children,
                })),
            }
        }
        ExpandTree::Union { children } => {
            let children: Vec<_> = children.iter().map(domain_expand_tree_to_proto).collect();
            v1::PermissionExpansionTree {
                node: Some(Node::Intermediate(IntermediateNode {
                    operation: intermediate_node::Operation::Union as i32,
                    children,
                })),
            }
        }
        ExpandTree::Intersection { children } => {
            let children: Vec<_> = children.iter().map(domain_expand_tree_to_proto).collect();
            v1::PermissionExpansionTree {
                node: Some(Node::Intermediate(IntermediateNode {
                    operation: intermediate_node::Operation::Intersection as i32,
                    children,
                })),
            }
        }
        ExpandTree::Exclusion { base, excluded } => v1::PermissionExpansionTree {
            node: Some(Node::Intermediate(IntermediateNode {
                operation: intermediate_node::Operation::Exclusion as i32,
                children: vec![
                    domain_expand_tree_to_proto(base),
                    domain_expand_tree_to_proto(excluded),
                ],
            })),
        },
        ExpandTree::Arrow { children, .. } => {
            let children: Vec<_> = children.iter().map(domain_expand_tree_to_proto).collect();
            v1::PermissionExpansionTree {
                node: Some(Node::Intermediate(IntermediateNode {
                    operation: intermediate_node::Operation::Union as i32,
                    children,
                })),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_object_reference_round_trip() {
        let domain = ObjectRef::new("document", "readme");
        let proto = domain_object_to_proto(&domain);
        let back = proto_object_to_domain(&proto);

        assert_eq!(back.object_type, "document");
        assert_eq!(back.object_id, "readme");
    }

    #[test]
    fn convert_direct_subject_round_trip() {
        let domain = SubjectRef::direct("user", "alice");
        let proto = domain_subject_to_proto(&domain);
        let back = proto_subject_to_domain(&proto);

        assert_eq!(back.subject_type, "user");
        assert_eq!(back.subject_id, "alice");
        assert!(back.subject_relation.is_none());
    }

    #[test]
    fn convert_userset_subject_round_trip() {
        let domain = SubjectRef::userset("group", "eng", "member");
        let proto = domain_subject_to_proto(&domain);
        let back = proto_subject_to_domain(&proto);

        assert_eq!(back.subject_type, "group");
        assert_eq!(back.subject_id, "eng");
        assert_eq!(back.subject_relation.as_deref(), Some("member"));
    }

    #[test]
    fn convert_relationship_round_trip() {
        let tuple = Tuple::new(
            ObjectRef::new("document", "readme"),
            "viewer",
            SubjectRef::direct("user", "alice"),
        );
        let proto = domain_tuple_to_proto(&tuple);
        let write = proto_relationship_to_write(&proto).unwrap();

        assert_eq!(write.object.object_type, "document");
        assert_eq!(write.relation, "viewer");
        assert_eq!(write.subject.subject_id, "alice");
    }

    #[test]
    fn convert_filter() {
        let proto = v1::RelationshipFilter {
            object_type: Some("document".to_string()),
            object_id: None,
            relation: Some("viewer".to_string()),
            subject_type: None,
            subject_id: None,
        };
        let domain = proto_filter_to_domain(&proto);

        assert_eq!(domain.object_type.as_deref(), Some("document"));
        assert!(domain.object_id.is_none());
        assert_eq!(domain.relation.as_deref(), Some("viewer"));
    }

    #[test]
    fn convert_consistency_full() {
        let proto = v1::Consistency {
            requirement: Some(v1::consistency::Requirement::FullConsistency(true)),
        };
        let domain = proto_consistency_to_domain(Some(&proto)).unwrap();
        assert_eq!(domain, Consistency::FullConsistency);
    }

    #[test]
    fn convert_consistency_minimize_latency() {
        let proto = v1::Consistency {
            requirement: Some(v1::consistency::Requirement::MinimizeLatency(true)),
        };
        let domain = proto_consistency_to_domain(Some(&proto)).unwrap();
        assert_eq!(domain, Consistency::MinimizeLatency);
    }

    #[test]
    fn convert_consistency_at_least_as_fresh() {
        let proto = v1::Consistency {
            requirement: Some(v1::consistency::Requirement::AtLeastAsFresh(v1::ZedToken {
                token: "42".to_string(),
            })),
        };
        let domain = proto_consistency_to_domain(Some(&proto)).unwrap();
        assert_eq!(domain, Consistency::AtLeastAsFresh(SnapshotToken::new(42)));
    }

    #[test]
    fn convert_consistency_none_defaults_to_minimize_latency() {
        let domain = proto_consistency_to_domain(None).unwrap();
        assert_eq!(domain, Consistency::MinimizeLatency);
    }

    #[test]
    fn convert_consistency_rejects_invalid_token() {
        let proto = v1::Consistency {
            requirement: Some(v1::consistency::Requirement::AtLeastAsFresh(v1::ZedToken {
                token: "not-a-number".to_string(),
            })),
        };
        let result = proto_consistency_to_domain(Some(&proto));
        assert!(result.is_err());
    }

    #[test]
    fn convert_snapshot_to_zed_token() {
        let token = snapshot_to_zed_token(Some(SnapshotToken::new(99)));
        assert_eq!(token.unwrap().token, "99");

        let none = snapshot_to_zed_token(None);
        assert!(none.is_none());
    }
}
