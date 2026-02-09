use std::sync::Arc;

use crate::schema::types::Schema;
use crate::tuple::SnapshotToken;

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

#[allow(dead_code)]
pub struct CheckEngine<T: TupleReader> {
    reader: Arc<T>,
    schema: Arc<Schema>,
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

        let _perm_def = type_def
            .get_permission(&request.permission)
            .ok_or_else(|| CheckError::PermissionNotFound {
                type_name: request.object_type.clone(),
                permission: request.permission.clone(),
            })?;

        Ok(CheckResult { allowed: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TupleReader;
    use crate::schema::types::{PermissionDef, RewriteRule, TypeDefinition};
    use crate::tuple::{Tuple, TupleFilter};

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
}
