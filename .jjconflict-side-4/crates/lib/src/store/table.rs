use std::marker::PhantomData;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    Result, Store, Transaction,
    store::{Registered, errors::StoreError},
};

// =============================================================================
// Row Operation Types
// =============================================================================

/// A single row operation within a Table entry.
///
/// Table entries store a list of row operations rather than the full table state.
/// This enables incremental cache updates and supports tables larger than memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableRowOp {
    /// The UUID primary key of the affected row
    pub uuid: String,
    /// The operation performed on this row
    pub kind: RowOpKind,
}

/// The kind of operation performed on a table row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RowOpKind {
    /// Set the row to a new value (insert or update)
    Set {
        /// Serialized row data as JSON
        data: String,
    },
    /// Delete the row (tombstone)
    Delete,
}

impl TableRowOp {
    /// Create a Set operation for a row
    pub fn set(uuid: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            uuid: uuid.into(),
            kind: RowOpKind::Set { data: data.into() },
        }
    }

    /// Create a Delete operation for a row
    pub fn delete(uuid: impl Into<String>) -> Self {
        Self {
            uuid: uuid.into(),
            kind: RowOpKind::Delete,
        }
    }
}

// =============================================================================
// Table Store
// =============================================================================

/// A Row-based Store
///
/// `Table` provides a record-oriented storage abstraction for entries in a subtree,
/// similar to a database table with automatic primary key generation.
///
/// # Features
/// - Automatically generates UUIDv4 primary keys for new records
/// - Provides CRUD operations (Create, Read, Update, Delete) for record-based data
/// - Supports searching across all records with a predicate function
///
/// # Type Parameters
/// - `T`: The record type to be stored, which must be serializable, deserializable, and cloneable
///
/// This abstraction simplifies working with collections of similarly structured data
/// by handling the details of:
/// - Primary key generation and management
/// - Serialization/deserialization of records
/// - Storage within the underlying CRDT (Doc)
pub struct Table<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone,
{
    name: String,
    atomic_op: Transaction,
    phantom: PhantomData<T>,
}

impl<T> Registered for Table<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone,
{
    fn type_id() -> &'static str {
        "table:v0"
    }
}

#[async_trait]
impl<T> Store for Table<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync,
{
    async fn new(op: &Transaction, subtree_name: String) -> Result<Self> {
        Ok(Self {
            name: subtree_name,
            atomic_op: op.clone(),
            phantom: PhantomData,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn transaction(&self) -> &Transaction {
        &self.atomic_op
    }
}

impl<T> Table<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone,
{
    /// Retrieves a row from the Table by its primary key.
    ///
    /// This method uses O(1) cache lookups when possible:
    /// 1. First checks local staged row operations in the transaction
    /// 2. If cache is valid, fetches only the requested row from cache
    /// 3. If cache needs rebuilding, rebuilds it then fetches the row
    /// 4. Falls back to walking entries for historical transactions
    ///
    /// # Arguments
    /// * `key` - The primary key (UUID string) of the record to retrieve
    ///
    /// # Returns
    /// * `Ok(T)` - The retrieved record if found
    /// * `Err(Error::NotFound)` - If no record exists with the given key
    ///
    /// # Errors
    /// Returns an error if:
    /// * The record doesn't exist (`Error::NotFound`)
    /// * There's a serialization/deserialization error
    pub async fn get(&self, key: impl AsRef<str>) -> Result<T> {
        let key = key.as_ref();

        // Check local staged row operations first (reverse order - last op wins)
        let local_ops = self.atomic_op.get_table_ops(&self.name);
        for op in local_ops.iter().rev() {
            if op.uuid == key {
                return match &op.kind {
                    RowOpKind::Set { data } => {
                        serde_json::from_str(data.as_str()).map_err(|e| {
                            StoreError::DeserializationFailed {
                                store: self.name.clone(),
                                reason: format!(
                                    "Failed to deserialize record for key '{key}': {e}"
                                ),
                            }
                            .into()
                        })
                    }
                    RowOpKind::Delete => Err(StoreError::KeyNotFound {
                        store: self.name.clone(),
                        key: key.to_string(),
                    }
                    .into()),
                };
            }
        }

        // Try to use cache for O(1) single-row lookup
        // ensure_table_cache_valid returns true if cache is usable (rebuilt if needed)
        // returns false only for historical transactions
        if self.atomic_op.ensure_table_cache_valid(&self.name).await? {
            // Cache is valid - fetch single row directly
            match self.atomic_op.get_single_cached_row(&self.name, key).await? {
                Some(row) => {
                    if row.is_tombstone {
                        return Err(StoreError::KeyNotFound {
                            store: self.name.clone(),
                            key: key.to_string(),
                        }
                        .into());
                    }
                    return serde_json::from_str(&row.data).map_err(|e| {
                        StoreError::DeserializationFailed {
                            store: self.name.clone(),
                            reason: format!("Failed to deserialize record for key '{key}': {e}"),
                        }
                        .into()
                    });
                }
                None => {
                    return Err(StoreError::KeyNotFound {
                        store: self.name.clone(),
                        key: key.to_string(),
                    }
                    .into());
                }
            }
        }

        // Fall back to walking entries for historical transactions
        // Uses get_full_state_cached which handles historical reads via build_state_from_row_ops
        let doc = self.atomic_op.get_full_state_cached(&self.name).await?;
        if let Some(value) = doc.get(key)
            && let Some(text) = value.as_text() {
                return serde_json::from_str(text).map_err(|e| {
                    StoreError::DeserializationFailed {
                        store: self.name.clone(),
                        reason: format!("Failed to deserialize record for key '{key}': {e}"),
                    }
                    .into()
                });
            }
        Err(StoreError::KeyNotFound {
            store: self.name.clone(),
            key: key.to_string(),
        }
        .into())
    }

    /// Inserts a new row into the Table and returns its generated primary key.
    ///
    /// This method:
    /// 1. Generates a new UUIDv4 as the primary key
    /// 2. Serializes the record
    /// 3. Stages a row operation in the transaction
    ///
    /// # Arguments
    /// * `row` - The record to insert
    ///
    /// # Returns
    /// * `Ok(String)` - The generated UUID primary key as a string
    ///
    /// # Errors
    /// Returns an error if there's a serialization error or the operation fails
    pub async fn insert(&self, row: T) -> Result<String> {
        // Generate a UUIDv4 for the primary key
        let primary_key = Uuid::new_v4().to_string();

        // Serialize the row
        let serialized_row =
            serde_json::to_string(&row).map_err(|e| StoreError::SerializationFailed {
                store: self.name.clone(),
                reason: format!("Failed to serialize record: {e}"),
            })?;

        // Stage the row operation
        let op = TableRowOp::set(&primary_key, serialized_row);
        self.atomic_op.stage_table_op(&self.name, op).await?;

        // Return the primary key
        Ok(primary_key)
    }

    /// Updates an existing row in the Table with a new value.
    ///
    /// This method completely replaces the existing record with the provided one.
    /// If the record doesn't exist yet, it will be created with the given key.
    ///
    /// # Arguments
    /// * `key` - The primary key of the record to update
    /// * `row` - The new record value
    ///
    /// # Returns
    /// * `Ok(())` - If the update was successful
    ///
    /// # Errors
    /// Returns an error if there's a serialization error or the operation fails
    pub async fn set(&self, key: impl AsRef<str>, row: T) -> Result<()> {
        let key_str = key.as_ref();

        // Serialize the row
        let serialized_row =
            serde_json::to_string(&row).map_err(|e| StoreError::SerializationFailed {
                store: self.name.clone(),
                reason: format!("Failed to serialize record for key '{key_str}': {e}"),
            })?;

        // Stage the row operation
        let op = TableRowOp::set(key_str, serialized_row);
        self.atomic_op.stage_table_op(&self.name, op).await
    }

    /// Deletes a row from the Table by its primary key.
    ///
    /// This method marks the record as deleted using a tombstone operation,
    /// ensuring the deletion is properly synchronized across distributed nodes.
    ///
    /// # Arguments
    /// * `key` - The primary key of the record to delete
    ///
    /// # Returns
    /// * `Ok(true)` - If a record existed and was deleted
    /// * `Ok(false)` - If no record existed with the given key
    ///
    /// # Errors
    /// Returns an error if there's a serialization error or the operation fails
    pub async fn delete(&self, key: impl AsRef<str>) -> Result<bool> {
        let key_str = key.as_ref();

        // Check if the record exists (checks both local and full state)
        let exists = self.get(key_str).await.is_ok();

        // If the record doesn't exist, return false early
        if !exists {
            return Ok(false);
        }

        // Stage the delete operation
        let op = TableRowOp::delete(key_str);
        self.atomic_op.stage_table_op(&self.name, op).await?;

        // Return true since we confirmed the record existed
        Ok(true)
    }

    /// Searches for rows matching a predicate function.
    ///
    /// # Arguments
    /// * `query` - A function that takes a reference to a record and returns a boolean
    ///
    /// # Returns
    /// * `Ok(Vec<(String, T)>)` - A vector of (primary_key, record) pairs that match the predicate
    ///
    /// # Errors
    /// Returns an error if there's a serialization error or the operation fails
    pub async fn search(&self, query: impl Fn(&T) -> bool) -> Result<Vec<(String, T)>> {
        let mut result = Vec::new();

        // Get the full state from the backend (using cache if available)
        let mut data = self.atomic_op.get_full_state_cached(&self.name).await?;

        // Apply in-transaction row operations to the cached state
        // This ensures search() sees uncommitted changes (like get() does)
        let local_ops = self.atomic_op.get_table_ops(&self.name);
        for op in &local_ops {
            match &op.kind {
                RowOpKind::Set { data: row_data } => {
                    data.set(&op.uuid, row_data.clone());
                }
                RowOpKind::Delete => {
                    data.remove(&op.uuid);
                }
            }
        }

        // Iterate through all key-value pairs
        for (key, map_value) in data.iter() {
            // Skip non-text values
            if let Some(value) = map_value.as_text() {
                // Deserialize the row
                let row: T =
                    serde_json::from_str(value).map_err(|e| StoreError::DeserializationFailed {
                        store: self.name.clone(),
                        reason: format!(
                            "Failed to deserialize record for key '{key}' during search: {e}"
                        ),
                    })?;

                // Check if the row matches the query
                if query(&row) {
                    result.push((key.clone(), row));
                }
            }
        }

        Ok(result)
    }
}
