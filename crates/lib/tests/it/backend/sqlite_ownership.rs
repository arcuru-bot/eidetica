use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use eidetica::Error;
use eidetica::backend::{BackendError, database::SqlxBackend};

use crate::SQLITE_OWNER_HELPER_ENV;

#[tokio::test]
async fn direct_sqlite_owner_is_exclusive_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("owner.db");

    let mut owner = Command::new(std::env::current_exe().unwrap())
        .env(SQLITE_OWNER_HELPER_ENV, &database_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(owner.stdout.take().unwrap());
    let mut ready = String::new();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "EIDETICA_SQLITE_OWNER_READY");

    let equivalent_path = dir.path().join(".").join("owner.db");
    let error = match SqlxBackend::open_sqlite(&equivalent_path).await {
        Ok(_) => panic!("a concurrent direct owner must be refused"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { ref namespace } if namespace == &database_path.canonicalize().unwrap().display().to_string())
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );
    assert!(
        error.to_string().contains("connect through"),
        "error should explain how to share the instance: {error}"
    );

    drop(owner.stdin.take());
    assert!(owner.wait().unwrap().success());

    SqlxBackend::open_sqlite(&database_path)
        .await
        .expect("ownership must be released when the owner process exits");
}

#[tokio::test]
async fn sqlite_uri_aliases_share_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("uri.db");
    let uri = format!("sqlite:{}?mode=rwc", database_path.display());
    let file_uri = format!("sqlite:file:{}?mode=rwc", database_path.display());
    let first = SqlxBackend::connect_sqlite(&uri).await.unwrap();

    let error = match SqlxBackend::connect_sqlite(&file_uri).await {
        Ok(_) => panic!("a file URI must contend with the equivalent SQLite path"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );

    drop(first);
}

#[tokio::test]
async fn sqlite_url_preserves_an_encoded_question_mark_in_the_filename() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("encoded?.db");
    let encoded_path = database_path.to_string_lossy().replace('?', "%3F");
    let url = format!("sqlite:{encoded_path}?mode=rwc");

    let first = SqlxBackend::connect_sqlite(&url)
        .await
        .expect("an encoded question mark is part of the SQLite filename");
    assert!(database_path.exists());

    let error = match SqlxBackend::open_sqlite(&database_path).await {
        Ok(_) => panic!("the encoded URL and filesystem path must share ownership"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );

    drop(first);
}

#[cfg(unix)]
#[tokio::test]
async fn sqlite_rejects_canonical_path_replacement_after_claim() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("database.db");
    let replacement_path = dir.path().join("replacement.db");
    std::fs::File::create(&database_path).unwrap();
    std::fs::File::create(&replacement_path).unwrap();

    let database_for_hook = database_path.clone();
    SqlxBackend::testing_after_sqlite_claim(database_path.canonicalize().unwrap(), move |_| {
        std::fs::rename(&replacement_path, &database_for_hook).unwrap();
    });
    let error =
        match SqlxBackend::connect_sqlite(&format!("sqlite:{}?mode=rw", database_path.display()))
            .await
        {
            Ok(_) => panic!(
                "a replaced canonical pathname must not connect under the old ownership lock"
            ),
            Err(error) => error,
        };
    assert!(
        error.to_string().contains("no longer matches claimed file"),
        "expected a claimed-file identity error, got {error:?}"
    );
}

#[tokio::test]
async fn sqlite_repeated_mode_uses_the_last_value_when_claiming_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("repeated-mode.db");
    let url = format!("sqlite:{}?mode=rw&mode=rwc", database_path.display());

    SqlxBackend::connect_sqlite(&url)
        .await
        .expect("the final mode=rwc must let ownership pre-open create the database");
    assert!(
        database_path.exists(),
        "the first mode=rw must not prevent the final mode=rwc from creating the database"
    );
}

#[tokio::test]
async fn sqlite_repeated_memory_mode_stays_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("must-not-exist.db");
    let url = format!(
        "sqlite:{}?mode=memory&mode=rw&cache=shared",
        database_path.display()
    );

    let backend = SqlxBackend::connect_sqlite(&url)
        .await
        .expect("an earlier mode=memory flag must remain effective");
    assert!(
        !database_path.exists(),
        "an accumulated memory mode must not open a filesystem database"
    );
    drop(backend);
}

#[tokio::test]
async fn sqlite_hard_links_share_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("original.db");
    let alias_path = dir.path().join("alias.db");
    let first = SqlxBackend::open_sqlite(&database_path).await.unwrap();
    std::fs::hard_link(&database_path, &alias_path).unwrap();

    let error = match SqlxBackend::open_sqlite(&alias_path).await {
        Ok(_) => panic!("hard links to one SQLite file must contend"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );

    drop(first);
}

#[tokio::test]
async fn persistent_sqlite_filename_containing_memory_is_owned() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("persistent:memory:.db");
    let url = format!("sqlite:{}?mode=rwc", database_path.display());
    let first = SqlxBackend::connect_sqlite(&url).await.unwrap();

    let error = match SqlxBackend::connect_sqlite(&url).await {
        Ok(_) => panic!("a persistent filename containing :memory: must still be owned"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );

    drop(first);
}

#[tokio::test]
async fn direct_sqlite_owner_is_exclusive_within_one_process() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("same-process.db");

    let first = SqlxBackend::open_sqlite(&database_path).await.unwrap();
    let error = match SqlxBackend::open_sqlite(&database_path).await {
        Ok(_) => panic!("a second direct backend must be refused"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            Error::Backend(ref error)
                if matches!(**error, BackendError::StorageAlreadyOwned { .. })
        ),
        "expected StorageAlreadyOwned, got {error:?}"
    );

    drop(first);

    SqlxBackend::open_sqlite(&database_path)
        .await
        .expect("dropping the backend must release ownership");
}

#[tokio::test]
async fn sqlite_testing_checkout_survives_backend_drop() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("stale-pool.db");

    let first = SqlxBackend::open_sqlite(&database_path).await.unwrap();
    let connection = first.test_sqlite_checked_out_connection().await.unwrap();
    drop(first);

    // SQLx keeps an already checked-out connection usable after Pool::close().
    // The production fence is the crate-private pool accessor; this test-only
    // hook exercises SQLx's documented lifecycle without making that shape public.
    let mut connection = connection.detach();
    let (value,): (i64,) = sqlx::query_as("SELECT 1")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(value, 1);
    drop(connection);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_sqlite_initialization_releases_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("failed.db");
    let url = format!("sqlite:{}?mode=rwc", database_path.display());

    sqlx::any::install_default_drivers();
    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("CREATE VIEW entries AS SELECT 1 AS value")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let error = match SqlxBackend::open_sqlite(&database_path).await {
        Ok(_) => panic!("the conflicting view must fail schema initialization after ownership"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("Index creation failed"),
        "expected a schema initialization failure, got {error:?}"
    );

    let pool = sqlx::any::AnyPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::query("DROP VIEW entries")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    SqlxBackend::open_sqlite(&database_path)
        .await
        .expect("failed initialization must release ownership");
}

#[tokio::test(flavor = "multi_thread")]
async fn simultaneous_sqlite_claims_have_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("simultaneous.db");
    std::fs::File::create(&database_path).unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));

    let claim = |barrier: std::sync::Arc<tokio::sync::Barrier>| {
        let database_path = database_path.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            SqlxBackend::open_sqlite(database_path).await
        })
    };
    let first = claim(barrier.clone());
    let second = claim(barrier.clone());
    barrier.wait().await;

    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(Error::Backend(error)) if matches!(**error, BackendError::StorageAlreadyOwned { .. })))
            .count(),
        1
    );
}

#[tokio::test]
async fn direct_sqlite_owner_is_exclusive_after_owner_crashes() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("crash.db");

    let mut owner = Command::new(std::env::current_exe().unwrap())
        .env(SQLITE_OWNER_HELPER_ENV, &database_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(owner.stdout.take().unwrap());
    let mut ready = String::new();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready.trim(), "EIDETICA_SQLITE_OWNER_READY");

    owner.kill().unwrap();
    let _ = owner.wait().unwrap();

    SqlxBackend::open_sqlite(&database_path)
        .await
        .expect("the operating system must release ownership after a crash");
}
