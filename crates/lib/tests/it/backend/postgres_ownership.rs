use std::time::Duration;

use eidetica::Error;
use eidetica::backend::{BackendError, database::SqlxBackend};
use sqlx::{AnyPool, Executor};

fn postgres_url() -> String {
    std::env::var("TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://localhost/eidetica_test".to_string())
}

fn postgres_tests_enabled() -> bool {
    std::env::var("TEST_BACKEND").as_deref() == Ok("postgres")
}

async fn test_schema() -> (String, String) {
    let schema = format!("ownership_{}", uuid::Uuid::new_v4().simple());
    let pool = admin_pool().await;
    pool.execute(format!("CREATE SCHEMA {schema}").as_str())
        .await
        .unwrap();
    (postgres_url(), schema)
}

async fn connect_schema(url: &str, schema: &str) -> SqlxBackend {
    SqlxBackend::test_connect_postgres_schema(url, schema.to_owned())
        .await
        .unwrap()
}

async fn admin_pool() -> AnyPool {
    sqlx::any::install_default_drivers();
    AnyPool::connect(&postgres_url()).await.unwrap()
}

async fn terminate(pool: &AnyPool, pid: i32) {
    let (terminated,): (bool,) = sqlx::query_as("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(terminated, "PostgreSQL session {pid} must be terminated");
}

fn assert_owned(error: Error) {
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );
    assert!(
        !error.to_string().contains(&postgres_url()),
        "ownership errors must not expose the connection URL: {error}"
    );
}

#[tokio::test]
async fn postgres_namespace_is_owned_for_backend_lifetime() {
    if !postgres_tests_enabled() {
        return;
    }
    let (url, schema) = test_schema().await;
    let first = connect_schema(&url, &schema).await;

    let error = match SqlxBackend::test_connect_postgres_schema(&url, schema.clone()).await {
        Ok(_) => panic!("a second backend for the same PostgreSQL namespace must be refused"),
        Err(error) => error,
    };
    assert_owned(error);

    drop(first);
    SqlxBackend::test_connect_postgres_schema(&url, schema)
        .await
        .expect("dropping the backend must release PostgreSQL ownership");
}

#[tokio::test]
async fn surviving_pool_session_blocks_takeover_after_keeper_loss() {
    if !postgres_tests_enabled() {
        return;
    }
    let (url, schema) = test_schema().await;
    let first = connect_schema(&url, &schema).await;
    let admin = admin_pool().await;
    let pool_pids = first.test_postgres_pool_pids().await.unwrap();
    terminate(&admin, first.test_postgres_owner_pid().await.unwrap()).await;

    let error = match SqlxBackend::test_connect_postgres_schema(&url, schema.clone()).await {
        Ok(_) => panic!("a surviving old pool session must block takeover"),
        Err(error) => error,
    };
    assert_owned(error);

    for pid in pool_pids {
        terminate(&admin, pid).await;
    }
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(owner) = SqlxBackend::test_connect_postgres_schema(&url, schema.clone()).await
            {
                break owner;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("takeover must succeed after all old sessions exit");
}

#[tokio::test]
async fn stale_pool_reconnect_is_fenced_after_takeover() {
    if !postgres_tests_enabled() {
        return;
    }
    let (url, schema) = test_schema().await;
    let first = connect_schema(&url, &schema).await;
    let old_pool = first.pool().clone();
    let old_token = first.test_postgres_token().to_owned();
    let admin = admin_pool().await;
    let mut pids = first.test_postgres_pool_pids().await.unwrap();
    pids.push(first.test_postgres_owner_pid().await.unwrap());
    for pid in pids {
        terminate(&admin, pid).await;
    }
    drop(first);

    let second = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(owner) = SqlxBackend::test_connect_postgres_schema(&url, schema.clone()).await
            {
                break owner;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("takeover must succeed after every old session exits");
    assert_ne!(old_token, second.test_postgres_token());

    let stale_acquire = tokio::spawn(async move { old_pool.acquire().await });
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !stale_acquire.is_finished(),
        "the stale pool must reject every reconnect after the ownership token changes"
    );
    stale_acquire.abort();
}

#[tokio::test]
async fn isolated_postgres_schemas_are_independent() {
    if !postgres_tests_enabled() {
        return;
    }
    let url = postgres_url();
    let first = SqlxBackend::connect_postgres_isolated(&url).await.unwrap();
    let second = SqlxBackend::connect_postgres_isolated(&url).await.unwrap();

    drop((first, second));
}
