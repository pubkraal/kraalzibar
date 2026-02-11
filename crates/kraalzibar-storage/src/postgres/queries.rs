use kraalzibar_core::tuple::{ObjectRef, SubjectRef, Tuple, TupleFilter, TupleWrite};

use crate::traits::StorageError;

const ACTIVE_TX_ID: i64 = i64::MAX;

fn to_storage_error(e: sqlx::Error) -> StorageError {
    StorageError::Internal(e.to_string())
}

pub async fn next_tx_id<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
) -> Result<i64, StorageError> {
    let query = format!("SELECT nextval('{schema}.tx_id_seq')");
    let row: (i64,) = sqlx::query_as(&query)
        .fetch_one(executor)
        .await
        .map_err(to_storage_error)?;

    if row.0 == ACTIVE_TX_ID {
        return Err(StorageError::Internal(
            "transaction ID sequence exhausted (reached sentinel value)".to_string(),
        ));
    }

    Ok(row.0)
}

pub async fn current_tx_id<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
) -> Result<i64, StorageError> {
    let query = format!(
        "SELECT COALESCE((SELECT last_value FROM {schema}.tx_id_seq WHERE is_called = true), 0)"
    );
    let row: (i64,) = sqlx::query_as(&query)
        .fetch_one(executor)
        .await
        .map_err(to_storage_error)?;
    Ok(row.0)
}

pub async fn insert_tuple<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
    write: &TupleWrite,
    tx_id: i64,
) -> Result<(), StorageError> {
    let query = format!(
        r#"
        INSERT INTO {schema}.relation_tuples
            (object_type, object_id, relation, subject_type, subject_id, subject_relation, created_tx_id, deleted_tx_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#
    );
    sqlx::query(&query)
        .bind(&write.object.object_type)
        .bind(&write.object.object_id)
        .bind(&write.relation)
        .bind(&write.subject.subject_type)
        .bind(&write.subject.subject_id)
        .bind(&write.subject.subject_relation)
        .bind(tx_id)
        .bind(ACTIVE_TX_ID)
        .execute(executor)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.is_unique_violation()
            {
                return StorageError::DuplicateTuple;
            }
            to_storage_error(e)
        })?;
    Ok(())
}

pub async fn delete_matching_tuples<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
    filter: &TupleFilter,
    tx_id: i64,
) -> Result<(), StorageError> {
    // $1 = ACTIVE_TX_ID (WHERE condition), $2 = tx_id (SET value)
    let mut conditions = vec!["deleted_tx_id = $1".to_string()];
    let mut binds: Vec<Option<&str>> = Vec::new();
    let mut bind_idx = 3;

    if let Some(ref ot) = filter.object_type {
        conditions.push(format!("object_type = ${bind_idx}"));
        binds.push(Some(ot.as_str()));
        bind_idx += 1;
    }
    if let Some(ref oi) = filter.object_id {
        conditions.push(format!("object_id = ${bind_idx}"));
        binds.push(Some(oi.as_str()));
        bind_idx += 1;
    }
    if let Some(ref r) = filter.relation {
        conditions.push(format!("relation = ${bind_idx}"));
        binds.push(Some(r.as_str()));
        bind_idx += 1;
    }
    if let Some(ref st) = filter.subject_type {
        conditions.push(format!("subject_type = ${bind_idx}"));
        binds.push(Some(st.as_str()));
        bind_idx += 1;
    }
    if let Some(ref si) = filter.subject_id {
        conditions.push(format!("subject_id = ${bind_idx}"));
        binds.push(Some(si.as_str()));
        bind_idx += 1;
    }
    if let Some(ref sr) = filter.subject_relation {
        match sr {
            None => {
                conditions.push("subject_relation IS NULL".to_string());
            }
            Some(rel) => {
                conditions.push(format!("subject_relation = ${bind_idx}"));
                binds.push(Some(rel.as_str()));
                bind_idx += 1;
            }
        }
    }
    let _ = bind_idx;

    let where_clause = conditions.join(" AND ");
    let query =
        format!("UPDATE {schema}.relation_tuples SET deleted_tx_id = $2 WHERE {where_clause}");

    let mut q = sqlx::query(&query).bind(ACTIVE_TX_ID).bind(tx_id);
    for bind in &binds {
        q = q.bind(*bind);
    }
    q.execute(executor).await.map_err(to_storage_error)?;
    Ok(())
}

pub async fn read_tuples<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
    filter: &TupleFilter,
    snapshot: i64,
    limit: Option<usize>,
) -> Result<Vec<Tuple>, StorageError> {
    // $1 = snapshot (used in both created/deleted conditions)
    let mut conditions = vec![
        "created_tx_id <= $1".to_string(),
        "deleted_tx_id > $1".to_string(),
    ];
    let mut binds: Vec<Option<&str>> = Vec::new();
    let mut bind_idx = 2;

    if let Some(ref ot) = filter.object_type {
        conditions.push(format!("object_type = ${bind_idx}"));
        binds.push(Some(ot.as_str()));
        bind_idx += 1;
    }
    if let Some(ref oi) = filter.object_id {
        conditions.push(format!("object_id = ${bind_idx}"));
        binds.push(Some(oi.as_str()));
        bind_idx += 1;
    }
    if let Some(ref r) = filter.relation {
        conditions.push(format!("relation = ${bind_idx}"));
        binds.push(Some(r.as_str()));
        bind_idx += 1;
    }
    if let Some(ref st) = filter.subject_type {
        conditions.push(format!("subject_type = ${bind_idx}"));
        binds.push(Some(st.as_str()));
        bind_idx += 1;
    }
    if let Some(ref si) = filter.subject_id {
        conditions.push(format!("subject_id = ${bind_idx}"));
        binds.push(Some(si.as_str()));
        bind_idx += 1;
    }
    if let Some(ref sr) = filter.subject_relation {
        match sr {
            None => {
                conditions.push("subject_relation IS NULL".to_string());
            }
            Some(rel) => {
                conditions.push(format!("subject_relation = ${bind_idx}"));
                binds.push(Some(rel.as_str()));
                bind_idx += 1;
            }
        }
    }
    let _ = bind_idx;

    let where_clause = conditions.join(" AND ");
    let limit_clause = match limit {
        Some(n) => format!(" LIMIT {n}"),
        None => String::new(),
    };
    let query = format!(
        r#"SELECT object_type, object_id, relation, subject_type, subject_id, subject_relation
           FROM {schema}.relation_tuples
           WHERE {where_clause}{limit_clause}"#
    );

    let mut q =
        sqlx::query_as::<_, (String, String, String, String, String, Option<String>)>(&query)
            .bind(snapshot);
    for bind in &binds {
        q = q.bind(*bind);
    }

    let rows = q.fetch_all(executor).await.map_err(to_storage_error)?;

    let tuples = rows
        .into_iter()
        .map(
            |(object_type, object_id, relation, subject_type, subject_id, subject_relation)| {
                let subject = match subject_relation {
                    None => SubjectRef::direct(&subject_type, &subject_id),
                    Some(ref rel) => SubjectRef::userset(&subject_type, &subject_id, rel),
                };
                Tuple::new(ObjectRef::new(&object_type, &object_id), &relation, subject)
            },
        )
        .collect();

    Ok(tuples)
}

pub async fn list_distinct_object_ids<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
    object_type: &str,
    snapshot: i64,
    limit: Option<usize>,
) -> Result<Vec<String>, StorageError> {
    let limit_clause = match limit {
        Some(n) => format!(" LIMIT {n}"),
        None => String::new(),
    };
    let query = format!(
        r#"SELECT DISTINCT object_id
           FROM {schema}.relation_tuples
           WHERE object_type = $1 AND created_tx_id <= $2 AND deleted_tx_id > $2
           ORDER BY object_id{limit_clause}"#
    );

    let rows: Vec<(String,)> = sqlx::query_as(&query)
        .bind(object_type)
        .bind(snapshot)
        .fetch_all(executor)
        .await
        .map_err(to_storage_error)?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

pub async fn write_schema_definition<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
    definition: &str,
) -> Result<(), StorageError> {
    let query = format!(
        r#"
        INSERT INTO {schema}.schema_definitions (definition)
        VALUES ($1)
        "#
    );
    sqlx::query(&query)
        .bind(definition)
        .execute(executor)
        .await
        .map_err(to_storage_error)?;
    Ok(())
}

pub async fn read_latest_schema<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    schema: &str,
) -> Result<Option<String>, StorageError> {
    let query = format!(
        r#"
        SELECT definition FROM {schema}.schema_definitions
        ORDER BY created_at DESC
        LIMIT 1
        "#
    );
    let row: Option<(String,)> = sqlx::query_as(&query)
        .fetch_optional(executor)
        .await
        .map_err(to_storage_error)?;
    Ok(row.map(|(d,)| d))
}
