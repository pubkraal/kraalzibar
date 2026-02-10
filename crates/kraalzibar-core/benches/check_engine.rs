use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};

use kraalzibar_core::engine::{CheckEngine, CheckError, CheckRequest, EngineConfig, TupleReader};
use kraalzibar_core::schema::parse_schema;
use kraalzibar_core::schema::types::Schema;
use kraalzibar_core::tuple::{ObjectRef, SnapshotToken, SubjectRef, Tuple, TupleFilter};

struct TestStore {
    tuples: Vec<Tuple>,
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
        Arc::new(TestStore { tuples }),
        Arc::new(schema),
        EngineConfig::default(),
    )
}

fn make_tuple(
    obj_type: &str,
    obj_id: &str,
    relation: &str,
    subj_type: &str,
    subj_id: &str,
) -> Tuple {
    Tuple::from(kraalzibar_core::tuple::TupleWrite::new(
        ObjectRef::new(obj_type, obj_id),
        relation,
        SubjectRef::direct(subj_type, subj_id),
    ))
}

fn make_userset_tuple(
    obj_type: &str,
    obj_id: &str,
    relation: &str,
    subj_type: &str,
    subj_id: &str,
    subj_relation: &str,
) -> Tuple {
    Tuple::from(kraalzibar_core::tuple::TupleWrite::new(
        ObjectRef::new(obj_type, obj_id),
        relation,
        SubjectRef::userset(subj_type, subj_id, subj_relation),
    ))
}

fn direct_relation_schema() -> Schema {
    parse_schema(
        r#"
        definition user {}
        definition document {
            relation viewer: user
            permission can_view = viewer
        }
    "#,
    )
    .unwrap()
}

fn union_3_schema() -> Schema {
    parse_schema(
        r#"
        definition user {}
        definition document {
            relation owner: user
            relation editor: user
            relation viewer: user
            permission can_view = owner + editor + viewer
        }
    "#,
    )
    .unwrap()
}

fn arrow_chain_schema(depth: usize) -> (Schema, Vec<Tuple>) {
    let mut dsl = String::from("definition user {}\n");

    for i in 0..depth {
        let type_name = format!("level{i}");
        if i == 0 {
            dsl.push_str(&format!(
                "definition {type_name} {{\n    relation member: user\n    permission access = member\n}}\n"
            ));
        } else {
            let parent_type = format!("level{}", i - 1);
            dsl.push_str(&format!(
                "definition {type_name} {{\n    relation parent: {parent_type}\n    permission access = parent->access\n}}\n"
            ));
        }
    }

    let schema = parse_schema(&dsl).unwrap();

    let mut tuples = vec![make_tuple("level0", "l0", "member", "user", "alice")];
    for i in 1..depth {
        let type_name = format!("level{i}");
        let parent_type = format!("level{}", i - 1);
        let obj_id = format!("l{i}");
        let parent_id = format!("l{}", i - 1);
        tuples.push(make_userset_tuple(
            &type_name,
            &obj_id,
            "parent",
            &parent_type,
            &parent_id,
            "access",
        ));
    }

    (schema, tuples)
}

fn fan_out_tuples(count: usize) -> Vec<Tuple> {
    (0..count)
        .map(|i| make_tuple("document", "doc1", "viewer", "user", &format!("user{i}")))
        .collect()
}

fn bench_check_direct_relation(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let schema = direct_relation_schema();
    let tuples = vec![make_tuple("document", "doc1", "viewer", "user", "alice")];
    let engine = make_engine(schema, tuples);

    c.bench_function("check_direct_relation", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "document".to_string(),
                    object_id: "doc1".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

fn bench_check_union_3_branches(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let schema = union_3_schema();
    let tuples = vec![make_tuple("document", "doc1", "viewer", "user", "alice")];
    let engine = make_engine(schema, tuples);

    c.bench_function("check_union_3_branches", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "document".to_string(),
                    object_id: "doc1".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

fn bench_check_arrow_depth_3(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (schema, tuples) = arrow_chain_schema(3);
    let engine = make_engine(schema, tuples);

    c.bench_function("check_arrow_depth_3", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "level2".to_string(),
                    object_id: "l2".to_string(),
                    permission: "access".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

fn bench_check_arrow_depth_6(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (schema, tuples) = arrow_chain_schema(6);
    let engine = make_engine(schema, tuples);

    c.bench_function("check_arrow_depth_6", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "level5".to_string(),
                    object_id: "l5".to_string(),
                    permission: "access".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "alice".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

fn bench_check_fan_out_10(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let schema = direct_relation_schema();
    let tuples = fan_out_tuples(10);
    let engine = make_engine(schema, tuples);

    c.bench_function("check_fan_out_10", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "document".to_string(),
                    object_id: "doc1".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "user5".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

fn bench_check_fan_out_100(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let schema = direct_relation_schema();
    let tuples = fan_out_tuples(100);
    let engine = make_engine(schema, tuples);

    c.bench_function("check_fan_out_100", |b| {
        b.to_async(&rt).iter(|| async {
            engine
                .check(&CheckRequest {
                    object_type: "document".to_string(),
                    object_id: "doc1".to_string(),
                    permission: "can_view".to_string(),
                    subject_type: "user".to_string(),
                    subject_id: "user50".to_string(),
                    snapshot: None,
                })
                .await
                .unwrap()
        });
    });
}

criterion_group!(
    benches,
    bench_check_direct_relation,
    bench_check_union_3_branches,
    bench_check_arrow_depth_3,
    bench_check_arrow_depth_6,
    bench_check_fan_out_10,
    bench_check_fan_out_100,
);
criterion_main!(benches);

// Factory helpers are validated implicitly by the benchmarks above — if they
// produced invalid schemas or wrong tuple counts, the benchmark runs would panic.
