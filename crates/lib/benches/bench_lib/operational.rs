//! Transaction granularity, multi-store, viewer, db lifecycle, mixed workload,
//! incremental growth, and entry history benchmarks.

use criterion::{BenchmarkId, Criterion, Throughput};
use eidetica::{
    crdt::Doc,
    store::{DocStore, Table},
};
use std::hint::black_box;

use super::{Record, make_record, populate_docstore_batched, populate_table_batched, rt};
use crate::helpers::setup_tree_async;

/// Compares many small transactions vs few large transactions for equivalent
/// total work. Shows the per-transaction overhead.
pub fn bench_transaction_granularity(
    c: &mut Criterion,
    group_name: &str,
    total_entries: usize,
    txn_counts: &[usize],
) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(10);

    for &txn_count in txn_counts {
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
pub fn bench_multi_store_transaction(c: &mut Criterion, group_name: &str, store_counts: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &store_count in store_counts {
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

/// Store viewer creation + read at varying sizes.
pub fn bench_store_viewer(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(20);

    for &db_size in sizes {
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

/// Database creation overhead (instance + user + db).
pub fn bench_database_creation(c: &mut Criterion, group_name: &str, db_counts: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
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
    for &db_count in db_counts {
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

/// Interleaved reads and writes simulating realistic usage patterns.
pub fn bench_mixed_workload(c: &mut Criterion, group_name: &str, db_size: usize) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(10);

    // Read-heavy: 90% reads, 10% writes (10 ops total per iteration)
    group.bench_function("read_heavy", |b| {
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
    group.bench_function("write_heavy", |b| {
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
    group.bench_function("balanced", |b| {
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

/// Measures amortized write cost as the database continuously grows.
/// Unlike write_scaling which uses fresh dbs, this reuses one growing db.
pub fn bench_incremental_growth(c: &mut Criterion, group_name: &str, initial_sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(30);

    for &initial_size in initial_sizes {
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

/// Measures get_all_entries and get_tips at scale.
pub fn bench_entry_history(c: &mut Criterion, group_name: &str, sizes: &[usize]) {
    let rt = rt();
    let mut group = c.benchmark_group(group_name);
    group.sample_size(15);

    for &db_size in sizes {
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
