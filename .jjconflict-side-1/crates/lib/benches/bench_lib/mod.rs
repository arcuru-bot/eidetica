//! Shared parameterized benchmark implementations.
//!
//! Contains benchmark logic as parameterized functions so that multiple
//! benchmark binaries can reuse the same implementations with different
//! parameter sets (e.g., default vs extended scales).

pub mod mutate;
pub mod operational;
pub mod read;
pub mod write;

use eidetica::{
    Database,
    store::{DocStore, Table},
};
use serde::{Deserialize, Serialize};
use tokio::runtime::Runtime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub id: usize,
    pub name: String,
    pub data: String,
    pub value: i64,
}

pub fn make_record(i: usize) -> Record {
    Record {
        id: i,
        name: format!("record_{i}"),
        data: format!("payload data for record {i} with some extra content"),
        value: i as i64 * 7,
    }
}

pub fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime")
}

/// Populate a DocStore with `count` key-value pairs, one transaction per entry.
#[allow(dead_code)]
pub async fn populate_docstore(db: &Database, count: usize) {
    for i in 0..count {
        let txn = db.new_transaction().await.expect("txn");
        let store = txn.get_store::<DocStore>("data").await.expect("store");
        store
            .set(format!("key_{i}"), format!("value_{i}"))
            .await
            .expect("set");
        txn.commit().await.expect("commit");
    }
}

/// Populate a DocStore with `count` entries using batched transactions.
/// Each transaction inserts `batch_size` entries.
pub async fn populate_docstore_batched(db: &Database, count: usize, batch_size: usize) {
    let mut i = 0;
    while i < count {
        let txn = db.new_transaction().await.expect("txn");
        let store = txn.get_store::<DocStore>("data").await.expect("store");
        let end = (i + batch_size).min(count);
        for j in i..end {
            store
                .set(format!("key_{j}"), format!("value_{j}"))
                .await
                .expect("set");
        }
        txn.commit().await.expect("commit");
        i = end;
    }
}

/// Populate a Table with `count` records, one transaction per record.
/// Returns the generated keys.
#[allow(dead_code)]
pub async fn populate_table(db: &Database, count: usize) -> Vec<String> {
    let mut keys = Vec::with_capacity(count);
    for i in 0..count {
        let txn = db.new_transaction().await.expect("txn");
        let table = txn
            .get_store::<Table<Record>>("records")
            .await
            .expect("store");
        let key = table.insert(make_record(i)).await.expect("insert");
        keys.push(key);
        txn.commit().await.expect("commit");
    }
    keys
}

/// Populate a Table with `count` records using batched transactions.
/// Returns the generated keys.
pub async fn populate_table_batched(db: &Database, count: usize, batch_size: usize) -> Vec<String> {
    let mut keys = Vec::with_capacity(count);
    let mut i = 0;
    while i < count {
        let txn = db.new_transaction().await.expect("txn");
        let table = txn
            .get_store::<Table<Record>>("records")
            .await
            .expect("store");
        let end = (i + batch_size).min(count);
        for j in i..end {
            let key = table.insert(make_record(j)).await.expect("insert");
            keys.push(key);
        }
        txn.commit().await.expect("commit");
        i = end;
    }
    keys
}
