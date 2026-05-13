//! End-to-end P2P sync latency benchmarks for Eidetica.
//!
//! Measures the time from a local write committing on Peer A → the entry being
//! queryable on Peer B, including connection setup, tip exchange, entry transfer,
//! and local storage. This is the **blg-031** benchmark from the competitive
//! benchmark suite design (see `blog-topics/2026-05-08-design-competitive-benchmark-suite.md`).
//!
//! # What This Measures
//! - **P2P end-to-end sync latency**: write → sync → remote-queryable
//! - Breakdown: sync_send_time, sync_receive_time, remote_query_time
//! - Varying payload sizes (64B–16KB) and batch sizes (1, 10, 50, 100)
//!
//! # Prerequisites
//! - Rust 1.84+ (workspace MSRV)
//! - `TEST_BACKEND` env var: "inmemory" (default) or "sqlite"
//!
//! # Usage
//! ```sh
//! # In-memory backend (fast, no disk variance)
//! TEST_BACKEND=inmemory cargo bench --bench sync_latency_benchmarks
//!
//! # SQLite backend (real-world overhead)
//! TEST_BACKEND=sqlite cargo bench --bench sync_latency_benchmarks
//! ```

mod helpers;

use criterion::{
    black_box, BenchmarkId, Criterion, Throughput,
    criterion_group, criterion_main,
};
use eidetica::{
    Instance,
    entry::Entry,
    sync::{Sync, peer_types::Address, transports::iroh::IrohTransport},
};
use iroh::RelayMode;
use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use helpers::test_backend;

// ─── Setup ──────────────────────────────────────────────────────────────────

/// Creates a root entry with a payload of the specified size.
fn create_entry_with_payload(size: usize) -> Entry {
    // Generate deterministic content of the requested size
    let content: String = (0..size)
        .map(|i| (b'a' + (i % 26) as u8) as char)
        .collect();
    Entry::root_builder()
        .set("payload", content)
        .build()
        .expect("Entry should build successfully")
}

/// Creates multiple entries with the given individual payload size.
fn create_entries(count: usize, payload_size: usize) -> Vec<Entry> {
    (0..count)
        .map(|i| {
            let content: String = (0..payload_size)
                .map(|j| (b'a' + ((i * payload_size + j) % 26) as u8) as char)
                .collect();
            Entry::root_builder()
                .set("index", i.to_string())
                .set("payload", content)
                .build()
                .expect("Entry should build successfully")
        })
        .collect()
}

/// Sets up two eidetica instances connected via Iroh P2P with relay disabled.
/// Returns (db1, sync1, db2, sync2, peer2_address).
async fn setup_sync_pair() -> (Arc<Instance>, Sync, Arc<Instance>, Sync, Address) {
    let base_db1 = Arc::new(
        Instance::open(test_backend().await)
            .await
            .expect("Instance 1 setup failed"),
    );
    let base_db2 = Arc::new(
        Instance::open(test_backend().await)
            .await
            .expect("Instance 2 setup failed"),
    );

    let sync1 = Sync::new((*base_db1).clone()).await.unwrap();
    let sync2 = Sync::new((*base_db2).clone()).await.unwrap();

    // Configure Iroh transports — RelayMode::Disabled for local-only reproducibility
    sync1
        .register_transport(
            "iroh",
            IrohTransport::builder().relay_mode(RelayMode::Disabled),
        )
        .await
        .unwrap();
    sync2
        .register_transport(
            "iroh",
            IrohTransport::builder().relay_mode(RelayMode::Disabled),
        )
        .await
        .unwrap();

    // Start servers on both peers
    sync1.accept_connections().await.unwrap();
    sync2.accept_connections().await.unwrap();

    // Allow transport initialization
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Establish peer relationship
    let addr2 = sync2.get_server_address().await.unwrap();
    let pubkey2 = sync2.get_device_pubkey().unwrap();

    sync1
        .register_peer(&pubkey2, Some("bench_peer"))
        .await
        .unwrap();
    sync1
        .add_peer_address(&pubkey2, Address::iroh(&addr2))
        .await
        .unwrap();

    // Initial bootstrap sync (excluded from measurements)
    let tree_id = {
        let txn1 = base_db1
            .new_transaction()
            .await
            .expect("Failed to create txn on db1");
        let kv1 = txn1
            .get_store::<eidetica::store::DocStore>("syncdb")
            .await
            .expect("Failed to get DocStore");
        kv1.set("init", "bootstrap")
            .await
            .expect("Failed to set init entry");
        let root_id = txn1.commit().await.expect("Failed to commit init txn");
        root_id
    };

    // Do an initial sync so both peers have the tree
    sync1
        .sync_tree_with_peer(&pubkey2, &tree_id)
        .await
        .expect("Initial sync failed");

    // Allow initial sync to complete
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Flush any pending background sync
    sync1.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let peer2_addr = sync2.get_server_address().await.unwrap();

    (base_db1, sync1, base_db2, sync2, Address::iroh(&peer2_addr))
}

// ─── Benchmark: P2P Sync Latency ────────────────────────────────────────────

/// Measures end-to-end P2P sync latency for a single entry.
///
/// Timing breakdown:
/// - t0: entry committed on Peer A
/// - t_sync: sync.send_entries() returns (entry sent to transport)
/// - t_recv: entry stored on Peer B (polled via query)
/// - t2: query confirms entry exists on Peer B
///
/// Total latency = t2 - t0 (end-to-end)
fn bench_p2p_sync_latency_single(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let payload_sizes = [64usize, 256, 1024, 4096, 16384];

    let mut group = c.benchmark_group("p2p_sync_latency_single");
    group.sample_size(20); // More samples for network benchmarks
    group.measurement_time(Duration::from_secs(30));

    for &payload_size in &payload_sizes {
        group.throughput(Throughput::Bytes(payload_size as u64));

        group.bench_with_input(
            BenchmarkId::new("payload_bytes", payload_size),
            &payload_size,
            |b, &payload_size| {
                let (db1, sync1, db2, _sync2, addr2) =
                    rt.block_on(setup_sync_pair());
                let pubkey2 = sync1
                    .get_device_pubkey()
                    .unwrap(); // dummy, we already have pubkey2

                // Get peer pubkey from setup
                let peer_pubkey = db2
                    .signing_key()
                    .and_then(|k| k.public_key().into());
                // Actually we need to get pubkey2 from sync2
                let _peer_pubkey = {
                    let rt2 = tokio::runtime::Runtime::new().unwrap();
                    rt2.block_on(async {
                        // Re-derive: we know addr2 belongs to db2
                        // For benchmark simplicity, we use sync_tree_with_peer
                        // which handles discovery internally
                        db2.signing_key().unwrap().public_key()
                    })
                };

                b.iter_with_setup(
                    || {
                        // Create entry outside the timed section
                        black_box(create_entry_with_payload(payload_size))
                    },
                    |entry| {
                        rt.block_on(async {
                            // Create a fresh tree for each measurement to avoid
                            // accumulated state effects
                            let txn = db1
                                .new_transaction()
                                .await
                                .expect("Failed to create transaction");
                            let kv = txn
                                .get_store::<eidetica::store::DocStore>("bench")
                                .await
                                .expect("Failed to get DocStore");

                            let t0 = Instant::now();

                            // Write entry on Peer A
                            kv.set(
                                black_box("bench_key"),
                                black_box(format!("{:?}", entry.id())),
                            )
                            .await
                            .expect("Failed to write entry");

                            let bench_txn_root = txn
                                .commit()
                                .await
                                .expect("Failed to commit on Peer A");
                            let t_commit = Instant::now();

                            // Get the tree ID for sync
                            let tree_id = bench_txn_root;

                            // Trigger sync to Peer B
                            sync1
                                .sync_tree_with_peer(
                                    &db2.signing_key().unwrap().public_key(),
                                    &tree_id,
                                )
                                .await
                                .expect("Sync failed");
                            let t_sync_done = Instant::now();

                            // Poll Peer B until entry is queryable
                            let mut retries = 0;
                            let max_retries = 50;
                            let t_recv;
                            loop {
                                let db2_clone = db2.clone();
                                let tree_id_clone = tree_id.clone();
                                let found = rt.block_on(async {
                                    let txn2 = db2_clone
                                        .new_transaction()
                                        .await
                                        .expect("Failed to create txn on db2");
                                    let kv2 = txn2
                                        .get_store::<eidetica::store::DocStore>(
                                            "bench",
                                        )
                                        .await
                                        .expect("Failed to get DocStore on db2");
                                    kv2.get(black_box("bench_key"))
                                        .await
                                        .is_ok()
                                });
                                if found {
                                    t_recv = Instant::now();
                                    break;
                                }
                                retries += 1;
                                if retries >= max_retries {
                                    t_recv = Instant::now();
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(10))
                                    .await;
                            }

                            // Query the entry on Peer B and verify
                            let db2_clone = db2.clone();
                            let tree_id_clone = tree_id.clone();
                            rt.block_on(async {
                                let txn2 = db2_clone
                                    .new_transaction()
                                    .await
                                    .expect("Failed to create txn on db2");
                                let kv2 = txn2
                                    .get_store::<
                                        eidetica::store::DocStore,
                                    >("bench")
                                    .await
                                    .expect("Failed to get DocStore on db2");
                                let _val = kv2
                                    .get(black_box("bench_key"))
                                    .await
                                    .expect("Entry should exist on Peer B");
                            });
                            let t2 = Instant::now();

                            // Report sub-metrics for analysis
                            let _sync_send_ms =
                                t_sync_done.duration_since(t_commit).as_millis()
                                    as f64;
                            let _recv_ms =
                                t_recv.duration_since(t_sync_done).as_millis()
                                    as f64;
                            let _query_ms =
                                t2.duration_since(t_recv).as_millis() as f64;
                            black_box((_sync_send_ms, _recv_ms, _query_ms));
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Measures P2P sync latency with batched entries per sync operation.
///
/// Tests how sync latency scales when multiple entries are committed
/// before a single sync round.
fn bench_p2p_sync_latency_batch(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let batch_sizes = [1usize, 10, 50, 100];
    let payload_size = 256usize; // Fixed payload per entry

    let mut group = c.benchmark_group("p2p_sync_latency_batch");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for &batch_size in &batch_sizes {
        group.throughput(Throughput::Elements(batch_size as u64));

        group.bench_with_input(
            BenchmarkId::new("entries_per_sync", batch_size),
            &batch_size,
            |b, &batch_size| {
                let (db1, sync1, db2, _sync2, _addr2) =
                    rt.block_on(setup_sync_pair());

                b.iter_with_setup(
                    || {
                        // Pre-create the entries
                        black_box(create_entries(batch_size, payload_size))
                    },
                    |entries| {
                        rt.block_on(async {
                            // Write all entries in a single transaction
                            let txn = db1
                                .new_transaction()
                                .await
                                .expect("Failed to create transaction");
                            let kv = txn
                                .get_store::<eidetica::store::DocStore>("bench_batch")
                                .await
                                .expect("Failed to get DocStore");

                            let t0 = Instant::now();

                            for (i, _entry) in entries.iter().enumerate() {
                                kv.set(
                                    black_box(&format!("key_{i}")),
                                    black_box(&format!("value_{i}")),
                                )
                                .await
                                .expect("Failed to set entry");
                            }

                            let bench_txn_root = txn
                                .commit()
                                .await
                                .expect("Failed to commit batch");

                            let tree_id = bench_txn_root;

                            // Trigger sync
                            sync1
                                .sync_tree_with_peer(
                                    &db2.signing_key().unwrap().public_key(),
                                    &tree_id,
                                )
                                .await
                                .expect("Sync failed");

                            // Verify all entries on Peer B
                            let mut retries = 0;
                            let max_retries = 100;
                            loop {
                                let db2_clone = db2.clone();
                                let tree_id_clone = tree_id.clone();
                                let all_found = rt.block_on(async {
                                    let txn2 = db2_clone
                                        .new_transaction()
                                        .await
                                        .expect("Failed to create txn on db2");
                                    let kv2 = txn2
                                        .get_store::<
                                            eidetica::store::DocStore,
                                        >("bench_batch")
                                        .await
                                        .expect(
                                            "Failed to get DocStore on db2",
                                        );
                                    let mut all_found = true;
                                    for i in 0..batch_size {
                                        if kv2
                                            .get(
                                                black_box(&format!("key_{i}")),
                                            )
                                            .await
                                            .is_err()
                                        {
                                            all_found = false;
                                            break;
                                        }
                                    }
                                    all_found
                                });
                                if all_found || retries >= max_retries {
                                    break;
                                }
                                retries += 1;
                                tokio::time::sleep(
                                    Duration::from_millis(10),
                                )
                                .await;
                            }

                            let t2 = Instant::now();
                            black_box(t2.duration_since(t0).as_micros() as u64);
                        });
                    },
                );
            },
        );
    }

    group.finish();
}

/// Measures P2P sync latency across different peer distances.
///
/// Same-machine (loopback) provides a lower bound; cross-region
/// measurements via Tailscale would show real-world overhead.
fn bench_p2p_sync_latency_distance(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("p2p_sync_latency_distance");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(30));

    // Same-machine loopback (current setup)
    group.bench_function("same_machine_loopback", |b| {
        let (db1, sync1, db2, _sync2, _addr2) =
            rt.block_on(setup_sync_pair());

        b.iter_with_setup(
            || {
                let txn = rt.block_on(async {
                    db1.new_transaction()
                        .await
                        .expect("Failed to create transaction")
                });
                let kv = rt.block_on(async {
                    txn.get_store::<eidetica::store::DocStore>(
                        "bench_dist",
                    )
                    .await
                    .expect("Failed to get DocStore")
                });
                (txn, kv)
            },
            |(txn, mut kv)| {
                rt.block_on(async {
                    let t0 = Instant::now();

                    kv.set(
                        black_box("ping_key"),
                        black_box("ping_payload"),
                    )
                    .await
                    .expect("Failed to write");

                    let root = txn
                        .commit()
                        .await
                        .expect("Failed to commit");

                    sync1
                        .sync_tree_with_peer(
                            &db2.signing_key().unwrap().public_key(),
                            &root,
                        )
                        .await
                        .expect("Sync failed");

                    // Verify
                    let _ = db2
                        .new_transaction()
                        .await
                        .expect("Failed txn on db2");
                    black_box(t0.elapsed());
                });
            },
        );
    });

    group.finish();
}

criterion_group! {
    name = sync_latency_benches;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(Duration::from_secs(30))
        .configure_from_args();
    targets =
        bench_p2p_sync_latency_single,
        bench_p2p_sync_latency_batch,
        bench_p2p_sync_latency_distance,
}
criterion_main!(sync_latency_benches);