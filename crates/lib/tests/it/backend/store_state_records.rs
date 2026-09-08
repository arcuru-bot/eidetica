use std::collections::BTreeMap;
use std::sync::Arc;

use eidetica::{
    backend::{
        BackendImpl, CacheScope, ProjectionDescriptor, RecordRange, StoreStateLifecycle,
        StoreStateRequest,
    },
    entry::ID,
};
use tokio::sync::Barrier;

use crate::helpers::test_backend;

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn existing_schema_v0_database_gets_store_state_tables() {
    use eidetica::backend::database::Sqlite;

    sqlx::any::install_default_drivers();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema-v0.db");
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let pool = sqlx::AnyPool::connect(&url).await.unwrap();
    sqlx::query("CREATE TABLE schema_version (version BIGINT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO schema_version (version) VALUES (0)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let backend = Sqlite::open(&path).await.unwrap();
    let version: (i64,) = sqlx::query_as("SELECT version FROM schema_version")
        .fetch_one(backend.pool())
        .await
        .unwrap();
    assert_eq!(version.0, 0);

    let tables: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN \
         ('store_state_namespaces', 'store_state_records')",
    )
    .fetch_one(backend.pool())
    .await
    .unwrap();
    assert_eq!(tables.0, 2);
}

fn request(database: &str, store: &str, lifecycle: StoreStateLifecycle) -> StoreStateRequest {
    StoreStateRequest {
        database: ID::from_bytes(database),
        store: store.to_string(),
        lifecycle,
        scope: CacheScope::Shared,
        projection: ProjectionDescriptor {
            name: "test/opaque".to_string(),
            version: 0,
        },
        source_key: b"snapshot".to_vec(),
    }
}

async fn publish(
    backend: &dyn BackendImpl,
    request: StoreStateRequest,
    records: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
) -> eidetica::backend::RecordView {
    let token = backend.begin_store_state_staging(request).await.unwrap();
    backend
        .stage_store_state_records(
            &token,
            records
                .into_iter()
                .map(|(key, value)| (key, Some(value)))
                .collect(),
        )
        .await
        .unwrap();
    backend.publish_store_state(token).await.unwrap()
}

#[tokio::test]
async fn point_lookup_namespace_separation_and_staging_invisibility() {
    let backend = test_backend().await;
    let derived = request("db", "store", StoreStateLifecycle::Derived);
    let token = backend
        .begin_store_state_staging(derived.clone())
        .await
        .unwrap();
    backend
        .stage_store_state_records(
            &token,
            BTreeMap::from([(b"key".to_vec(), Some(b"derived".to_vec()))]),
        )
        .await
        .unwrap();
    assert!(
        backend
            .resolve_store_state(&derived)
            .await
            .unwrap()
            .is_none()
    );

    let derived_view = backend.publish_store_state(token).await.unwrap();
    assert_eq!(
        backend
            .store_state_record_get(&derived_view, b"key")
            .await
            .unwrap(),
        Some(b"derived".to_vec())
    );

    let authoritative = request("db", "store", StoreStateLifecycle::Authoritative);
    let authoritative_view = publish(
        backend.as_ref(),
        authoritative.clone(),
        [(b"key".to_vec(), b"authority".to_vec())],
    )
    .await;
    assert_ne!(derived_view, authoritative_view);
    assert_eq!(
        backend
            .store_state_record_get(&authoritative_view, b"key")
            .await
            .unwrap(),
        Some(b"authority".to_vec())
    );
    assert!(
        backend
            .resolve_store_state(&derived)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .resolve_store_state(&authoritative)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn ordered_half_open_scan_pages_without_duplicates_or_skips() {
    let backend = test_backend().await;
    let view = publish(
        backend.as_ref(),
        request("scan", "binary", StoreStateLifecycle::Derived),
        [
            (vec![0x00], vec![0]),
            (vec![0x00, 0xff], vec![1]),
            (vec![0x01], vec![2]),
            (vec![0x01, 0x00], vec![3]),
            (vec![0xff], vec![4]),
        ],
    )
    .await;

    let range = RecordRange {
        start: Some(vec![0x00]),
        end: Some(vec![0xff]),
    };
    let first = backend
        .store_state_record_scan(&view, &range, None, 2)
        .await
        .unwrap();
    assert_eq!(
        first.records.iter().map(|r| &r.0).collect::<Vec<_>>(),
        vec![&vec![0x00], &vec![0x00, 0xff]]
    );
    let second = backend
        .store_state_record_scan(&view, &range, first.next.as_deref(), 2)
        .await
        .unwrap();
    assert_eq!(
        second.records.iter().map(|r| &r.0).collect::<Vec<_>>(),
        vec![&vec![0x01], &vec![0x01, 0x00]]
    );
    assert!(second.next.is_none());
}

#[tokio::test]
async fn failed_publish_is_invisible_and_ready_derived_is_immutable() {
    let backend = test_backend().await;
    let request = request("failure", "store", StoreStateLifecycle::Derived);
    let token = backend
        .begin_store_state_staging(request.clone())
        .await
        .unwrap();
    backend
        .stage_store_state_records(&token, BTreeMap::from([(b"bad".to_vec(), None)]))
        .await
        .unwrap();
    assert!(backend.publish_store_state(token).await.is_err());
    assert!(
        backend
            .resolve_store_state(&request)
            .await
            .unwrap()
            .is_none()
    );

    let token = backend
        .begin_store_state_staging(request.clone())
        .await
        .unwrap();
    backend
        .stage_store_state_records(
            &token,
            BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
        )
        .await
        .unwrap();
    let published = backend.publish_store_state(token.clone()).await.unwrap();
    assert!(
        backend
            .stage_store_state_records(
                &token,
                BTreeMap::from([(b"key".to_vec(), Some(b"changed".to_vec()))]),
            )
            .await
            .is_err()
    );
    assert_eq!(
        backend
            .store_state_record_get(&published, b"key")
            .await
            .unwrap(),
        Some(b"value".to_vec())
    );
}

#[tokio::test]
async fn clearing_derived_records_preserves_authoritative_bytes() {
    let backend = test_backend().await;
    let authoritative_request = request("clear", "store", StoreStateLifecycle::Authoritative);
    let authoritative = publish(
        backend.as_ref(),
        authoritative_request.clone(),
        [(b"key".to_vec(), b"authority".to_vec())],
    )
    .await;
    publish(
        backend.as_ref(),
        request("clear", "store", StoreStateLifecycle::Derived),
        [(b"key".to_vec(), b"derived".to_vec())],
    )
    .await;

    backend.clear_derived_store_state().await.unwrap();

    assert_eq!(
        backend
            .store_state_record_get(&authoritative, b"key")
            .await
            .unwrap(),
        Some(b"authority".to_vec())
    );
    assert!(
        backend
            .resolve_store_state(&authoritative_request)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .resolve_store_state(&request("clear", "store", StoreStateLifecycle::Derived))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn zero_limit_scan_yields_an_empty_page() {
    let backend = test_backend().await;
    let view = publish(
        backend.as_ref(),
        request("zero", "limit", StoreStateLifecycle::Derived),
        [(b"key".to_vec(), b"value".to_vec())],
    )
    .await;

    let page = backend
        .store_state_record_scan(&view, &RecordRange::default(), None, 0)
        .await
        .unwrap();
    assert!(page.records.is_empty());
    assert!(page.next.is_none());
}

/// Two materializers can derive the same state from the same source at once.
/// The loser adopts the winner's namespace instead of failing, so a cold-read
/// race stays invisible to callers.
#[tokio::test]
async fn racing_publishers_of_one_target_agree_on_a_single_winner() {
    let backend = test_backend().await;
    let request = request("race", "store", StoreStateLifecycle::Derived);

    let first = backend
        .begin_store_state_staging(request.clone())
        .await
        .unwrap();
    let second = backend
        .begin_store_state_staging(request.clone())
        .await
        .unwrap();
    for token in [&first, &second] {
        backend
            .stage_store_state_records(
                token,
                BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .await
            .unwrap();
    }

    let winner = backend.publish_store_state(first).await.unwrap();
    let loser = backend.publish_store_state(second).await.unwrap();
    assert_eq!(winner, loser);
    assert_eq!(
        backend.resolve_store_state(&request).await.unwrap(),
        Some(winner.clone())
    );
    assert_eq!(
        backend
            .store_state_record_get(&winner, b"key")
            .await
            .unwrap(),
        Some(b"value".to_vec())
    );
}

/// Publishing the same (cloned) staging token twice must be idempotent: the
/// second publish returns the same view and the published state stays intact.
/// A repeat publish that deletes the ready namespace turns a retry into data
/// loss, so this asserts the state survives, not just the return value.
#[tokio::test]
async fn repeat_publish_of_a_cloned_token_keeps_published_state() {
    let backend = test_backend().await;
    let request = request("repeat", "store", StoreStateLifecycle::Derived);
    let token = backend
        .begin_store_state_staging(request.clone())
        .await
        .unwrap();
    backend
        .stage_store_state_records(
            &token,
            BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
        )
        .await
        .unwrap();

    let first = backend.publish_store_state(token.clone()).await.unwrap();
    let second = backend.publish_store_state(token).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        backend.resolve_store_state(&request).await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        backend
            .store_state_record_get(&first, b"key")
            .await
            .unwrap(),
        Some(b"value".to_vec())
    );
}

/// A view whose namespace was cleared (or was never ready) is an explicit
/// error — never a missing key and never an empty page. A missing key on a
/// live view still reads as absent, and a published-but-empty snapshot still
/// scans as an empty page, so callers can tell the three cases apart.
#[tokio::test]
async fn cleared_view_errors_instead_of_reading_missing_or_empty() {
    let backend = test_backend().await;
    let derived_request = request("cleared", "store", StoreStateLifecycle::Derived);
    let view = publish(
        backend.as_ref(),
        derived_request.clone(),
        [(b"key".to_vec(), b"value".to_vec())],
    )
    .await;
    assert_eq!(
        backend
            .store_state_record_get(&view, b"absent")
            .await
            .unwrap(),
        None
    );

    backend.clear_derived_store_state().await.unwrap();
    assert!(
        backend
            .resolve_store_state(&derived_request)
            .await
            .unwrap()
            .is_none()
    );

    let get_err = backend
        .store_state_record_get(&view, b"key")
        .await
        .unwrap_err();
    assert!(
        get_err.is_invalid_store_state_view(),
        "cleared view must error, got: {get_err}"
    );
    let scan_err = backend
        .store_state_record_scan(&view, &RecordRange::default(), None, 16)
        .await
        .unwrap_err();
    assert!(
        scan_err.is_invalid_store_state_view(),
        "cleared view must error, got: {scan_err}"
    );
}

/// A namespace can be published with no records at all. That valid-but-empty
/// snapshot scans as an empty page — the Ok case the cleared-view test above
/// contrasts against.
#[tokio::test]
async fn valid_empty_snapshot_scans_as_an_empty_page() {
    let backend = test_backend().await;
    let view = publish(
        backend.as_ref(),
        request("empty", "store", StoreStateLifecycle::Derived),
        std::iter::empty::<(Vec<u8>, Vec<u8>)>(),
    )
    .await;

    let page = backend
        .store_state_record_scan(&view, &RecordRange::default(), None, 16)
        .await
        .unwrap();
    assert!(page.records.is_empty());
    assert!(page.next.is_none());
}

/// Eight materializers race to publish one target from a barrier start so the
/// publishes genuinely overlap. Every publisher must agree on a single winner
/// whose records stay intact — no torn namespaces, no duplicate ready state.
/// This is a regression guard on the serialization outcome, not a proof of the
/// locking: it passes by construction on any correct serialization and fails
/// on torn or duplicated state however the tasks interleave.
#[tokio::test]
async fn barrier_started_publishers_agree_on_a_single_winner() {
    const PUBLISHERS: usize = 8;
    let backend: Arc<dyn BackendImpl> = test_backend().await.into();
    let request = request("barrier", "store", StoreStateLifecycle::Derived);

    let mut tokens = Vec::with_capacity(PUBLISHERS);
    for _ in 0..PUBLISHERS {
        let token = backend
            .begin_store_state_staging(request.clone())
            .await
            .unwrap();
        backend
            .stage_store_state_records(
                &token,
                BTreeMap::from([(b"key".to_vec(), Some(b"value".to_vec()))]),
            )
            .await
            .unwrap();
        tokens.push(token);
    }

    let barrier = Arc::new(Barrier::new(PUBLISHERS));
    let mut tasks = tokio::task::JoinSet::new();
    for token in tokens {
        let backend = Arc::clone(&backend);
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            backend.publish_store_state(token).await
        });
    }

    let mut views = Vec::with_capacity(PUBLISHERS);
    while let Some(outcome) = tasks.join_next().await {
        views.push(outcome.unwrap().unwrap());
    }
    for view in &views {
        assert_eq!(view, &views[0]);
    }
    assert_eq!(
        backend.resolve_store_state(&request).await.unwrap(),
        Some(views[0].clone())
    );
    assert_eq!(
        backend
            .store_state_record_get(&views[0], b"key")
            .await
            .unwrap(),
        Some(b"value".to_vec())
    );
}
