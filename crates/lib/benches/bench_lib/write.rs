//! Write scaling benchmarks (DocStore + Table, single + batch).

use criterion::{BenchmarkId, Criterion, Throughput};
use eidetica::store::{DocStore, Table};
use std::hint::black_box;

use super::{Record, make_record, populate_docstore_batched, populate_table_batched, rt};
use crate::helpers::setup_tree_async;

/// Single-entry write into databases of varying sizes.
/// Measures how insert cost scales with existing data.
pub fn bench_docstore_write_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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

/// Batch writes -- insert N entries in a single transaction, into an empty db.
/// Throughput measured per entry.
pub fn bench_docstore_batch_write_scaling(c: &mut Criterion, group_name: &str, batches: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(15);

    for &batch in batches {
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

/// Single-record insert into Tables of varying sizes.
pub fn bench_table_write_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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

/// Batch insert into Table -- many records in a single transaction.
pub fn bench_table_batch_write_scaling(c: &mut Criterion, group_name: &str, batches: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(15);

    for &batch in batches {
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
