//! Extended benchmarks with larger-scale parameters.
//!
//! Thin wrapper over `bench_lib` with extended parameter sets for deep
//! performance analysis. These are excluded from the default `just bench` /
//! `nix build .#bench` workflow due to their longer run time.
//!
//! Run with: `just bench-ext` or `cargo bench --bench extended_benchmarks`
//!
//! All benchmarks are local-only (no sync). Use TEST_BACKEND env var to select
//! the storage backend (default: sqlite, also: inmemory).

mod bench_lib;
mod helpers;

use criterion::{Criterion, criterion_group, criterion_main};

// -- Write ------------------------------------------------------------------

fn docstore_write_scaling(c: &mut Criterion) {
    bench_lib::write::bench_docstore_write_scaling(c, "ext/docstore_write_scaling", &[2000, 5000]);
}

fn docstore_batch_write(c: &mut Criterion) {
    bench_lib::write::bench_docstore_batch_write_scaling(
        c,
        "ext/docstore_batch_write",
        &[500, 1000],
    );
}

fn table_write_scaling(c: &mut Criterion) {
    bench_lib::write::bench_table_write_scaling(c, "ext/table_write_scaling", &[2000, 5000]);
}

fn table_batch_write(c: &mut Criterion) {
    bench_lib::write::bench_table_batch_write_scaling(c, "ext/table_batch_write", &[500, 1000]);
}

// -- Read -------------------------------------------------------------------

fn docstore_read_scaling(c: &mut Criterion) {
    bench_lib::read::bench_docstore_read_scaling(c, "ext/docstore_read_scaling", &[2000, 5000]);
}

fn docstore_get_all(c: &mut Criterion) {
    bench_lib::read::bench_docstore_get_all_scaling(c, "ext/docstore_get_all", &[2000, 5000]);
}

fn docstore_contains_key(c: &mut Criterion) {
    bench_lib::read::bench_docstore_contains_key(c, "ext/docstore_contains_key", &[2000, 5000]);
}

fn table_read_scaling(c: &mut Criterion) {
    bench_lib::read::bench_table_read_scaling(c, "ext/table_read_scaling", &[2000, 5000]);
}

fn table_search(c: &mut Criterion) {
    bench_lib::read::bench_table_search_scaling(c, "ext/table_search", &[2000, 5000]);
}

// -- Mutate -----------------------------------------------------------------

fn docstore_update_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_docstore_update_scaling(
        c,
        "ext/docstore_update_scaling",
        &[2000, 5000],
    );
}

fn table_update_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_table_update_scaling(c, "ext/table_update_scaling", &[2000, 5000]);
}

fn docstore_delete_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_docstore_delete_scaling(
        c,
        "ext/docstore_delete_scaling",
        &[2000, 5000],
    );
}

fn table_delete_scaling(c: &mut Criterion) {
    bench_lib::mutate::bench_table_delete_scaling(c, "ext/table_delete_scaling", &[2000, 5000]);
}

// -- Operational ------------------------------------------------------------

fn transaction_granularity(c: &mut Criterion) {
    bench_lib::operational::bench_transaction_granularity(
        c,
        "ext/transaction_granularity",
        200,
        &[1, 10, 50, 100, 200],
    );
}

fn multi_store_transaction(c: &mut Criterion) {
    bench_lib::operational::bench_multi_store_transaction(
        c,
        "ext/multi_store_transaction",
        &[1, 2, 4, 8],
    );
}

fn store_viewer(c: &mut Criterion) {
    bench_lib::operational::bench_store_viewer(c, "ext/store_viewer", &[2000, 5000]);
}

fn database_creation(c: &mut Criterion) {
    bench_lib::operational::bench_database_creation(c, "ext/database_lifecycle", &[2, 5, 10]);
}

fn mixed_workload(c: &mut Criterion) {
    bench_lib::operational::bench_mixed_workload(c, "ext/mixed_workload", 1000);
}

fn incremental_growth(c: &mut Criterion) {
    bench_lib::operational::bench_incremental_growth(c, "ext/incremental_growth", &[1000, 2000]);
}

fn entry_history(c: &mut Criterion) {
    bench_lib::operational::bench_entry_history(c, "ext/entry_history", &[2000, 5000]);
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
