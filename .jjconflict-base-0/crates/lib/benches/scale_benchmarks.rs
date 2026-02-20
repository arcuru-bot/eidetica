//! Comprehensive benchmarks for local database operations at scale (default parameters).
//!
//! Thin wrapper over `bench_lib` with default parameter sets suitable for
//! routine CI and development benchmarking. For larger-scale parameter sweeps,
//! see `extended_benchmarks`.
//!
//! All benchmarks are local-only (no sync). Use TEST_BACKEND env var to select
//! the storage backend (default: sqlite, also: inmemory).

mod bench_lib;
mod helpers;

use criterion::{Criterion, criterion_group, criterion_main};

// -- Write ------------------------------------------------------------------

fn docstore_write_scaling(c: &mut Criterion) {
    bench_lib::write::bench_docstore_write_scaling(
        c,
        "docstore_write_scaling",
        &[0, 100, 500, 1000],
    );
}

fn docstore_batch_write(c: &mut Criterion) {
    bench_lib::write::bench_docstore_batch_write_scaling(
        c,
        "docstore_batch_write",
        &[10, 50, 100, 200],
    );
}

fn table_write_scaling(c: &mut Criterion) {
    bench_lib::write::bench_table_write_scaling(c, "table_write_scaling", &[0, 100, 500, 1000]);
}

fn table_batch_write(c: &mut Criterion) {
    bench_lib::write::bench_table_batch_write_scaling(c, "table_batch_write", &[10, 50, 100, 200]);
}

// -- Read -------------------------------------------------------------------

fn docstore_read_scaling(c: &mut Criterion) {
    bench_lib::read::bench_docstore_read_scaling(c, "docstore_read_scaling", &[10, 100, 500, 1000]);
}

fn docstore_get_all(c: &mut Criterion) {
    bench_lib::read::bench_docstore_get_all_scaling(c, "docstore_get_all", &[10, 100, 500, 1000]);
}

fn docstore_contains_key(c: &mut Criterion) {
    bench_lib::read::bench_docstore_contains_key(c, "docstore_contains_key", &[100, 500, 1000]);
}

fn table_read_scaling(c: &mut Criterion) {
    bench_lib::read::bench_table_read_scaling(c, "table_read_scaling", &[10, 100, 500, 1000]);
}

fn table_search(c: &mut Criterion) {
    bench_lib::read::bench_table_search_scaling(c, "table_search", &[10, 100, 500, 1000]);
}

// -- Mutate -----------------------------------------------------------------

fn docstore_update_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_docstore_update_scaling(
        c,
        "docstore_update_scaling",
        &[10, 100, 500, 1000],
    );
}

fn table_update_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_table_update_scaling(c, "table_update_scaling", &[10, 100, 500, 1000]);
}

fn docstore_delete_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_docstore_delete_scaling(
        c,
        "docstore_delete_scaling",
        &[10, 100, 500, 1000],
    );
}

fn table_delete_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_table_delete_scaling(c, "table_delete_scaling", &[10, 100, 500, 1000]);
}

// -- Operational ------------------------------------------------------------

fn transaction_granularity(c: &mut Criterion) {
    bench_lib::operational::bench_transaction_granularity(
        c,
        "transaction_granularity",
        50,
        &[1, 5, 10, 50],
    );
}

fn multi_store_transaction(c: &mut Criterion) {
    bench_lib::operational::bench_multi_store_transaction(
        c,
        "multi_store_transaction",
        &[1, 2, 4, 8],
    );
}

fn store_viewer(c: &mut Criterion) {
    bench_lib::operational::bench_store_viewer(c, "store_viewer", &[10, 100, 500, 1000]);
}

fn database_creation(c: &mut Criterion) {
    bench_lib::operational::bench_database_creation(c, "database_lifecycle", &[2, 5, 10]);
}

fn mixed_workload(c: &mut Criterion) {
    bench_lib::operational::bench_mixed_workload(c, "mixed_workload", 200);
}

fn incremental_growth(c: &mut Criterion) {
    bench_lib::operational::bench_incremental_growth(c, "incremental_growth", &[0, 100, 500]);
}

fn entry_history(c: &mut Criterion) {
    bench_lib::operational::bench_entry_history(c, "entry_history", &[100, 500, 1000]);
}

// -- Criterion wiring -------------------------------------------------------

criterion_group! {
    name = write_benches;
    config = Criterion::default().configure_from_args();
    targets =
        docstore_write_scaling,
        docstore_batch_write,
        table_write_scaling,
        table_batch_write,
}

criterion_group! {
    name = read_benches;
    config = Criterion::default().configure_from_args();
    targets =
        docstore_read_scaling,
        docstore_get_all,
        docstore_contains_key,
        table_read_scaling,
        table_search,
}

criterion_group! {
    name = mutate_benches;
    config = Criterion::default().configure_from_args();
    targets =
        docstore_update_scaling,
        table_update_scaling,
        docstore_delete_scaling,
        table_delete_scaling,
}

criterion_group! {
    name = operational_benches;
    config = Criterion::default().configure_from_args();
    targets =
        transaction_granularity,
        multi_store_transaction,
        store_viewer,
        database_creation,
        mixed_workload,
        incremental_growth,
        entry_history,
}

criterion_main!(
    write_benches,
    read_benches,
    mutate_benches,
    operational_benches
);
