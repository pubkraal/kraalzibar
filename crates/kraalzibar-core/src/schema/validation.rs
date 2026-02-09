use super::types::Schema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaLimits {
    pub max_types: usize,
    pub max_relations_per_type: usize,
    pub max_permissions_per_type: usize,
}

impl Default for SchemaLimits {
    fn default() -> Self {
        Self {
            max_types: 50,
            max_relations_per_type: 30,
            max_permissions_per_type: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("too many types: {count} exceeds limit of {limit}")]
    TooManyTypes { count: usize, limit: usize },
    #[error("too many relations in type '{type_name}': {count} exceeds limit of {limit}")]
    TooManyRelations {
        type_name: String,
        count: usize,
        limit: usize,
    },
    #[error("too many permissions in type '{type_name}': {count} exceeds limit of {limit}")]
    TooManyPermissions {
        type_name: String,
        count: usize,
        limit: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakingChange {
    TypeRemoved { type_name: String },
    RelationRemoved { type_name: String, relation: String },
    SubjectTypesChanged { type_name: String, relation: String },
}

pub fn validate_schema_limits(
    schema: &Schema,
    limits: &SchemaLimits,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    if schema.types.len() > limits.max_types {
        errors.push(ValidationError::TooManyTypes {
            count: schema.types.len(),
            limit: limits.max_types,
        });
    }

    for type_def in &schema.types {
        if type_def.relations.len() > limits.max_relations_per_type {
            errors.push(ValidationError::TooManyRelations {
                type_name: type_def.name.clone(),
                count: type_def.relations.len(),
                limit: limits.max_relations_per_type,
            });
        }
        if type_def.permissions.len() > limits.max_permissions_per_type {
            errors.push(ValidationError::TooManyPermissions {
                type_name: type_def.name.clone(),
                count: type_def.permissions.len(),
                limit: limits.max_permissions_per_type,
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn detect_breaking_changes(old: &Schema, new: &Schema) -> Vec<BreakingChange> {
    let mut changes = Vec::new();

    for old_type in &old.types {
        match new.get_type(&old_type.name) {
            None => {
                changes.push(BreakingChange::TypeRemoved {
                    type_name: old_type.name.clone(),
                });
            }
            Some(new_type) => {
                for old_rel in &old_type.relations {
                    match new_type.relations.iter().find(|r| r.name == old_rel.name) {
                        None => {
                            changes.push(BreakingChange::RelationRemoved {
                                type_name: old_type.name.clone(),
                                relation: old_rel.name.clone(),
                            });
                        }
                        Some(new_rel) => {
                            if old_rel.subject_types != new_rel.subject_types {
                                changes.push(BreakingChange::SubjectTypesChanged {
                                    type_name: old_type.name.clone(),
                                    relation: old_rel.name.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::parse_schema;

    fn limits_with(max_types: usize, max_rels: usize, max_perms: usize) -> SchemaLimits {
        SchemaLimits {
            max_types,
            max_relations_per_type: max_rels,
            max_permissions_per_type: max_perms,
        }
    }

    #[test]
    fn schema_within_limits_passes() {
        let schema =
            parse_schema("definition user {} definition group { relation member: user }").unwrap();
        let limits = SchemaLimits::default();

        assert!(validate_schema_limits(&schema, &limits).is_ok());
    }

    #[test]
    fn exceeding_max_types_rejected() {
        let schema = parse_schema("definition a {} definition b {} definition c {}").unwrap();
        let limits = limits_with(2, 30, 30);

        let errors = validate_schema_limits(&schema, &limits).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0],
            ValidationError::TooManyTypes { count: 3, limit: 2 }
        );
    }

    #[test]
    fn exceeding_max_relations_rejected() {
        let schema =
            parse_schema("definition doc { relation a: user relation b: user relation c: user }")
                .unwrap();
        let limits = limits_with(50, 2, 30);

        let errors = validate_schema_limits(&schema, &limits).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            ValidationError::TooManyRelations {
                type_name,
                count: 3,
                limit: 2
            } if type_name == "doc"
        ));
    }

    #[test]
    fn exceeding_max_permissions_rejected() {
        let schema = parse_schema(
            "definition doc { relation a: user permission p1 = a permission p2 = a permission p3 = a }",
        )
        .unwrap();
        let limits = limits_with(50, 30, 2);

        let errors = validate_schema_limits(&schema, &limits).unwrap_err();

        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            ValidationError::TooManyPermissions {
                type_name,
                count: 3,
                limit: 2
            } if type_name == "doc"
        ));
    }

    #[test]
    fn adding_new_type_is_safe() {
        let old = parse_schema("definition user {}").unwrap();
        let new =
            parse_schema("definition user {} definition group { relation member: user }").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert!(changes.is_empty());
    }

    #[test]
    fn removing_type_is_breaking() {
        let old =
            parse_schema("definition user {} definition group { relation member: user }").unwrap();
        let new = parse_schema("definition user {}").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0],
            BreakingChange::TypeRemoved {
                type_name: "group".to_string()
            }
        );
    }

    #[test]
    fn removing_relation_is_breaking() {
        let old =
            parse_schema("definition doc { relation owner: user relation editor: user }").unwrap();
        let new = parse_schema("definition doc { relation owner: user }").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0],
            BreakingChange::RelationRemoved {
                type_name: "doc".to_string(),
                relation: "editor".to_string(),
            }
        );
    }

    #[test]
    fn adding_relation_is_safe() {
        let old = parse_schema("definition doc { relation owner: user }").unwrap();
        let new =
            parse_schema("definition doc { relation owner: user relation editor: user }").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert!(changes.is_empty());
    }

    #[test]
    fn changing_subject_types_is_breaking() {
        let old = parse_schema("definition doc { relation editor: user | group#member }").unwrap();
        let new = parse_schema("definition doc { relation editor: user }").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0],
            BreakingChange::SubjectTypesChanged {
                type_name: "doc".to_string(),
                relation: "editor".to_string(),
            }
        );
    }

    #[test]
    fn changing_permission_rule_is_not_breaking() {
        let old = parse_schema(
            "definition doc { relation owner: user relation editor: user permission can_edit = owner }",
        )
        .unwrap();
        let new = parse_schema(
            "definition doc { relation owner: user relation editor: user permission can_edit = owner + editor }",
        )
        .unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert!(changes.is_empty());
    }

    #[test]
    fn adding_permission_is_safe() {
        let old = parse_schema("definition doc { relation owner: user }").unwrap();
        let new =
            parse_schema("definition doc { relation owner: user permission can_edit = owner }")
                .unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert!(changes.is_empty());
    }

    #[test]
    fn removing_permission_is_not_breaking() {
        let old =
            parse_schema("definition doc { relation owner: user permission can_edit = owner }")
                .unwrap();
        let new = parse_schema("definition doc { relation owner: user }").unwrap();

        let changes = detect_breaking_changes(&old, &new);

        assert!(changes.is_empty());
    }
}
