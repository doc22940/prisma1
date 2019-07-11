use crate::{
    error::SqlError,
    transaction_ext,
    write_query::{DeleteActions, NestedActions, WriteQueryBuilder},
};
use connector_interface::{error::RecordFinderInfo, filter::RecordFinder};
use prisma_models::{GraphqlId, RelationFieldRef, SingleRecord};
use prisma_query::connector::{Transaction, Queryable};
use std::sync::Arc;

/// A top level delete that removes one record. Violating any relations or a
/// non-existing record will cause an error.
///
/// Will return the deleted record if the delete was successful.
pub fn execute(conn: &mut Transaction, record_finder: &RecordFinder) -> crate::Result<SingleRecord> {
    let model = record_finder.field.model();
    let record = transaction_ext::find_record(conn, record_finder)?;
    let id = record.get_id_value(Arc::clone(&model)).unwrap();

    DeleteActions::check_relation_violations(Arc::clone(&model), &[&id], |select| {
        let ids = transaction_ext::select_ids(conn, select)?;
        Ok(ids.into_iter().next())
    })?;

    for delete in WriteQueryBuilder::delete_many(model, &[&id]) {
        conn.delete(delete)?;
    }

    Ok(record)
}

/// A nested delete that removes one item related to the given `parent_id`.
/// If no `RecordFinder` is given, will delete the first item from the
/// table.
///
/// Errors thrown from domain violations:
///
/// - Violating any relations where the deleted record is required
/// - If the deleted record is not connected to the parent
/// - The record does not exist
pub fn execute_nested(
    conn: &mut Transaction,
    parent_id: &GraphqlId,
    actions: &NestedActions,
    record_finder: &Option<RecordFinder>,
    relation_field: RelationFieldRef,
) -> crate::Result<()> {
    if let Some(ref record_finder) = record_finder {
        transaction_ext::find_id(conn, record_finder)?;
    };

    let find = transaction_ext::find_id_by_parent(
        conn,
        Arc::clone(&relation_field),
        parent_id,
        record_finder
    );

    let child_id = find.map_err(|e| match e {
        SqlError::RecordsNotConnected {
            relation_name,
            parent_name,
            parent_where: _,
            child_name,
            child_where,
        } => {
            let model = Arc::clone(&relation_field.model());

            SqlError::RecordsNotConnected {
                relation_name: relation_name,
                parent_name: parent_name,
                parent_where: Some(RecordFinderInfo::for_id(model, parent_id)),
                child_name: child_name,
                child_where: child_where,
            }
        }
        e => e,
    })?;

    {
        let (select, check) = actions.ensure_connected(parent_id, &child_id);
        let ids = transaction_ext::select_ids(conn, select)?;
        check(ids.into_iter().next().is_some())?;
    }

    let related_model = relation_field.related_model();

    DeleteActions::check_relation_violations(related_model, &[&child_id; 1], |select| {
        let ids = transaction_ext::select_ids(conn, select)?;
        Ok(ids.into_iter().next())
    })?;

    for delete in WriteQueryBuilder::delete_many(relation_field.related_model(), &[&child_id]) {
        conn.delete(delete)?;
    }

    Ok(())
}