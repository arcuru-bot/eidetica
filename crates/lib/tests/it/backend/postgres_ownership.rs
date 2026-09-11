use eidetica::Error;
use eidetica::backend::{BackendError, database::SqlxBackend};

fn postgres_url() -> String {
    std::env::var("TEST_POSTGRES_URL")
        .unwrap_or_else(|_| "postgres://localhost/eidetica_test".to_string())
}

fn postgres_tests_enabled() -> bool {
    std::env::var("TEST_BACKEND").as_deref() == Ok("postgres")
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
    let url = postgres_url();
    let first = SqlxBackend::connect_postgres(&url).await.unwrap();

    let error = match SqlxBackend::connect_postgres(&url).await {
        Ok(_) => panic!("a second backend for the same PostgreSQL namespace must be refused"),
        Err(error) => error,
    };
    assert_owned(error);

    drop(first);
    SqlxBackend::connect_postgres(&url)
        .await
        .expect("dropping the backend must release PostgreSQL ownership");
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
