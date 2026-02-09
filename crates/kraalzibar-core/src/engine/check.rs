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

        let allowed = self
            .evaluate_rule(
                &perm_def.rule,
                &request.object_type,
                &request.object_id,
                &request.subject_type,
                &request.subject_id,
                request.snapshot,
            )
            .await?;

        Ok(CheckResult { allowed })
    }

    async fn evaluate_rule(
        &self,
        rule: &RewriteRule,
        object_type: &str,
        object_id: &str,
        subject_type: &str,
        subject_id: &str,
        snapshot: Option<SnapshotToken>,
    ) -> Result<bool, CheckError> {
        match rule {
            RewriteRule::This(name) => {
                self.evaluate_this(
                    name,
                    object_type,
                    object_id,
                    subject_type,
                    subject_id,
                    snapshot,
                )
                .await
            }
            _ => Ok(false),
        }
    }

    async fn evaluate_this(
        &self,
        name: &str,
        object_type: &str,
        object_id: &str,
        subject_type: &str,
        subject_id: &str,
        snapshot: Option<SnapshotToken>,
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

            let tuples = self.reader.read_tuples(&filter, snapshot).await?;

            for tuple in &tuples {
                if tuple.subject.subject_relation.is_none()
                    && tuple.subject.subject_type == subject_type
                    && tuple.subject.subject_id == subject_id
                {
                    return Ok(true);
                }
            }

            return Ok(false);
        }

        Err(CheckError::RelationNotFound {
            type_name: object_type.to_string(),
            relation: name.to_string(),
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

    fn make_engine(schema: Schema, tuples: Vec<Tuple>) -> CheckEngine<TestStore> {
        CheckEngine::new(
            Arc::new(TestStore::new(tuples)),
            Arc::new(schema),
            EngineConfig::default(),
        )
    }

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
}
