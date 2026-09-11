//! SQL-based backend implementations for Eidetica storage.
//!
//! This module provides SQL database backends that implement the `BackendImpl` trait,
//! allowing Eidetica entries to be stored in relational databases.
//!
//! ## Available Backends
//!
//! - **SQLite** (feature: `sqlite`): Embedded database
//! - **PostgreSQL** (feature: `postgres`): PostgreSQL database
//!
//! ## Architecture
//!
//! The SQL backend uses sqlx with `AnyPool` for multi-database support.
//! All methods are async to match the async `BackendImpl` trait.
//!
//! ## Schema and Migrations
//!
//! The database schema is defined in the [`schema`] module and automatically
//! initialized when connecting. Migrations are handled via code-based functions
//! rather than SQL files to support dialect differences between SQLite and PostgreSQL.
//!
//! See [`schema`] module documentation for details on adding migrations.

mod storage;
mod traversal;

/// Schema definition and migration system.
pub mod schema;

use std::any::Any;
#[cfg(feature = "sqlite")]
use std::collections::HashMap;
#[cfg(feature = "sqlite")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "sqlite")]
use std::io;
#[cfg(feature = "sqlite")]
use std::path::{Path, PathBuf};
#[cfg(feature = "sqlite")]
use std::str::FromStr;
#[cfg(feature = "sqlite")]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(feature = "sqlite")]
use fs2::FileExt;
use sqlx::AnyPool;
use sqlx::Executor;
use sqlx::any::AnyPoolOptions;
#[cfg(feature = "sqlite")]
use sqlx::sqlite::SqliteConnectOptions;

use crate::Result;
use crate::backend::errors::BackendError;
use crate::backend::{
    BackendImpl, InstanceMetadata, InstanceSecrets, RecordMutations, RecordPage, RecordRange,
    RecordView, StagingToken, StoreStateRequest, VerificationStatus,
};
use crate::entry::{Entry, ID};
use crate::snapshot::Snapshot;

/// Extension trait for sqlx Result types to simplify error handling.
///
/// Similar to `anyhow::Context`, this trait adds a method to convert
/// sqlx errors to `BackendError::SqlxError` with a context message.
pub(crate) trait SqlxResultExt<T> {
    /// Convert sqlx error to BackendError with context message.
    fn sql_context(self, context: &str) -> Result<T>;
}

impl<T> SqlxResultExt<T> for std::result::Result<T, sqlx::Error> {
    fn sql_context(self, context: &str) -> Result<T> {
        self.map_err(|e| {
            BackendError::SqlxError {
                reason: format!("{context}: {e}"),
                source: Some(e),
            }
            .into()
        })
    }
}

/// Database backend kind for SQL dialect selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbKind {
    /// SQLite database
    Sqlite,
    /// PostgreSQL database
    Postgres,
}

/// SQL-based backend implementing `BackendImpl` using sqlx.
///
/// This backend supports both SQLite and PostgreSQL through sqlx's `AnyPool`.
///
/// # Concurrency
///
/// `SqlxBackend` is `Send + Sync` as required by `BackendImpl`. The underlying
/// sqlx pool handles connection pooling and thread safety. A process holds its
/// file-backed SQLite ownership until process exit; additional backends in that
/// process may use the same database, while other processes are refused.
///
/// # Test Isolation
///
/// For PostgreSQL, each backend instance can use its own schema for test isolation.
/// Use `connect_postgres_isolated()` to create an isolated backend for testing.
pub struct SqlxBackend {
    pool: AnyPool,
    kind: DbKind,
}

#[cfg(feature = "sqlite")]
static SQLITE_OWNERS: OnceLock<Mutex<HashMap<PathBuf, File>>> = OnceLock::new();

#[cfg(feature = "sqlite")]
fn claim_sqlite(url: &str) -> Result<()> {
    let options = SqliteConnectOptions::from_str(url).map_err(|error| BackendError::SqlxError {
        reason: format!("Failed to parse SQLite connection URL: {error}"),
        source: Some(error),
    })?;
    let database_path = canonical_database_path(options.get_filename()).map_err(|error| {
        BackendError::SqlxError {
            reason: format!(
                "Failed to identify SQLite database `{}`: {error}",
                options.get_filename().display()
            ),
            source: None,
        }
    })?;
    let owners = SQLITE_OWNERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut owners = owners.lock().map_err(|_| BackendError::SqlxError {
        reason: "SQLite ownership registry is unavailable".to_string(),
        source: None,
    })?;

    // More than one backend may use the database within its owning process.
    if owners.contains_key(&database_path) {
        return Ok(());
    }

    let lock_path = sqlite_lock_path(&database_path);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| BackendError::SqlxError {
            reason: format!(
                "Failed to open SQLite ownership file `{}`: {error}",
                lock_path.display()
            ),
            source: None,
        })?;

    match file.try_lock_exclusive() {
        Ok(()) => {
            owners.insert(database_path, file);
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err(BackendError::SqliteAlreadyOwned {
                path: database_path,
            }
            .into())
        }
        Err(error) => Err(BackendError::SqlxError {
            reason: format!(
                "Failed to claim SQLite ownership file `{}`: {error}",
                lock_path.display()
            ),
            source: None,
        }
        .into()),
    }
}

#[cfg(feature = "sqlite")]
fn canonical_database_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };

    if absolute.exists() {
        return absolute.canonicalize();
    }

    let file_name = absolute.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "database path has no file name",
        )
    })?;
    let parent = absolute.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "database path has no parent")
    })?;
    Ok(parent.canonicalize()?.join(file_name))
}

#[cfg(feature = "sqlite")]
fn sqlite_lock_path(database_path: &Path) -> PathBuf {
    let mut name = database_path
        .file_name()
        .expect("canonical database path has a file name")
        .to_os_string();
    name.push(".eidetica-owner");
    database_path.with_file_name(name)
}

impl SqlxBackend {
    /// Get a reference to the underlying pool.
    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    /// Get the database kind.
    pub fn kind(&self) -> DbKind {
        self.kind
    }

    /// Check if this backend is using SQLite.
    pub fn is_sqlite(&self) -> bool {
        self.kind == DbKind::Sqlite
    }

    /// Check if this backend is using PostgreSQL.
    pub fn is_postgres(&self) -> bool {
        self.kind == DbKind::Postgres
    }
}

// Test-only Store-state stage pause hook.
//
// A regression test for same-token stage-vs-publish interleavings needs to
// park a `stage_store_state_records` call after token validation (advisory
// locks held) but before any record writes, run a competing publish, then
// release the stage. Production call paths cannot pause mid-transaction, and
// a sleep-based race would be flaky by construction, so this narrow hook
// exists behind the `testing` feature only: it is compiled out of every
// production build, adds no trait surface, and is a no-op (one map miss)
// unless a test registered a gate for the exact staging namespace. Gates are
// one-shot and keyed by namespace UUID, so parallel tests cannot observe or
// disturb each other.
#[cfg(feature = "testing")]
#[derive(Debug)]
pub struct StoreStateStagePause {
    validated: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[cfg(feature = "testing")]
impl StoreStateStagePause {
    fn new() -> Self {
        Self {
            validated: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }

    /// Wait until the staged call reaches the pause point (bounded).
    ///
    /// # Panics
    ///
    /// Panics after 15 seconds: a timeout means the test never drove a stage
    /// through the gate, i.e. a harness bug, not a backend result.
    pub async fn wait_validated(&self) {
        if tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.validated.notified(),
        )
        .await
        .is_err()
        {
            panic!("Store-state stage pause gate never reached: test harness bug");
        }
    }

    /// Let the paused stage proceed to its writes.
    pub fn release(&self) {
        self.release.notify_one();
    }
}

#[cfg(feature = "testing")]
static STORE_STATE_STAGE_PAUSES: std::sync::OnceLock<
    tokio::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<StoreStateStagePause>>>,
> = std::sync::OnceLock::new();

#[cfg(feature = "testing")]
fn store_state_stage_pauses() -> &'static tokio::sync::Mutex<
    std::collections::HashMap<String, std::sync::Arc<StoreStateStagePause>>,
> {
    STORE_STATE_STAGE_PAUSES
        .get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Register a one-shot pause gate for the given staging namespace.
///
/// The next `stage_store_state_records` call for this namespace signals the
/// gate after validating its token and waits (bounded) for [`StoreStateStagePause::release`]
/// before writing any records. Returns the gate the test drives.
#[cfg(feature = "testing")]
impl SqlxBackend {
    pub async fn testing_register_stage_pause(
        namespace_id: &str,
    ) -> std::sync::Arc<StoreStateStagePause> {
        let gate = std::sync::Arc::new(StoreStateStagePause::new());
        store_state_stage_pauses()
            .lock()
            .await
            .insert(namespace_id.to_string(), gate.clone());
        gate
    }
}

/// Fire the pause gate for a namespace, if a test registered one.
///
/// Called from `stage_store_state_records` after token validation, before
/// record writes. One-shot: the gate is removed before signalling, so a late
/// duplicate stage for the same namespace proceeds unpaused.
#[cfg(feature = "testing")]
pub(crate) async fn fire_store_state_stage_pause(namespace_id: &str) {
    let gate = store_state_stage_pauses().lock().await.remove(namespace_id);
    let Some(gate) = gate else { return };
    gate.validated.notify_one();
    if tokio::time::timeout(std::time::Duration::from_secs(15), gate.release.notified())
        .await
        .is_err()
    {
        panic!(
            "Store-state stage pause gate for namespace {namespace_id} was never released: test harness bug"
        );
    }
}

// SQLite-specific implementations
#[cfg(feature = "sqlite")]
impl SqlxBackend {
    /// Open a SQLite database at the given path.
    ///
    /// Creates the database file and schema if they don't exist.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the SQLite database file
    ///
    /// # Example
    ///
    /// ```ignore
    /// use eidetica::backend::database::sql::SqlxBackend;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let backend = SqlxBackend::open_sqlite("my_database.db").await.unwrap();
    /// }
    /// ```
    pub async fn open_sqlite<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        // mode=rwc: read-write-create (create file if it doesn't exist)
        let url = format!("sqlite:{}?mode=rwc", path.as_ref().display());
        Self::connect_sqlite(&url).await
    }

    /// Connect to a SQLite database using a connection URL.
    ///
    /// # Arguments
    ///
    /// * `url` - SQLite connection URL (e.g., "sqlite:./my.db")
    pub async fn connect_sqlite(url: &str) -> Result<Self> {
        // Install any driver support
        sqlx::any::install_default_drivers();

        // Detect if this is an in-memory database. Two URL conventions
        // exist: `?mode=memory` (sqlx's explicit query flag, used by
        // `Sqlite::in_memory`) and `:memory:` (SQLite's classic magic
        // filename, embedded in URI-filename forms like
        // `sqlite:file::memory:?cache=shared`).
        let is_in_memory = url.contains("mode=memory") || url.contains(":memory:");
        if !is_in_memory {
            claim_sqlite(url)?;
        }

        // For SQLite in-memory databases with shared cache, we must prevent
        // all connections from being closed. When the last connection closes,
        // the in-memory database is destroyed and all data is lost.
        //
        // IMPORTANT: SQLite pragmas like busy_timeout and synchronous are per-connection
        // settings. We use after_connect to ensure every connection in the pool has
        // these configured, not just one.
        let pool = if is_in_memory {
            AnyPoolOptions::new()
                .max_connections(5)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
                .after_connect(|conn, _meta| {
                    Box::pin(async move {
                        // In-memory databases don't need WAL mode (all in RAM)
                        // but still need busy_timeout for lock contention
                        conn.execute("PRAGMA busy_timeout = 5000;").await?;
                        Ok(())
                    })
                })
                .connect(url)
                .await
                .sql_context("Failed to connect to SQLite")?
        } else {
            AnyPoolOptions::new()
                .max_connections(5)
                .after_connect(|conn, _meta| {
                    Box::pin(async move {
                        // File-based SQLite per-connection settings:
                        // - synchronous=NORMAL: Balanced durability (safe with WAL)
                        // - busy_timeout=5000: Wait up to 5s for locks before failing
                        //
                        // Note: journal_mode=WAL is a database-level setting that persists,
                        // so we only set it once after pool creation, not per-connection.
                        conn.execute("PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;")
                            .await?;
                        Ok(())
                    })
                })
                .connect(url)
                .await
                .sql_context("Failed to connect to SQLite")?
        };

        // Set WAL mode once (database-level setting that persists in the file)
        if !is_in_memory {
            sqlx::query("PRAGMA journal_mode = WAL;")
                .execute(&pool)
                .await
                .sql_context("Failed to set SQLite WAL mode")?;
        }

        let backend = Self {
            pool,
            kind: DbKind::Sqlite,
        };

        // Initialize schema
        schema::initialize(&backend).await?;

        Ok(backend)
    }

    /// Create an in-memory SQLite database (async).
    ///
    /// The database exists only for the lifetime of this backend instance.
    /// Useful for testing.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use eidetica::backend::database::sql::SqlxBackend;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let backend = SqlxBackend::sqlite_in_memory().await.unwrap();
    /// }
    /// ```
    pub async fn sqlite_in_memory() -> Result<Self> {
        // Use shared cache mode for in-memory SQLite so all connections in the pool
        // share the same database. Without this, each connection gets its own
        // isolated in-memory database.
        // Use a unique name per instance to avoid sharing between tests.
        let unique_id = uuid::Uuid::new_v4();
        let url = format!("sqlite:file:mem_{unique_id}?mode=memory&cache=shared");
        Self::connect_sqlite(&url).await
    }
}

// PostgreSQL-specific implementations
#[cfg(feature = "postgres")]
impl SqlxBackend {
    /// Connect to a PostgreSQL database using a connection URL.
    ///
    /// This connects to the default (public) schema. For test isolation,
    /// use `connect_postgres_isolated()` instead.
    ///
    /// # Arguments
    ///
    /// * `url` - PostgreSQL connection URL (e.g., "postgres://user:pass@localhost/dbname")
    ///
    /// # Example
    ///
    /// ```ignore
    /// use eidetica::backend::database::sql::SqlxBackend;
    ///
    /// let backend = SqlxBackend::connect_postgres("postgres://localhost/eidetica").await.unwrap();
    /// ```
    pub async fn connect_postgres(url: &str) -> Result<Self> {
        Self::connect_postgres_with_schema(url, None).await
    }

    /// Connect to a PostgreSQL database with a specific schema for isolation.
    ///
    /// Creates a unique schema if `schema_name` is provided, providing test isolation.
    /// Each test can use its own schema so they don't interfere with each other.
    ///
    /// # Arguments
    ///
    /// * `url` - PostgreSQL connection URL
    /// * `schema_name` - Optional schema name. If None, uses the default (public) schema.
    async fn connect_postgres_with_schema(url: &str, schema_name: Option<String>) -> Result<Self> {
        // Install any driver support
        sqlx::any::install_default_drivers();

        // If schema_name is provided, first create the schema, then use after_connect
        // to set search_path on each connection. This is more reliable than URL options
        // which don't work consistently across all network configurations.
        if let Some(ref schema) = schema_name {
            // First connect to create the schema if needed
            let temp_pool = AnyPoolOptions::new()
                .max_connections(1)
                .connect(url)
                .await
                .sql_context("Failed to connect to PostgreSQL")?;

            // Create schema if it doesn't exist
            let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {schema}");
            sqlx::query(&create_schema)
                .execute(&temp_pool)
                .await
                .sql_context(&format!("Failed to create schema {schema}"))?;

            temp_pool.close().await;
        }

        // Build pool with after_connect hook to set search_path on each connection
        // For isolated (test) connections, use smaller pool to avoid exhausting
        // PostgreSQL's max_connections when running many tests in parallel.
        let schema_for_hook = schema_name.clone();
        let is_isolated = schema_name.is_some();
        let mut pool_options = AnyPoolOptions::new();

        if is_isolated {
            // Test isolation: 2 connections is enough, with longer timeout to wait
            // rather than fail when many tests run in parallel
            pool_options = pool_options
                .max_connections(2)
                .acquire_timeout(Duration::from_secs(30));
        } else {
            // Production: 5 connections for real concurrency needs
            pool_options = pool_options.max_connections(5);
        }

        let pool = pool_options
            .after_connect(move |conn, _meta| {
                let schema = schema_for_hook.clone();
                Box::pin(async move {
                    if let Some(ref s) = schema {
                        let set_path = format!("SET search_path TO {s}");
                        conn.execute(set_path.as_str()).await?;
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await
            .sql_context("Failed to connect to PostgreSQL")?;

        let backend = Self {
            pool,
            kind: DbKind::Postgres,
        };

        // Initialize schema (tables will be created in the current search_path)
        schema::initialize(&backend).await?;

        Ok(backend)
    }

    /// Connect to a PostgreSQL database with test isolation.
    ///
    /// Creates a unique schema for this backend instance, ensuring tests
    /// don't interfere with each other when run in parallel.
    ///
    /// # Arguments
    ///
    /// * `url` - PostgreSQL connection URL (e.g., "postgres://user:pass@localhost/dbname")
    ///
    /// # Example
    ///
    /// ```ignore
    /// use eidetica::backend::database::sql::SqlxBackend;
    ///
    /// let backend = SqlxBackend::connect_postgres_isolated("postgres://localhost/eidetica").await.unwrap();
    /// // This backend uses its own isolated schema
    /// ```
    pub async fn connect_postgres_isolated(url: &str) -> Result<Self> {
        // Generate a unique schema name using UUID
        // PostgreSQL schema names must start with a letter and be lowercase
        let unique_id = uuid::Uuid::new_v4().simple().to_string();
        let schema_name = format!("test_{unique_id}");
        Self::connect_postgres_with_schema(url, Some(schema_name)).await
    }
}

#[async_trait]
impl BackendImpl for SqlxBackend {
    async fn resolve_store_state(&self, request: &StoreStateRequest) -> Result<Option<RecordView>> {
        storage::resolve_store_state(self, request).await
    }

    async fn begin_store_state_staging(&self, request: StoreStateRequest) -> Result<StagingToken> {
        storage::begin_store_state_staging(self, request).await
    }

    async fn stage_store_state_records(
        &self,
        token: &StagingToken,
        records: RecordMutations,
    ) -> Result<()> {
        storage::stage_store_state_records(self, token, records).await
    }

    async fn publish_store_state(&self, token: StagingToken) -> Result<RecordView> {
        storage::publish_store_state(self, token).await
    }

    async fn abort_store_state(&self, token: StagingToken) -> Result<()> {
        storage::abort_store_state(self, token).await
    }

    async fn store_state_record_get(
        &self,
        view: &RecordView,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        storage::store_state_record_get(self, view, key).await
    }

    async fn store_state_record_scan(
        &self,
        view: &RecordView,
        range: &RecordRange,
        after: Option<&[u8]>,
        limit: usize,
    ) -> Result<RecordPage> {
        storage::store_state_record_scan(self, view, range, after, limit).await
    }

    async fn clear_derived_store_state(&self) -> Result<()> {
        storage::clear_derived_store_state(self).await
    }
    async fn get(&self, id: &ID) -> Result<Entry> {
        storage::get(self, id).await
    }

    async fn get_verification_status(&self, id: &ID) -> Result<VerificationStatus> {
        storage::get_verification_status(self, id).await
    }

    async fn put(&self, entry: Entry) -> Result<()> {
        storage::put(self, entry).await
    }

    async fn update_verification_status(
        &self,
        id: &ID,
        verification_status: VerificationStatus,
    ) -> Result<()> {
        storage::update_verification_status(self, id, verification_status).await
    }

    async fn get_entries_by_verification_status(
        &self,
        status: VerificationStatus,
    ) -> Result<Vec<ID>> {
        storage::get_entries_by_verification_status(self, status).await
    }

    async fn snapshot(&self, tree: &ID) -> Result<Snapshot> {
        traversal::snapshot(self, tree).await.map(Snapshot::new)
    }

    async fn store_snapshot(&self, tree: &ID, store: &str) -> Result<Snapshot> {
        traversal::store_snapshot(self, tree, store)
            .await
            .map(Snapshot::new)
    }

    async fn store_snapshot_at(
        &self,
        tree: &ID,
        store: &str,
        main_snapshot: &Snapshot,
    ) -> Result<Snapshot> {
        traversal::store_snapshot_at(self, tree, store, main_snapshot.tips())
            .await
            .map(Snapshot::new)
    }

    async fn all_roots(&self) -> Result<Vec<ID>> {
        storage::all_roots(self).await
    }

    async fn find_merge_base(
        &self,
        tree: &ID,
        store: &str,
        entry_ids: &[ID],
    ) -> Result<Option<ID>> {
        traversal::find_merge_base(self, tree, store, entry_ids).await
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    async fn get_tree(&self, tree: &ID) -> Result<Vec<Entry>> {
        storage::get_tree(self, tree).await
    }

    async fn get_store(&self, tree: &ID, store: &str) -> Result<Vec<Entry>> {
        storage::get_store(self, tree, store).await
    }

    async fn get_tree_from_tips(&self, tree: &ID, tips: &[ID]) -> Result<Vec<Entry>> {
        traversal::get_tree_from_tips(self, tree, tips).await
    }

    async fn store_at(&self, tree: &ID, store: &str, snapshot: &Snapshot) -> Result<Vec<Entry>> {
        traversal::store_at(self, tree, store, snapshot.tips()).await
    }

    async fn get_sorted_store_parents(
        &self,
        tree_id: &ID,
        entry_id: &ID,
        store: &str,
    ) -> Result<Vec<ID>> {
        traversal::get_sorted_store_parents(self, tree_id, entry_id, store).await
    }

    async fn get_path_from_to(
        &self,
        tree_id: &ID,
        store: &str,
        from_id: Option<&ID>,
        to_ids: &[ID],
    ) -> Result<Vec<ID>> {
        traversal::get_path_from_to(self, tree_id, store, from_id, to_ids).await
    }

    async fn get_instance_metadata(&self) -> Result<Option<InstanceMetadata>> {
        storage::get_instance_metadata(self).await
    }

    async fn set_instance_metadata(&self, metadata: &InstanceMetadata) -> Result<()> {
        storage::set_instance_metadata(self, metadata).await
    }

    async fn get_instance_secrets(&self) -> Result<Option<InstanceSecrets>> {
        storage::get_instance_secrets(self).await
    }

    async fn set_instance_secrets(&self, secrets: &InstanceSecrets) -> Result<()> {
        storage::set_instance_secrets(self, secrets).await
    }
}

/// Namespace for SQLite database constructors.
///
/// Provides ergonomic factory methods for creating SQLite-backed storage.
/// All methods return `SqlxBackend` which implements `BackendImpl`.
///
/// # Example
///
/// ```ignore
/// use eidetica::backend::database::Sqlite;
///
/// // File-based storage
/// let backend = Sqlite::open("my_data.db").await?;
///
/// // In-memory (for testing)
/// let backend = Sqlite::in_memory().await?;
/// ```
#[cfg(feature = "sqlite")]
pub struct Sqlite;

#[cfg(feature = "sqlite")]
impl Sqlite {
    /// Open a SQLite database at the given path.
    ///
    /// Creates the database file and schema if they don't exist.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the SQLite database file
    pub async fn open<P: AsRef<std::path::Path>>(path: P) -> Result<SqlxBackend> {
        SqlxBackend::open_sqlite(path).await
    }

    /// Create an in-memory SQLite database.
    ///
    /// The database exists only for the lifetime of the returned backend.
    /// Useful for testing.
    pub async fn in_memory() -> Result<SqlxBackend> {
        SqlxBackend::sqlite_in_memory().await
    }

    /// Connect to a SQLite database using a connection URL.
    ///
    /// # Arguments
    ///
    /// * `url` - SQLite connection URL (e.g., "sqlite:./my.db")
    pub async fn connect(url: &str) -> Result<SqlxBackend> {
        SqlxBackend::connect_sqlite(url).await
    }
}

/// Namespace for PostgreSQL database constructors.
///
/// Provides ergonomic factory methods for creating PostgreSQL-backed storage.
/// All methods return `SqlxBackend` which implements `BackendImpl`.
///
/// # Example
///
/// ```ignore
/// use eidetica::backend::database::Postgres;
///
/// // Connect to PostgreSQL
/// let backend = Postgres::connect("postgres://user:pass@localhost/mydb").await?;
///
/// // With test isolation (unique schema per instance)
/// let backend = Postgres::connect_isolated("postgres://localhost/test").await?;
/// ```
#[cfg(feature = "postgres")]
pub struct Postgres;

#[cfg(feature = "postgres")]
impl Postgres {
    /// Connect to a PostgreSQL database using a connection URL.
    ///
    /// This connects to the default (public) schema. For test isolation,
    /// use `connect_isolated()` instead.
    ///
    /// # Arguments
    ///
    /// * `url` - PostgreSQL connection URL (e.g., "postgres://user:pass@localhost/dbname")
    pub async fn connect(url: &str) -> Result<SqlxBackend> {
        SqlxBackend::connect_postgres(url).await
    }

    /// Connect to a PostgreSQL database with test isolation.
    ///
    /// Creates a unique schema for this backend instance, ensuring tests
    /// don't interfere with each other when run in parallel.
    ///
    /// # Arguments
    ///
    /// * `url` - PostgreSQL connection URL
    pub async fn connect_isolated(url: &str) -> Result<SqlxBackend> {
        SqlxBackend::connect_postgres_isolated(url).await
    }
}

/// A publish carrying a token whose target does not match the namespace it
/// names must never disturb a ready namespace.
///
/// Unlike `InMemory` (which resolves by target and adopts the ready winner),
/// the SQL publish names the namespace: the `UPDATE` only flips
/// lifecycle/status and keeps the begin-time identity, using `target` for
/// locking and failure-path winner adoption. Either way the ready snapshot
/// and its records always survive a malformed clone.
#[cfg(all(test, feature = "sqlite"))]
mod store_state_token_tests {
    use std::collections::BTreeMap;

    use super::Sqlite;
    use crate::backend::{
        BackendImpl, CacheScope, ProjectionDescriptor, StagingToken, StoreStateLifecycle,
        StoreStateRequest,
    };
    use crate::entry::ID;

    fn request(store: &str) -> StoreStateRequest {
        StoreStateRequest {
            database: ID::from_bytes("db"),
            store: store.to_string(),
            lifecycle: StoreStateLifecycle::Derived,
            scope: CacheScope::Shared,
            projection: ProjectionDescriptor {
                name: "test/opaque".to_string(),
                version: 0,
            },
            source_key: b"snapshot".to_vec(),
        }
    }

    #[tokio::test]
    async fn mismatched_target_publish_preserves_ready_namespace() {
        let backend = Sqlite::in_memory().await.unwrap();

        // Ready namespace for target A.
        let request_a = request("store-a");
        let token_a = backend
            .begin_store_state_staging(request_a.clone())
            .await
            .unwrap();
        backend
            .stage_store_state_records(
                &token_a,
                BTreeMap::from([(b"key".to_vec(), Some(b"value-a".to_vec()))]),
            )
            .await
            .unwrap();
        let view_a = backend.publish_store_state(token_a).await.unwrap();

        // Staging namespace for target B.
        let request_b = request("store-b");
        let token_b = backend
            .begin_store_state_staging(request_b.clone())
            .await
            .unwrap();
        backend
            .stage_store_state_records(
                &token_b,
                BTreeMap::from([(b"key".to_vec(), Some(b"value-b".to_vec()))]),
            )
            .await
            .unwrap();

        // Malformed clone: B's namespace id, A's target. The SQL publish names
        // the namespace (the UPDATE only flips lifecycle/status and keeps the
        // begin-time identity; `target` drives locking and winner adoption),
        // so B becomes ready under its own identity while ready A is
        // untouched: no ready snapshot is ever modified by a mismatched token.
        let bad = StagingToken {
            namespace_id: token_b.namespace_id.clone(),
            target: request_a.clone(),
        };
        let published_b = backend.publish_store_state(bad).await.unwrap();
        assert_eq!(published_b.namespace_id, token_b.namespace_id);
        assert_eq!(
            backend.resolve_store_state(&request_b).await.unwrap(),
            Some(published_b.clone())
        );
        assert_eq!(
            backend
                .store_state_record_get(&published_b, b"key")
                .await
                .unwrap(),
            Some(b"value-b".to_vec())
        );
        assert_eq!(
            backend.resolve_store_state(&request_a).await.unwrap(),
            Some(view_a.clone())
        );
        assert_eq!(
            backend
                .store_state_record_get(&view_a, b"key")
                .await
                .unwrap(),
            Some(b"value-a".to_vec())
        );

        // Malformed clone: A's (ready) namespace id with a target that
        // resolves nowhere. The publish fails and the ready row survives the
        // guarded discard with its records intact.
        let nowhere = request("store-nowhere");
        let bad_ready = StagingToken {
            namespace_id: view_a.namespace_id.clone(),
            target: nowhere.clone(),
        };
        assert!(backend.publish_store_state(bad_ready).await.is_err());
        assert_eq!(
            backend.resolve_store_state(&request_a).await.unwrap(),
            Some(view_a.clone())
        );
        assert_eq!(
            backend
                .store_state_record_get(&view_a, b"key")
                .await
                .unwrap(),
            Some(b"value-a".to_vec())
        );
        assert_eq!(backend.resolve_store_state(&nowhere).await.unwrap(), None);
    }
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_postgres_initialization_releases_ownership() {
        if std::env::var("TEST_BACKEND").as_deref() != Ok("postgres") {
            return;
        }

        let url = std::env::var("TEST_POSTGRES_URL")
            .unwrap_or_else(|_| "postgres://localhost/eidetica_test".to_string());
        sqlx::any::install_default_drivers();
        let schema = format!("test_{}", uuid::Uuid::new_v4().simple());
        let setup = AnyPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&setup)
            .await
            .unwrap();
        sqlx::query(&format!(
            "CREATE VIEW {schema}.entries AS SELECT 1 AS value"
        ))
        .execute(&setup)
        .await
        .unwrap();

        assert!(
            SqlxBackend::connect_postgres_with_schema(&url, Some(schema.clone()))
                .await
                .is_err(),
            "the conflicting view must make schema initialization fail"
        );
        sqlx::query(&format!("DROP VIEW {schema}.entries"))
            .execute(&setup)
            .await
            .unwrap();

        SqlxBackend::connect_postgres_with_schema(&url, Some(schema.clone()))
            .await
            .expect("failed initialization must release PostgreSQL ownership");
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&setup)
            .await
            .unwrap();
        setup.close().await;
    }
}
