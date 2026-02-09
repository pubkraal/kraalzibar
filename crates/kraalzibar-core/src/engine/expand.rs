use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::schema::types::{RewriteRule, Schema};
use crate::tuple::{SnapshotToken, SubjectRef, TupleFilter};

use super::{CheckError, EngineConfig, TupleReader};

pub struct ExpandRequest {
    pub object_type: String,
    pub object_id: String,
    pub permission: String,
    pub snapshot: Option<SnapshotToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpandTree {
    Leaf {
        subject: SubjectRef,
    },
    This {
        relation: String,
        subjects: Vec<ExpandTree>,
    },
    Union {
        children: Vec<ExpandTree>,
    },
    Intersection {
        children: Vec<ExpandTree>,
    },
    Exclusion {
        base: Box<ExpandTree>,
        excluded: Box<ExpandTree>,
    },
    Arrow {
        tupleset_relation: String,
        computed_permission: String,
        children: Vec<ExpandTree>,
    },
}

struct ExpandContext {
    snapshot: Option<SnapshotToken>,
    depth: usize,
    visited: HashSet<(String, String, String)>,
}

pub struct ExpandEngine<T: TupleReader> {
    reader: Arc<T>,
    schema: Arc<Schema>,
    config: EngineConfig,
}

impl<T: TupleReader> ExpandEngine<T> {
    pub fn new(reader: Arc<T>, schema: Arc<Schema>, config: EngineConfig) -> Self {
        Self {
            reader,
            schema,
            config,
        }
    }

    pub async fn expand(&self, request: &ExpandRequest) -> Result<ExpandTree, CheckError> {
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

        let ctx = ExpandContext {
            snapshot: request.snapshot,
            depth: 0,
            visited: HashSet::new(),
        };
        self.expand_rule(
            &perm_def.rule,
            &request.object_type,
            &request.object_id,
            &ctx,
        )
        .await
    }

    fn expand_rule<'a>(
        &'a self,
        rule: &'a RewriteRule,
        object_type: &'a str,
        object_id: &'a str,
        ctx: &'a ExpandContext,
    ) -> Pin<Box<dyn Future<Output = Result<ExpandTree, CheckError>> + Send + 'a>> {
        Box::pin(async move {
            if ctx.depth > self.config.max_depth {
                return Err(CheckError::MaxDepthExceeded(ctx.depth));
            }

            match rule {
                RewriteRule::This(name) => {
                    self.expand_this(name, object_type, object_id, ctx).await
                }
                RewriteRule::Union(children) => {
                    let mut expanded = Vec::new();
                    for child in children {
                        expanded.push(self.expand_rule(child, object_type, object_id, ctx).await?);
                    }
                    Ok(ExpandTree::Union { children: expanded })
                }
                RewriteRule::Intersection(children) => {
                    let mut expanded = Vec::new();
                    for child in children {
                        expanded.push(self.expand_rule(child, object_type, object_id, ctx).await?);
                    }
                    Ok(ExpandTree::Intersection { children: expanded })
                }
                RewriteRule::Exclusion(base, excluded) => {
                    let base_tree = self.expand_rule(base, object_type, object_id, ctx).await?;
                    let excluded_tree = self
                        .expand_rule(excluded, object_type, object_id, ctx)
                        .await?;
                    Ok(ExpandTree::Exclusion {
                        base: Box::new(base_tree),
                        excluded: Box::new(excluded_tree),
                    })
                }
                RewriteRule::Arrow(tupleset_rel, computed_perm) => {
                    self.expand_arrow(tupleset_rel, computed_perm, object_type, object_id, ctx)
                        .await
                }
            }
        })
    }

    async fn expand_this(
        &self,
        name: &str,
        object_type: &str,
        object_id: &str,
        ctx: &ExpandContext,
    ) -> Result<ExpandTree, CheckError> {
        let type_def = self
            .schema
            .get_type(object_type)
            .ok_or_else(|| CheckError::TypeNotFound(object_type.to_string()))?;

        if let Some(perm_def) = type_def.get_permission(name) {
            let child_ctx = ExpandContext {
                snapshot: ctx.snapshot,
                depth: ctx.depth + 1,
                visited: ctx.visited.clone(),
            };
            return self
                .expand_rule(&perm_def.rule, object_type, object_id, &child_ctx)
                .await;
        }

        if type_def.get_relation(name).is_some() {
            let filter = TupleFilter {
                object_type: Some(object_type.to_string()),
                object_id: Some(object_id.to_string()),
                relation: Some(name.to_string()),
                ..Default::default()
            };

            let tuples = self.reader.read_tuples(&filter, ctx.snapshot).await?;

            let mut subjects = Vec::new();
            for tuple in &tuples {
                if let Some(ref rel) = tuple.subject.subject_relation {
                    let userset_filter = TupleFilter {
                        object_type: Some(tuple.subject.subject_type.clone()),
                        object_id: Some(tuple.subject.subject_id.clone()),
                        relation: Some(rel.clone()),
                        ..Default::default()
                    };
                    let userset_tuples = self
                        .reader
                        .read_tuples(&userset_filter, ctx.snapshot)
                        .await?;
                    for ut in &userset_tuples {
                        subjects.push(ExpandTree::Leaf {
                            subject: ut.subject.clone(),
                        });
                    }
                } else {
                    subjects.push(ExpandTree::Leaf {
                        subject: tuple.subject.clone(),
                    });
                }
            }

            return Ok(ExpandTree::This {
                relation: name.to_string(),
                subjects,
            });
        }

        Err(CheckError::RelationNotFound {
            type_name: object_type.to_string(),
            relation: name.to_string(),
        })
    }

    async fn expand_arrow(
        &self,
        tupleset_rel: &str,
        computed_perm: &str,
        object_type: &str,
        object_id: &str,
        ctx: &ExpandContext,
    ) -> Result<ExpandTree, CheckError> {
        let filter = TupleFilter {
            object_type: Some(object_type.to_string()),
            object_id: Some(object_id.to_string()),
            relation: Some(tupleset_rel.to_string()),
            ..Default::default()
        };

        let tuples = self.reader.read_tuples(&filter, ctx.snapshot).await?;

        let mut children = Vec::new();
        for tuple in &tuples {
            let target_type = &tuple.subject.subject_type;
            let target_id = &tuple.subject.subject_id;

            let visit_key = (
                target_type.clone(),
                target_id.clone(),
                computed_perm.to_string(),
            );
            if ctx.visited.contains(&visit_key) {
                continue;
            }

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

            let mut child_visited = ctx.visited.clone();
            child_visited.insert(visit_key);
            let child_ctx = ExpandContext {
                snapshot: ctx.snapshot,
                depth: ctx.depth + 1,
                visited: child_visited,
            };

            let child_tree = self
                .expand_rule(&perm_def.rule, target_type, target_id, &child_ctx)
                .await?;
            children.push(child_tree);
        }

        Ok(ExpandTree::Arrow {
            tupleset_relation: tupleset_rel.to_string(),
            computed_permission: computed_perm.to_string(),
            children,
        })
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

    fn make_expand_engine(schema: Schema, tuples: Vec<Tuple>) -> ExpandEngine<TestStore> {
        ExpandEngine::new(
            Arc::new(TestStore::new(tuples)),
            Arc::new(schema),
            EngineConfig::default(),
        )
    }

    #[tokio::test]
    async fn expand_rejects_unknown_type() {
        let schema = Schema { types: vec![] };
        let engine = make_expand_engine(schema, vec![]);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let err = engine.expand(&request).await.unwrap_err();
        assert!(matches!(err, CheckError::TypeNotFound(ref t) if t == "document"));
    }

    #[tokio::test]
    async fn expand_direct_relation_returns_subjects() {
        let schema = Schema {
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
        };
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "viewer",
                SubjectRef::direct("user", "alice"),
            ),
            Tuple::new(
                ObjectRef::new("document", "readme"),
                "viewer",
                SubjectRef::direct("user", "bob"),
            ),
        ];
        let engine = make_expand_engine(schema, tuples);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        match tree {
            ExpandTree::This {
                ref relation,
                ref subjects,
            } => {
                assert_eq!(relation, "viewer");
                assert_eq!(subjects.len(), 2);
            }
            _ => panic!("expected This node, got: {tree:?}"),
        }
    }

    #[tokio::test]
    async fn expand_union_returns_union_node() {
        let schema = Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![
                    RelationDef {
                        name: "owner".to_string(),
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
                        RewriteRule::This("viewer".to_string()),
                    ]),
                }],
            }],
        };
        let tuples = vec![Tuple::new(
            ObjectRef::new("document", "readme"),
            "owner",
            SubjectRef::direct("user", "alice"),
        )];
        let engine = make_expand_engine(schema, tuples);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        assert!(matches!(tree, ExpandTree::Union { ref children } if children.len() == 2));
    }

    #[tokio::test]
    async fn expand_intersection_returns_intersection_node() {
        let schema = Schema {
            types: vec![TypeDefinition {
                name: "document".to_string(),
                relations: vec![
                    RelationDef {
                        name: "viewer".to_string(),
                        subject_types: vec![],
                    },
                    RelationDef {
                        name: "allowed".to_string(),
                        subject_types: vec![],
                    },
                ],
                permissions: vec![PermissionDef {
                    name: "view".to_string(),
                    rule: RewriteRule::Intersection(vec![
                        RewriteRule::This("viewer".to_string()),
                        RewriteRule::This("allowed".to_string()),
                    ]),
                }],
            }],
        };
        let engine = make_expand_engine(schema, vec![]);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        assert!(matches!(tree, ExpandTree::Intersection { ref children } if children.len() == 2));
    }

    #[tokio::test]
    async fn expand_exclusion_returns_exclusion_node() {
        let schema = Schema {
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
        };
        let engine = make_expand_engine(schema, vec![]);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        assert!(matches!(tree, ExpandTree::Exclusion { .. }));
    }

    #[tokio::test]
    async fn expand_arrow_returns_nested_tree() {
        let schema = Schema {
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
        };
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
        let engine = make_expand_engine(schema, tuples);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        match tree {
            ExpandTree::Arrow {
                ref tupleset_relation,
                ref computed_permission,
                ref children,
            } => {
                assert_eq!(tupleset_relation, "parent");
                assert_eq!(computed_permission, "view");
                assert_eq!(children.len(), 1);
                match &children[0] {
                    ExpandTree::This {
                        relation, subjects, ..
                    } => {
                        assert_eq!(relation, "viewer");
                        assert_eq!(subjects.len(), 1);
                    }
                    other => panic!("expected This node in arrow child, got: {other:?}"),
                }
            }
            other => panic!("expected Arrow node, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn expand_userset_expands_group_members() {
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
            Tuple::new(
                ObjectRef::new("group", "eng"),
                "member",
                SubjectRef::direct("user", "bob"),
            ),
        ];
        let engine = make_expand_engine(schema, tuples);

        let request = ExpandRequest {
            object_type: "document".to_string(),
            object_id: "readme".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let tree = engine.expand(&request).await.unwrap();
        match tree {
            ExpandTree::This {
                ref relation,
                ref subjects,
            } => {
                assert_eq!(relation, "viewer");
                assert_eq!(subjects.len(), 2);
            }
            other => panic!("expected This node, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn expand_handles_depth_limit() {
        let schema = Schema {
            types: vec![TypeDefinition {
                name: "folder".to_string(),
                relations: vec![
                    RelationDef {
                        name: "parent".to_string(),
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
                        RewriteRule::This("viewer".to_string()),
                        RewriteRule::Arrow("parent".to_string(), "view".to_string()),
                    ]),
                }],
            }],
        };
        let tuples = vec![
            Tuple::new(
                ObjectRef::new("folder", "f1"),
                "parent",
                SubjectRef::direct("folder", "f2"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f2"),
                "parent",
                SubjectRef::direct("folder", "f3"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f3"),
                "parent",
                SubjectRef::direct("folder", "f4"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f4"),
                "parent",
                SubjectRef::direct("folder", "f5"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f5"),
                "parent",
                SubjectRef::direct("folder", "f6"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f6"),
                "parent",
                SubjectRef::direct("folder", "f7"),
            ),
            Tuple::new(
                ObjectRef::new("folder", "f7"),
                "parent",
                SubjectRef::direct("folder", "f8"),
            ),
        ];
        let engine = make_expand_engine(schema, tuples);

        let request = ExpandRequest {
            object_type: "folder".to_string(),
            object_id: "f1".to_string(),
            permission: "view".to_string(),
            snapshot: None,
        };

        let err = engine.expand(&request).await.unwrap_err();
        assert!(matches!(err, CheckError::MaxDepthExceeded(_)));
    }
}
