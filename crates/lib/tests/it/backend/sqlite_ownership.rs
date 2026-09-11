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

#[tokio::test(flavor = "multi_thread")]
async fn failed_sqlite_initialization_releases_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let database_path = dir.path().join("failed.db");
    std::fs::create_dir(&database_path).unwrap();

    if SqlxBackend::open_sqlite(&database_path).await.is_ok() {
        panic!("a directory must fail SQLite initialization");
    }
    std::fs::remove_dir(&database_path).unwrap();

    SqlxBackend::open_sqlite(&database_path)
        .await
        .expect("failed initialization must release ownership");
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
