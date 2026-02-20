//! Update and delete benchmarks.

use criterion::{BenchmarkId, Criterion};
use eidetica::store::{DocStore, Table};
use std::hint::black_box;

use super::{Record, populate_docstore_batched, populate_table_batched, rt};
use crate::helpers::setup_tree_async;

/// Overwriting an existing key in DocStore at varying database sizes.
pub fn bench_docstore_update_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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
pub fn bench_table_update_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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

/// Deleting a key from DocStore at varying sizes.
pub fn bench_docstore_delete_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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
pub fn bench_table_delete_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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
