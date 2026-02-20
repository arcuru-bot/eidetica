//! Read scaling benchmarks (single read, get_all, contains_key, table search).

use criterion::{BenchmarkId, Criterion, Throughput};
use eidetica::store::{DocStore, Table};
use std::hint::black_box;

use super::{Record, populate_docstore_batched, populate_table_batched, rt};
use crate::helpers::setup_tree_async;

/// Single-key read from databases of varying sizes.
pub fn bench_docstore_read_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(30);

    for &db_size in sizes {
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
pub fn bench_docstore_get_all_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(15);

    for &db_size in sizes {
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
pub fn bench_docstore_contains_key(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(30);

    for &db_size in sizes {
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

/// Single-record read from Tables of varying sizes.
pub fn bench_table_read_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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
pub fn bench_table_search_scaling(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(15);

    for &db_size in sizes {
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

        // Selective search -- matches ~10% of records
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
