use kraalzibar_core::tuple::TupleFilter;

use crate::traits::{RelationshipStore, SchemaStore, StorageError};

pub async fn run_gc_cycle<S: RelationshipStore + SchemaStore>(
    store: &S,
    schema: &kraalzibar_core::schema::types::Schema,
) -> Result<usize, StorageError> {
    let all_tuples = store.read(&TupleFilter::default(), None).await?;

    let mut orphan_filters = Vec::new();

    for tuple in &all_tuples {
        let type_def = schema.get_type(&tuple.object.object_type);
        let is_orphan = match type_def {
            None => true,
            Some(td) => !td.relations.iter().any(|r| r.name == tuple.relation),
        };

        if is_orphan {
            orphan_filters.push(TupleFilter {
                object_type: Some(tuple.object.object_type.clone()),
                object_id: Some(tuple.object.object_id.clone()),
                relation: Some(tuple.relation.clone()),
                subject_type: Some(tuple.subject.subject_type.clone()),
                subject_id: Some(tuple.subject.subject_id.clone()),
                subject_relation: Some(tuple.subject.subject_relation.clone()),
            });
        }
    }

    let count = orphan_filters.len();
    if !orphan_filters.is_empty() {
        store.write(&[], &orphan_filters).await?;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryStore;
    use crate::traits::RelationshipStore;
    use kraalzibar_core::schema::parse_schema;
    use kraalzibar_core::tuple::{ObjectRef, SubjectRef, TupleWrite};

    fn make_write(obj_type: &str, obj_id: &str, relation: &str, subj: SubjectRef) -> TupleWrite {
        TupleWrite::new(ObjectRef::new(obj_type, obj_id), relation, subj)
    }

    #[tokio::test]
    async fn gc_removes_orphaned_tuples_from_removed_type() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", SubjectRef::direct("user", "a")),
                    make_write("old_type", "1", "rel", SubjectRef::direct("user", "b")),
                ],
                &[],
            )
            .await
            .unwrap();

        let schema =
            parse_schema("definition user {} definition doc { relation viewer: user }").unwrap();

        let removed = run_gc_cycle(&store, &schema).await.unwrap();

        assert_eq!(removed, 1);

        let remaining = store.read(&TupleFilter::default(), None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].object.object_type, "doc");
    }

    #[tokio::test]
    async fn gc_removes_orphaned_tuples_from_removed_relation() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", SubjectRef::direct("user", "a")),
                    make_write("doc", "2", "old_rel", SubjectRef::direct("user", "b")),
                ],
                &[],
            )
            .await
            .unwrap();

        let schema =
            parse_schema("definition user {} definition doc { relation viewer: user }").unwrap();

        let removed = run_gc_cycle(&store, &schema).await.unwrap();

        assert_eq!(removed, 1);

        let remaining = store.read(&TupleFilter::default(), None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].relation, "viewer");
    }

    #[tokio::test]
    async fn gc_leaves_valid_tuples_untouched() {
        let store = InMemoryStore::new();
        store
            .write(
                &[
                    make_write("doc", "1", "viewer", SubjectRef::direct("user", "a")),
                    make_write("doc", "2", "editor", SubjectRef::direct("user", "b")),
                ],
                &[],
            )
            .await
            .unwrap();

        let schema = parse_schema(
            "definition user {} definition doc { relation viewer: user relation editor: user }",
        )
        .unwrap();

        let removed = run_gc_cycle(&store, &schema).await.unwrap();

        assert_eq!(removed, 0);

        let remaining = store.read(&TupleFilter::default(), None).await.unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[tokio::test]
    async fn gc_on_empty_store_returns_zero() {
        let store = InMemoryStore::new();
        let schema = parse_schema("definition user {}").unwrap();

        let removed = run_gc_cycle(&store, &schema).await.unwrap();

        assert_eq!(removed, 0);
    }
}
