//! Comprehensive benchmarks for local database operations at scale.
//!
//! Tests write, read, update, and delete performance across scaled database
//! sizes to reveal how operations behave as the database grows. Covers both
//! DocStore and Table store types, mixed workloads, transaction overhead,
//! store viewers, and multi-database scenarios.
//!
//! All benchmarks are local-only (no sync). Use TEST_BACKEND env var to select
//! the storage backend (default: sqlite, also: inmemory).

mod helpers;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use eidetica::{Database, crdt::Doc, store::DocStore, store::Table};
use helpers::setup_tree_async;
use serde::{Deserialize, Serialize};
use std::hint::black_box;
use tokio::runtime::Runtime;

// ---------------------------------------------------------------------------
// Shared types and helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    id: usize,
    name: String,
    data: String,
    value: i64,
}

fn make_record(i: usize) -> Record {
    Record {
        id: i,
        name: format!("record_{i}"),
        data: format!("payload data for record {i} with some extra content"),
        value: i as i64 * 7,
    }
}

fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to build Tokio runtime")
}

/// Populate a DocStore with `count` key-value pairs, one transaction per entry.
#[allow(dead_code)]
async fn populate_docstore(db: &Database, count: usize) {
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
async fn populate_docstore_batched(db: &Database, count: usize, batch_size: usize) {
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
async fn populate_table(db: &Database, count: usize) -> Vec<String> {
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
async fn populate_table_batched(db: &Database, count: usize, batch_size: usize) -> Vec<String> {
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

// ---------------------------------------------------------------------------
// 1. Write Scaling — DocStore
// ---------------------------------------------------------------------------

/// Single-entry write into databases of varying sizes.
/// Measures how insert cost scales with existing data.
fn bench_docstore_write_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_write_scaling");
    group.sample_size(20);

    for &db_size in &[0, 100, 500, 1000, 2000] {
        group.bench_with_input(
            BenchmarkId::new("single_write", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            populate_docstore_batched(&db, size, 50).await;
                            (inst, db)
                        })
                    },
                    |(inst, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let store = txn.get_store::<DocStore>("data").await.expect("store");
                            store
                                .set(
                                    black_box("new_key"),
                                    black_box("new_value_with_some_content"),
                                )
                                .await
                                .expect("set");
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Batch writes — insert N entries in a single transaction, into an empty db.
/// Throughput measured per entry.
fn bench_docstore_batch_write_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_batch_write");
    group.sample_size(15);

    for &batch in &[10, 50, 100, 500, 1000] {
        group.throughput(Throughput::Elements(batch as u64));
        group.bench_with_input(
            BenchmarkId::new("batch", batch),
            &batch,
            |b, &batch_size| {
                b.iter_with_setup(
                    || rt.block_on(setup_tree_async()),
                    |(inst, _user, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let store = txn.get_store::<DocStore>("data").await.expect("store");
                            for i in 0..batch_size {
                                store
                                    .set(black_box(format!("k_{i}")), black_box(format!("v_{i}")))
                                    .await
                                    .expect("set");
                            }
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. Write Scaling — Table
// ---------------------------------------------------------------------------

/// Single-record insert into Tables of varying sizes.
fn bench_table_write_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_write_scaling");
    group.sample_size(20);

    for &db_size in &[0, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("single_insert", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            let _ = populate_table_batched(&db, size, 50).await;
                            (inst, db)
                        })
                    },
                    |(inst, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let table = txn
                                .get_store::<Table<Record>>("records")
                                .await
                                .expect("store");
                            let _ =
                                black_box(table.insert(make_record(9999)).await.expect("insert"));
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Batch insert into Table — many records in a single transaction.
fn bench_table_batch_write_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_batch_write");
    group.sample_size(15);

    for &batch in &[10, 50, 100, 500] {
        group.throughput(Throughput::Elements(batch as u64));
        group.bench_with_input(
            BenchmarkId::new("batch", batch),
            &batch,
            |b, &batch_size| {
                b.iter_with_setup(
                    || rt.block_on(setup_tree_async()),
                    |(inst, _user, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let table = txn
                                .get_store::<Table<Record>>("records")
                                .await
                                .expect("store");
                            for i in 0..batch_size {
                                let _ = table.insert(make_record(i)).await.expect("insert");
                            }
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. Read Scaling — DocStore
// ---------------------------------------------------------------------------

/// Single-key read from databases of varying sizes.
fn bench_docstore_read_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_read_scaling");
    group.sample_size(30);

    for &db_size in &[10, 100, 500, 1000, 2000] {
        group.bench_with_input(
            BenchmarkId::new("single_read", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;
                let target_key = format!("key_{}", size / 2);

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let store = txn.get_store::<DocStore>("data").await.expect("store");
                        let _ = black_box(store.get(black_box(&target_key)).await.expect("get"));
                    });
                });
            },
        );
    }

    group.finish();
}

/// Read all keys from databases of varying sizes (get_all).
fn bench_docstore_get_all_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_get_all");
    group.sample_size(15);

    for &db_size in &[10, 100, 500, 1000] {
        group.throughput(Throughput::Elements(db_size as u64));
        group.bench_with_input(
            BenchmarkId::new("get_all", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let store = txn.get_store::<DocStore>("data").await.expect("store");
                        let _ = black_box(store.get_all().await.expect("get_all"));
                    });
                });
            },
        );
    }

    group.finish();
}

/// contains_key performance at varying sizes.
fn bench_docstore_contains_key(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_contains_key");
    group.sample_size(30);

    for &db_size in &[100, 500, 1000] {
        group.bench_with_input(BenchmarkId::new("hit", db_size), &db_size, |b, &size| {
            let (inst, _user, db) = rt.block_on(async {
                let (inst, user, db) = setup_tree_async().await;
                populate_docstore_batched(&db, size, 50).await;
                (inst, user, db)
            });
            let _ = &inst;
            let target_key = format!("key_{}", size / 2);

            b.iter(|| {
                rt.block_on(async {
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    let _ = black_box(store.contains_key(black_box(&target_key)).await);
                });
            });
        });

        group.bench_with_input(BenchmarkId::new("miss", db_size), &db_size, |b, &size| {
            let (inst, _user, db) = rt.block_on(async {
                let (inst, user, db) = setup_tree_async().await;
                populate_docstore_batched(&db, size, 50).await;
                (inst, user, db)
            });
            let _ = &inst;

            b.iter(|| {
                rt.block_on(async {
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    let _ = black_box(store.contains_key(black_box("nonexistent_key")).await);
                });
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 4. Read Scaling — Table
// ---------------------------------------------------------------------------

/// Single-record read from Tables of varying sizes.
fn bench_table_read_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_read_scaling");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("single_get", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db, keys) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    let keys = populate_table_batched(&db, size, 50).await;
                    (inst, user, db, keys)
                });
                let _ = &inst;
                let target_key = &keys[size / 2];

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let table = txn
                            .get_store::<Table<Record>>("records")
                            .await
                            .expect("store");
                        let _ = black_box(table.get(black_box(target_key)).await.expect("get"));
                    });
                });
            },
        );
    }

    group.finish();
}

/// Table search (full scan with predicate) at varying sizes.
fn bench_table_search_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_search");
    group.sample_size(15);

    for &db_size in &[10, 100, 500, 1000] {
        group.throughput(Throughput::Elements(db_size as u64));
        group.bench_with_input(
            BenchmarkId::new("search_all", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    let _ = populate_table_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let table = txn
                            .get_store::<Table<Record>>("records")
                            .await
                            .expect("store");
                        let _ = black_box(
                            table
                                .search(|r: &Record| r.value > 50)
                                .await
                                .expect("search"),
                        );
                    });
                });
            },
        );

        // Selective search — matches ~10% of records
        group.bench_with_input(
            BenchmarkId::new("search_selective", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    let _ = populate_table_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;
                let threshold = (size as i64) * 7 * 9 / 10; // top 10%

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let table = txn
                            .get_store::<Table<Record>>("records")
                            .await
                            .expect("store");
                        let _ = black_box(
                            table
                                .search(|r: &Record| r.value > threshold)
                                .await
                                .expect("search"),
                        );
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 5. Update Scaling
// ---------------------------------------------------------------------------

/// Overwriting an existing key in DocStore at varying database sizes.
fn bench_docstore_update_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_update_scaling");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("overwrite", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            populate_docstore_batched(&db, size, 50).await;
                            (inst, db)
                        })
                    },
                    |(inst, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let store = txn.get_store::<DocStore>("data").await.expect("store");
                            // Overwrite existing key in the middle
                            store
                                .set(
                                    black_box(format!("key_{}", size / 2)),
                                    black_box("updated_value_content"),
                                )
                                .await
                                .expect("set");
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Update an existing record in Table via set().
fn bench_table_update_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_update_scaling");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("set_existing", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            let keys = populate_table_batched(&db, size, 50).await;
                            let target_key = keys[size / 2].clone();
                            (inst, db, target_key)
                        })
                    },
                    |(inst, db, target_key)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let table = txn
                                .get_store::<Table<Record>>("records")
                                .await
                                .expect("store");
                            table
                                .set(
                                    black_box(&target_key),
                                    black_box(Record {
                                        id: 9999,
                                        name: "updated".to_string(),
                                        data: "updated payload data".to_string(),
                                        value: 42,
                                    }),
                                )
                                .await
                                .expect("set");
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 6. Delete Scaling
// ---------------------------------------------------------------------------

/// Deleting a key from DocStore at varying sizes.
fn bench_docstore_delete_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("docstore_delete_scaling");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("single_delete", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            populate_docstore_batched(&db, size, 50).await;
                            (inst, db)
                        })
                    },
                    |(inst, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let store = txn.get_store::<DocStore>("data").await.expect("store");
                            let _ = black_box(
                                store
                                    .delete(black_box(format!("key_{}", size / 2)))
                                    .await
                                    .expect("delete"),
                            );
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Deleting a record from Table at varying sizes.
fn bench_table_delete_scaling(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("table_delete_scaling");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("single_delete", db_size),
            &db_size,
            |b, &size| {
                b.iter_with_setup(
                    || {
                        rt.block_on(async {
                            let (inst, _user, db) = setup_tree_async().await;
                            let keys = populate_table_batched(&db, size, 50).await;
                            let target_key = keys[size / 2].clone();
                            (inst, db, target_key)
                        })
                    },
                    |(inst, db, target_key)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            let table = txn
                                .get_store::<Table<Record>>("records")
                                .await
                                .expect("store");
                            let _ = black_box(
                                table.delete(black_box(&target_key)).await.expect("delete"),
                            );
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 7. Transaction Overhead
// ---------------------------------------------------------------------------

/// Compares many small transactions vs few large transactions for equivalent
/// total work. Shows the per-transaction overhead.
fn bench_transaction_granularity(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("transaction_granularity");
    group.sample_size(10);

    let total_entries = 100;

    // N entries across K transactions
    for &txn_count in &[1, 10, 50, 100] {
        let entries_per_txn = total_entries / txn_count;
        group.throughput(Throughput::Elements(total_entries as u64));
        group.bench_with_input(
            BenchmarkId::new("txns", txn_count),
            &(txn_count, entries_per_txn),
            |b, &(txn_count, entries_per_txn)| {
                b.iter_with_setup(
                    || rt.block_on(setup_tree_async()),
                    |(inst, _user, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            for t in 0..txn_count {
                                let txn = db.new_transaction().await.expect("txn");
                                let store = txn.get_store::<DocStore>("data").await.expect("store");
                                for e in 0..entries_per_txn {
                                    let idx = t * entries_per_txn + e;
                                    store
                                        .set(format!("k_{idx}"), format!("v_{idx}"))
                                        .await
                                        .expect("set");
                                }
                                txn.commit().await.expect("commit");
                            }
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Multiple stores accessed within a single transaction.
fn bench_multi_store_transaction(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("multi_store_transaction");
    group.sample_size(20);

    for &store_count in &[1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("stores", store_count),
            &store_count,
            |b, &num_stores| {
                b.iter_with_setup(
                    || rt.block_on(setup_tree_async()),
                    |(inst, _user, db)| {
                        let _ = &inst;
                        rt.block_on(async {
                            let txn = db.new_transaction().await.expect("txn");
                            for s in 0..num_stores {
                                let store = txn
                                    .get_store::<DocStore>(&format!("store_{s}"))
                                    .await
                                    .expect("store");
                                store
                                    .set("key", format!("value_from_store_{s}"))
                                    .await
                                    .expect("set");
                            }
                            txn.commit().await.expect("commit");
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 8. Store Viewer (read-only access)
// ---------------------------------------------------------------------------

/// Store viewer creation + read at varying sizes.
fn bench_store_viewer(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("store_viewer");
    group.sample_size(20);

    for &db_size in &[10, 100, 500, 1000] {
        group.bench_with_input(
            BenchmarkId::new("docstore_viewer_read", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;
                let target_key = format!("key_{}", size / 2);

                b.iter(|| {
                    rt.block_on(async {
                        let viewer = db
                            .get_store_viewer::<DocStore>("data")
                            .await
                            .expect("viewer");
                        let _ = black_box(viewer.get(black_box(&target_key)).await.expect("get"));
                    });
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("table_viewer_read", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db, keys) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    let keys = populate_table_batched(&db, size, 50).await;
                    (inst, user, db, keys)
                });
                let _ = &inst;
                let target_key = &keys[size / 2];

                b.iter(|| {
                    rt.block_on(async {
                        let viewer = db
                            .get_store_viewer::<Table<Record>>("records")
                            .await
                            .expect("viewer");
                        let _ = black_box(viewer.get(black_box(target_key)).await.expect("get"));
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 9. Database Lifecycle
// ---------------------------------------------------------------------------

/// Database creation overhead (instance + user + db).
fn bench_database_creation(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("database_lifecycle");
    group.sample_size(20);

    group.bench_function("create_single_db", |b| {
        b.iter(|| {
            rt.block_on(async {
                let (inst, _user, db) = setup_tree_async().await;
                let _ = &inst;
                black_box(&db);
            });
        });
    });

    // Multiple databases on the same instance
    for &db_count in &[2, 5, 10] {
        group.bench_with_input(
            BenchmarkId::new("create_n_dbs", db_count),
            &db_count,
            |b, &count| {
                b.iter(|| {
                    rt.block_on(async {
                        let (inst, mut user, db) = setup_tree_async().await;
                        let _ = &inst;
                        let mut dbs = vec![db];
                        let key_id = user.get_default_key().expect("key");
                        for _ in 1..count {
                            let db = user
                                .create_database(Doc::new(), &key_id)
                                .await
                                .expect("create db");
                            dbs.push(db);
                        }
                        black_box(dbs);
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 10. Mixed Workloads
// ---------------------------------------------------------------------------

/// Interleaved reads and writes simulating realistic usage patterns.
fn bench_mixed_workload(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("mixed_workload");
    group.sample_size(10);

    let db_size = 500;

    // Read-heavy: 90% reads, 10% writes (10 ops total per iteration)
    group.bench_function("read_heavy_500", |b| {
        let (inst, _user, db) = rt.block_on(async {
            let (inst, user, db) = setup_tree_async().await;
            populate_docstore_batched(&db, db_size, 50).await;
            (inst, user, db)
        });
        let _ = &inst;
        let mut write_counter = 0usize;

        b.iter(|| {
            rt.block_on(async {
                // 9 reads
                for r in 0..9 {
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    let key = format!("key_{}", (r * 50) % db_size);
                    let _ = black_box(store.get(&key).await.expect("get"));
                }
                // 1 write
                let txn = db.new_transaction().await.expect("txn");
                let store = txn.get_store::<DocStore>("data").await.expect("store");
                store
                    .set(format!("rh_{write_counter}"), "mixed_value")
                    .await
                    .expect("set");
                txn.commit().await.expect("commit");
                write_counter += 1;
            });
        });
    });

    // Write-heavy: 10% reads, 90% writes (10 ops total per iteration)
    group.bench_function("write_heavy_500", |b| {
        let (inst, _user, db) = rt.block_on(async {
            let (inst, user, db) = setup_tree_async().await;
            populate_docstore_batched(&db, db_size, 50).await;
            (inst, user, db)
        });
        let _ = &inst;
        let mut write_counter = 0usize;

        b.iter(|| {
            rt.block_on(async {
                // 1 read
                let txn = db.new_transaction().await.expect("txn");
                let store = txn.get_store::<DocStore>("data").await.expect("store");
                let _ = black_box(store.get("key_100").await.expect("get"));

                // 9 writes (each in own transaction for realistic overhead)
                for _ in 0..9 {
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    store
                        .set(format!("wh_{write_counter}"), "mixed_value")
                        .await
                        .expect("set");
                    txn.commit().await.expect("commit");
                    write_counter += 1;
                }
            });
        });
    });

    // Balanced: 50% reads, 50% writes (10 ops total per iteration)
    group.bench_function("balanced_500", |b| {
        let (inst, _user, db) = rt.block_on(async {
            let (inst, user, db) = setup_tree_async().await;
            populate_docstore_batched(&db, db_size, 50).await;
            (inst, user, db)
        });
        let _ = &inst;
        let mut write_counter = 0usize;

        b.iter(|| {
            rt.block_on(async {
                // Interleave: read, write, read, write, ...
                for r in 0..5 {
                    // read
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    let key = format!("key_{}", (r * 100) % db_size);
                    let _ = black_box(store.get(&key).await.expect("get"));

                    // write
                    let txn = db.new_transaction().await.expect("txn");
                    let store = txn.get_store::<DocStore>("data").await.expect("store");
                    store
                        .set(format!("bal_{write_counter}"), "mixed_value")
                        .await
                        .expect("set");
                    txn.commit().await.expect("commit");
                    write_counter += 1;
                }
            });
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 11. Incremental Growth — continuous writes into a growing database
// ---------------------------------------------------------------------------

/// Measures amortized write cost as the database continuously grows.
/// Unlike write_scaling which uses fresh dbs, this reuses one growing db.
fn bench_incremental_growth(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("incremental_growth");
    group.sample_size(30);

    for &initial_size in &[0, 100, 500] {
        group.bench_with_input(
            BenchmarkId::new("docstore", initial_size),
            &initial_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;
                let mut counter = size;

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let store = txn.get_store::<DocStore>("data").await.expect("store");
                        store
                            .set(format!("inc_{counter}"), format!("val_{counter}"))
                            .await
                            .expect("set");
                        txn.commit().await.expect("commit");
                        counter += 1;
                    });
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("table", initial_size),
            &initial_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    let _ = populate_table_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;
                let mut counter = size;

                b.iter(|| {
                    rt.block_on(async {
                        let txn = db.new_transaction().await.expect("txn");
                        let table = txn
                            .get_store::<Table<Record>>("records")
                            .await
                            .expect("store");
                        let _ = table.insert(make_record(counter)).await.expect("insert");
                        txn.commit().await.expect("commit");
                        counter += 1;
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 12. Entry History Access
// ---------------------------------------------------------------------------

/// Measures get_all_entries and get_tips at scale.
fn bench_entry_history(c: &mut Criterion) {
    let rt = rt();
    let mut group = c.benchmark_group("entry_history");
    group.sample_size(15);

    for &db_size in &[100, 500, 1000] {
        group.throughput(Throughput::Elements(db_size as u64));

        group.bench_with_input(
            BenchmarkId::new("get_all_entries", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;

                b.iter(|| {
                    rt.block_on(async {
                        let entries =
                            black_box(db.get_all_entries().await.expect("get_all_entries"));
                        black_box(entries.len());
                    });
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("get_tips", db_size),
            &db_size,
            |b, &size| {
                let (inst, _user, db) = rt.block_on(async {
                    let (inst, user, db) = setup_tree_async().await;
                    populate_docstore_batched(&db, size, 50).await;
                    (inst, user, db)
                });
                let _ = &inst;

                b.iter(|| {
                    rt.block_on(async {
                        let tips = black_box(db.get_tips().await.expect("get_tips"));
                        black_box(tips.len());
                    });
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion setup
// ---------------------------------------------------------------------------

criterion_group! {
    name = write_benches;
    config = Criterion::default().configure_from_args();
    targets =
        bench_docstore_write_scaling,
        bench_docstore_batch_write_scaling,
        bench_table_write_scaling,
        bench_table_batch_write_scaling,
}

criterion_group! {
    name = read_benches;
    config = Criterion::default().configure_from_args();
    targets =
        bench_docstore_read_scaling,
        bench_docstore_get_all_scaling,
        bench_docstore_contains_key,
        bench_table_read_scaling,
        bench_table_search_scaling,
}

criterion_group! {
    name = mutate_benches;
    config = Criterion::default().configure_from_args();
    targets =
        bench_docstore_update_scaling,
        bench_table_update_scaling,
        bench_docstore_delete_scaling,
        bench_table_delete_scaling,
}

criterion_group! {
    name = operational_benches;
    config = Criterion::default().configure_from_args();
    targets =
        bench_transaction_granularity,
        bench_multi_store_transaction,
        bench_store_viewer,
        bench_database_creation,
        bench_mixed_workload,
        bench_incremental_growth,
        bench_entry_history,
}

criterion_main!(
    write_benches,
    read_benches,
    mutate_benches,
    operational_benches
);
