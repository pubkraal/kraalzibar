use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::schema::types::{RewriteRule, Schema};
use crate::tuple::{SnapshotToken, TupleFilter};

use super::{CheckError, EngineConfig, TupleReader};

pub struct CheckRequest {
    pub object_type: String,
    pub object_id: String,
    pub permission: String,
    pub subject_type: String,
    pub subject_id: String,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug)]
pub struct CheckResult {
    pub allowed: bool,
}

struct CheckContext<'a> {
    subject_type: &'a str,
    subject_id: &'a str,
    snapshot: Option<SnapshotToken>,
}

pub struct CheckEngine<T: TupleReader> {
    reader: Arc<T>,
    schema: Arc<Schema>,
    #[allow(dead_code)]
    config: EngineConfig,
}

impl<T: TupleReader> CheckEngine<T> {
    pub fn new(reader: Arc<T>, schema: Arc<Schema>, config: EngineConfig) -> Self {
        Self {
            reader,
            schema,
            config,
        }
    }

    pub async fn check(&self, request: &CheckRequest) -> Result<CheckResult, CheckError> {
        let type_def = self
            .schema
            .get_type(&request.object_type)
            .ok_or_else(|| CheckError::TypeNotFound(request.object_type.clone()))?;

        let perm_def = type_def
            .get_permission(&request.permission)
            .ok_or_else(|| CheckError::PermissionNotFound {
                type_name: request.object_type.clone(),
                permission: request.permission.clone(),
            })?;

        let ctx = CheckContext {
            subject_type: &request.subject_type,
            subject_id: &request.subject_id,
            snapshot: request.snapshot,
        };

        let allowed = self
            .evaluate_rule(
                &perm_def.rule,
                &request.object_type,
                &request.object_id,
                &ctx,
            )
            .await?;

        Ok(CheckResult { allowed })
    }

    fn evaluate_rule<'a>(
        &'a self,
        rule: &'a RewriteRule,
        object_type: &'a str,
        object_id: &'a str,
        ctx: &'a CheckContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<bool, CheckError>> + Send + 'a>> {
        Box::pin(async move {
            match rule {
                RewriteRule::This(name) => {
                    self.evaluate_this(name, object_type, object_id, ctx).await
                }
                RewriteRule::Union(children) => {
                    for child in children {
                        if self
                            .evaluate_rule(child, object_type, object_id, ctx)
                            .await?
                        {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                RewriteRule::Intersection(children) => {
                    for child in children {
                        if !self
                            .evaluate_rule(child, object_type, object_id, ctx)
                            .await?
                        {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                RewriteRule::Exclusion(base, excluded) => {
                    let base_result = self
                        .evaluate_rule(base, object_type, object_id, ctx)
                        .await?;
                    if !base_result {
                        return Ok(false);
                    }
                    let excluded_result = self
                        .evaluate_rule(excluded, object_type, object_id, ctx)
                        .await?;
                    Ok(!excluded_result)
                }
                RewriteRule::Arrow(tupleset_rel, computed_perm) => {
                    self.evaluate_arrow(tupleset_rel, computed_perm, object_type, object_id, ctx)
                        .await
                }
            }
        })
    }

    async fn evaluate_this(
        &self,
        name: &str,
        object_type: &str,
        object_id: &str,
        ctx: &CheckContext<'_>,
    ) -> Result<bool, CheckError> {
        let type_def = self
            .schema
            .get_type(object_type)
            .ok_or_else(|| CheckError::TypeNotFound(object_type.to_string()))?;

        if type_def.get_relation(name).is_some() {
            let filter = TupleFilter {
                object_type: Some(object_type.to_string()),
                object_id: Some(object_id.to_string()),
                relation: Some(name.to_string()),
                ..Default::default()
            };

            let tuples = self.reader.read_tuples(&filter, ctx.snapshot).await?;

            for tuple in &tuples {
                if tuple.subject.subject_relation.is_none() {
                    if tuple.subject.subject_type == ctx.subject_type
                        && tuple.subject.subject_id == ctx.subject_id
                    {
                        return Ok(true);
                    }
                } else if let Some(ref rel) = tuple.subject.subject_relation {
                    let userset_filter = TupleFilter {
                        object_type: Some(tuple.subject.subject_type.clone()),
                        object_id: Some(tuple.subject.subject_id.clone()),
                        relation: Some(rel.clone()),
                        subject_type: Some(ctx.subject_type.to_string()),
                        subject_id: Some(ctx.subject_id.to_string()),
                        subject_relation: Some(None),
                    };
                    let userset_tuples = self
                        .reader
                        .read_tuples(&userset_filter, ctx.snapshot)
                        .await?;
                    if !userset_tuples.is_empty() {
                        return Ok(true);
                    }
                }
            }

            return Ok(false);
        }

        Err(CheckError::RelationNotFound {
            type_name: object_type.to_string(),
            relation: name.to_string(),
        })
    }

    async fn evaluate_arrow(
        &self,
        tupleset_rel: &str,
        computed_perm: &str,
        object_type: &str,
        object_id: &str,
        ctx: &CheckContext<'_>,
    ) -> Result<bool, CheckError> {
        let filter = TupleFilter {
            object_type: Some(object_type.to_string()),
            object_id: Some(object_id.to_string()),
            relation: Some(tupleset_rel.to_string()),
            ..Default::default()
        };

        let tuples = self.reader.read_tuples(&filter, ctx.snapshot).await?;

        for tuple in &tuples {
            let target_type = &tuple.subject.subject_type;
            let target_id = &tuple.subject.subject_id;

            let target_type_def = self
                .schema
                .get_type(target_type)
                .ok_or_else(|| CheckError::TypeNotFound(target_type.clone()))?;

            let perm_def = target_type_def
                .get_permission(computed_perm)
                .ok_or_else(|| CheckError::PermissionNotFound {
                    type_name: target_type.clone(),
                    permission: computed_perm.to_string(),
                })?;

            if self
                .evaluate_rule(&perm_def.rule, target_type, target_id, ctx)
                .await?
            {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TupleReader;
    use crate::schema::types::{PermissionDef, RelationDef, RewriteRule, TypeDefinition};
    use crate::tuple::{ObjectRef, SubjectRef, Tuple, TupleFilter};

    struct TestStore {
        tuples: Vec<Tuple>,
    }

    impl TestStore {
        fn new(tuples: Vec<Tuple>) -> Self {
            Self { tuples }
        }
    }

    impl TupleReader for TestStore {
        async fn read_tuples(
            &self,
            filter: &TupleFilter,
            _snapshot: Option<SnapshotToken>,
        ) -> Result<Vec<Tuple>, CheckError> {
            Ok(self
                .tuples
                .iter()
                .filter(|t| filter.matches(t))
                .cloned()
                .collect())
        }
    }

    fn make_engine(schema: Schema, tuples: Vec<Tuple>) -> CheckEngine<TestStore> {
        CheckEngine::new(
            Arc::new(TestStore::new(tuples)),
            Arc::new(schema),
            EngineConfig::default(),
        )
    }

    #[allow(dead_code)]
    fn simple_schema_with_permission(
        type_name: &str,
        permission_name: &str,
        rule: RewriteRule,
    ) -> Schema {
        Schema {
            types: vec![TypeDefinition {
                name: type_name.to_string(),
                relations: vec![],
                permissions: vec![PermissionDef {
                    name: permission_name.to_string(),
                    rule,
                }],
            }],
        }
    }

    fn doc_schema_with_viewer() -> Schema {
        Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![RelationDef {
                    name: "viewer".to_string(),
                    subject_types: vec![],
                }],
                permissions: vec![PermissionDef {
                    name: "view".to_string(),
                    rule: RewriteRule::This("viewer".to_string()),
                }],
            }],
        }
    }

    #[tokio::test]
    async fn check_rejects_unknown_type() {
        let schema = Schema { types: vec![] };
        let engine = make_engine(schema, vec![]);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let err = engine.check(&request).await.unwrap_err();
        assert!(
            matches!(err, CheckError::TypeNotFound(ref t) if t == "document"),
            "expected TypeNotFound, got: {err}"
        );
    }

    #[tokio::test]
    async fn check_rejects_unknown_permission() {
        let schema = Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![],
                permissions: vec![],
            }],
        };
        let engine = make_engine(schema, vec![]);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let err = engine.check(&request).await.unwrap_err();
        assert!(
            matches!(
                err,
                CheckError::PermissionNotFound {
                    ref type_name,
                    ref permission,
                } if type_name == "document" && permission == "view"
            ),
            "expected PermissionNotFound, got: {err}"
        );
    }

    #[tokio::test]
    async fn check_direct_relation_grants_access() {
        let schema = doc_schema_with_viewer();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "viewer",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_direct_relation_denies_no_tuple() {
        let schema = doc_schema_with_viewer();
        let engine = make_engine(schema, vec![]);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn check_direct_relation_denies_different_subject() {
        let schema = doc_schema_with_viewer();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "viewer",
            SubjectRef::direct("user", "bob"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    fn doc_schema_with_union_view() -> Schema {
        Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![
                    RelationDef {
                        name: "owner".to_string(),
                        subject_types: vec![],
                    },
                    RelationDef {
                        name: "editor".to_string(),
                        subject_types: vec![],
                    },
                    RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    },
                ],
                permissions: vec![PermissionDef {
                    name: "view".to_string(),
                    rule: RewriteRule::Union(vec![
                        RewriteRule::This("owner".to_string()),
                        RewriteRule::This("editor".to_string()),
                        RewriteRule::This("viewer".to_string()),
                    ]),
                }],
            }],
        }
    }

    #[tokio::test]
    async fn check_union_grants_on_first_branch() {
        let schema = doc_schema_with_union_view();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "owner",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_union_grants_on_second_branch() {
        let schema = doc_schema_with_union_view();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "editor",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_union_denies_when_no_branch_matches() {
        let schema = doc_schema_with_union_view();
        let engine = make_engine(schema, vec![]);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    fn doc_schema_with_intersection() -> Schema {
        Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![
                    RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    },
                    RelationDef {
                        name: "allowed_ip".to_string(),
                        subject_types: vec![],
                    },
                ],
                permissions: vec![PermissionDef {
                    name: "view".to_string(),
                    rule: RewriteRule::Intersection(vec![
                        RewriteRule::This("viewer".to_string()),
                        RewriteRule::This("allowed_ip".to_string()),
                    ]),
                }],
            }],
        }
    }

    #[tokio::test]
    async fn check_intersection_grants_when_all_match() {
        let schema = doc_schema_with_intersection();
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            ),
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "allowed_ip",
                SubjectRef::direct("user", "alice"),
            ),
        ];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_intersection_denies_when_one_missing() {
        let schema = doc_schema_with_intersection();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "viewer",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    fn doc_schema_with_exclusion() -> Schema {
        Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![
                    RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    },
                    RelationDef {
                        name: "blocked".to_string(),
                        subject_types: vec![],
                    },
                ],
                permissions: vec![PermissionDef {
                    name: "view".to_string(),
                    rule: RewriteRule::Exclusion(
                        Box::new(RewriteRule::This("viewer".to_string())),
                        Box::new(RewriteRule::This("blocked".to_string())),
                    ),
                }],
            }],
        }
    }

    #[tokio::test]
    async fn check_exclusion_grants() {
        let schema = doc_schema_with_exclusion();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "viewer",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_exclusion_denies_when_excluded() {
        let schema = doc_schema_with_exclusion();
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            ),
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "blocked",
                SubjectRef::direct("user", "alice"),
            ),
        ];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn check_exclusion_denies_when_base_false() {
        let schema = doc_schema_with_exclusion();
        let engine = make_engine(schema, vec![]);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    fn doc_and_folder_schema() -> Schema {
        Schema {
            types: vec![
                TypeDefinition {
                    name: "folder".to_string(),
                    relations: vec![RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    }],
                    permissions: vec![PermissionDef {
                        name: "view".to_string(),
                        rule: RewriteRule::This("viewer".to_string()),
                    }],
                },
                TypeDefinition {
                    name: "document".to_string(),
                    relations: vec![RelationDef {
                        name: "parent".to_string(),
                        subject_types: vec![],
                    }],
                    permissions: vec![PermissionDef {
                        name: "view".to_string(),
                        rule: RewriteRule::Arrow("parent".to_string(), "view".to_string()),
                    }],
                },
            ],
        }
    }

    #[tokio::test]
    async fn check_arrow_follows_relation_to_parent() {
        let schema = doc_and_folder_schema();
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "parent",
                SubjectRef::direct("folder", "root"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "root"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            ),
        ];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn check_arrow_denies_when_parent_denies() {
        let schema = doc_and_folder_schema();
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "parent",
            SubjectRef::direct("folder", "root"),
        )];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn check_userset_traverses_group_membership() {
        // Schema: document has viewer relation, group has member relation
        // document:readme#viewer@group:eng#member (userset subject)
        // group:eng#member@user:alice (alice is a member of eng)
        // Check: can user:alice view document:readme? -> yes, via group membership
        let schema = Schema {
            types: vec![
                TypeDefinition {
                    name: "group".to_string(),
                    relations: vec![RelationDef {
                        name: "member".to_string(),
                        subject_types: vec![],
                    }],
                    permissions: vec![],
                },
                TypeDefinition {
                    name: "document".to_string(),
                    relations: vec![RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    }],
                    permissions: vec![PermissionDef {
                        name: "view".to_string(),
                        rule: RewriteRule::This("viewer".to_string()),
                    }],
                },
            ],
        };
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "viewer",
                SubjectRef::userset("group", "eng", "member"),
            ),
            Tuple::new(
                ObjectRef::new("group", "eng"),
                "member",
                SubjectRef::direct("user", "alice"),
            ),
        ];
        let engine = make_engine(schema, tuples);

        let request = CheckRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            subject_type: "user".to_string(),
            subject_id: "alice".to_string(),
            snapshot: None,
        };

        let result = engine.check(&request).await.unwrap();
        assert!(result.allowed);
    }
}
