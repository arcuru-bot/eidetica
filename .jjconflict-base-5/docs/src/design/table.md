# Table Store

> **Status: Implemented**
>
> Core functionality is complete including proper LWW semantics with embedded
> subtree heights in cached row metadata.

The Table store is a row-oriented data store with O(1) single-row lookups, incremental cache updates, and row-level Last-Write-Wins (LWW) conflict resolution. It stores individual row operations in entries rather than full serialized documents, enabling efficient reads from tables of any size without loading the entire table into memory.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Data Model](#data-model)
- [Read Path](#read-path)
- [Write Path](#write-path)
- [Caching Infrastructure](#caching-infrastructure)
- [Cache Maintenance](#cache-maintenance)
- [Cache Rebuild Coordination](#cache-rebuild-coordination)
- [Conflict Resolution](#conflict-resolution)
- [Performance Characteristics](#performance-characteristics)
- [Implementation Status](#implementation-status)
- [Future Considerations](#future-considerations)

## Overview

Table provides a record-oriented storage abstraction similar to a database table with automatic UUID primary key generation. Unlike Doc-based stores that serialize all data into a CRDT document, Table treats each row as an independent unit with its own LWW semantics.

**Key Properties:**

- **O(1) single-row lookups**: `get(key)` fetches one row from cache, not the full table
- **Incremental cache updates**: Tip changes apply only diff entries, not full rebuild
- **Memory efficient**: No need to load full table state into memory
- **Row-level LWW**: Conflicts resolved per-row using `(height, entry_id)` comparison

**API:**

```rust,ignore
// Get a typed store handle
let users: Table<User> = tx.get_store("users").await?;

// CRUD operations
let id = users.insert(User { name: "Alice".into() }).await?;
let user = users.get(&id).await?;
users.set(&id, User { name: "Alicia".into() }).await?;
users.delete(&id).await?;

// Search with predicate
let results = users.search(|u| u.name.starts_with("A")).await?;
```

## Architecture

### Component Diagram

```text
┌─────────────────────────────────────────────────────────────────────────────┐
│                             Table<T>                                        │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │  get(key)  │  insert(row)  │  set(key,row)  │  delete(key)  │  search  │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────┬─────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Transaction                                       │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  table_row_ops: HashMap<store, Vec<TableRowOp>>                      │   │
│  │  ─────────────────────────────────────────────────────────────────── │   │
│  │  stage_table_op()          - Stage a row operation                   │   │
│  │  get_table_ops()           - Get staged ops for in-tx reads          │   │
│  │  ensure_table_cache_valid() - Validate/rebuild cache                 │   │
│  │  get_single_cached_row()   - O(1) cache lookup                       │   │
│  │  get_full_state_cached()   - Get Doc with cache                      │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
└───────────────────────────────────┬─────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Backend                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │  cache_rebuild_coordinator: Arc<CacheRebuildCoordinator>             │   │
│  │  ─────────────────────────────────────────────────────────────────── │   │
│  │  get_cached_row()          - Fetch single cached row                 │   │
│  │  upsert_cached_row()       - Update cached row                       │   │
│  │  get_all_cached_rows()     - Fetch all cached rows (for Doc)         │   │
│  │  clear_cached_rows()       - Clear cache for rebuild                 │   │
│  │  get_cached_tips()         - Get tips cache was built from           │   │
│  │  set_cached_tips()         - Update cached tips after rebuild        │   │
│  │  get_entries_between_tips() - Compute diff for incremental update   │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Data Flow

**Read (warm cache):**
```text
Table::get(key)
    → Check staged ops (in-transaction changes)
    → Check cache validity (tips match?)
    → get_single_cached_row(key) → O(1) SQL lookup
    → Deserialize and return
```

**Read (cold/stale cache):**
```text
Table::get(key)
    → Check staged ops
    → Cache invalid (tips don't match)
    → Coordinate rebuild (prevent races)
    → build_table_cache_from_entries() or apply_cache_diff()
    → get_single_cached_row(key)
    → Deserialize and return
```

**Write:**
```text
Table::set(key, value)
    → Serialize value to JSON
    → Create TableRowOp::Set
    → stage_table_op() → add to table_row_ops map
    → update_subtree() → serialize ops list to entry builder
    → (on commit) → Entry created with ops in SubTreeNode.data
```

## Data Model

### Entry Format

Table entries store a list of row operations in `SubTreeNode.data`:

```rust,ignore
/// A single row operation within a Table entry.
pub struct TableRowOp {
    /// The UUID primary key of the affected row
    pub uuid: String,
    /// The operation performed on this row
    pub kind: RowOpKind,
}

/// The kind of operation performed on a table row.
pub enum RowOpKind {
    /// Set the row to a new value (insert or update)
    Set { data: String },  // Serialized row JSON
    /// Delete the row (tombstone)
    Delete,
}
```

Entry structure:

```text
Entry {
    tree: TreeNode { height: 5, parents: [...], ... },
    subtrees: [
        SubTreeNode {
            name: "users",
            parents: [...],          // Subtree parents
            data: "[                  // Serialized Vec<TableRowOp>
                {\"uuid\":\"abc\",\"kind\":{\"Set\":{\"data\":\"{...}\"}}},
                {\"uuid\":\"def\",\"kind\":\"Delete\"}
            ]",
        }
    ],
    sig: ...,
}
```

### Cached Row

Materialized row state with LWW metadata:

```rust,ignore
/// A cached row from a Table store.
pub struct CachedRow {
    /// Serialized row data (JSON)
    pub data: String,
    /// Whether this row is deleted (tombstone)
    pub is_tombstone: bool,
    /// The Entry ID that last modified this row
    pub last_modified_entry_id: ID,
    /// Height of the last modifying entry
    pub last_modified_height: usize,
}
```

The `last_modified_entry_id` and `last_modified_height` fields enable LWW conflict resolution during incremental cache updates.

## Read Path

### Current Tips Read (Common Case)

```rust,ignore
pub async fn get(&self, key: impl AsRef<str>) -> Result<T> {
    let key = key.as_ref();

    // 1. Check local staged row operations first (reverse order - last op wins)
    let local_ops = self.atomic_op.get_table_ops(&self.name);
    for op in local_ops.iter().rev() {
        if op.uuid == key {
            return match &op.kind {
                RowOpKind::Set { data } => serde_json::from_str(data),
                RowOpKind::Delete => Err(KeyNotFound),
            };
        }
    }

    // 2. Try to use cache for O(1) single-row lookup
    if self.atomic_op.ensure_table_cache_valid(&self.name).await? {
        // Cache is valid - fetch single row directly
        match self.atomic_op.get_single_cached_row(&self.name, key).await? {
            Some(row) if !row.is_tombstone => {
                return serde_json::from_str(&row.data);
            }
            _ => return Err(KeyNotFound),
        }
    }

    // 3. Fall back to walking entries for historical transactions
    let doc = self.atomic_op.get_full_state_cached(&self.name).await?;
    // ... extract from doc
}
```

### Historical Read

For reads at non-current tips (historical transactions), the cache is bypassed and entries are walked directly:

```rust,ignore
pub async fn build_state_from_row_ops(&self, subtree_name: &str, tips: &[ID]) -> Result<Doc> {
    let entries = backend.get_store_from_tips(tree_id, subtree_name, tips).await?;

    let mut doc = Doc::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Process in reverse order (highest height first) - first occurrence wins
    for entry in entries.into_iter().rev() {
        let ops: Vec<TableRowOp> = deserialize(entry.data(subtree_name))?;
        for op in ops {
            if seen.contains(&op.uuid) { continue; }
            seen.insert(op.uuid.clone());

            match &op.kind {
                RowOpKind::Set { data } => doc.set(&op.uuid, data.clone()),
                RowOpKind::Delete => doc.remove(&op.uuid),
            }
        }
    }
    Ok(doc)
}
```

## Write Path

Writes stage row operations in the transaction, which are committed as an entry:

```rust,ignore
pub async fn set(&self, key: impl AsRef<str>, row: T) -> Result<()> {
    let key_str = key.as_ref();
    let serialized_row = serde_json::to_string(&row)?;

    // Stage the row operation
    let op = TableRowOp::set(key_str, serialized_row);
    self.atomic_op.stage_table_op(&self.name, op).await
}

pub async fn delete(&self, key: impl AsRef<str>) -> Result<bool> {
    // Check if record exists first
    let exists = self.get(key_str).await.is_ok();
    if !exists { return Ok(false); }

    // Stage the delete operation
    let op = TableRowOp::delete(key_str);
    self.atomic_op.stage_table_op(&self.name, op).await?;
    Ok(true)
}
```

The `stage_table_op()` method:

1. Adds the operation to `table_row_ops: HashMap<store, Vec<TableRowOp>>`
2. Serializes all ops for this store to JSON
3. Updates the subtree data in the entry builder

On commit, the entry is created with the serialized ops list in `SubTreeNode.data`. The cache is **not** updated on write - it updates lazily on the next read.

## Caching Infrastructure

### Cache Validity Model

The cache is a materialized view of CRDT state at a specific set of tips.

**Key Insight**: CRDT state is fully determined by the tips. Two databases with identical tips have identical state.

**Validity Check**:

```text
cached_tips == current_tips  →  Cache is valid
cached_tips != current_tips  →  Cache is stale, update needed
```

This is an exact set comparison (sorted). If any tip differs, the cache needs updating.

### Backend Cache Methods

```rust,ignore
// Single row operations
async fn get_cached_row(&self, tree: &ID, store: &str, uuid: &str)
    -> Result<Option<CachedRow>>;
async fn upsert_cached_row(&self, tree: &ID, store: &str, uuid: &str, row: &CachedRow)
    -> Result<()>;

// Bulk operations
async fn get_all_cached_rows(&self, tree: &ID, store: &str)
    -> Result<HashMap<String, CachedRow>>;
async fn clear_cached_rows(&self, tree: &ID, store: &str)
    -> Result<()>;

// Cache metadata
async fn get_cached_tips(&self, tree: &ID, store: &str)
    -> Result<Option<Vec<ID>>>;
async fn set_cached_tips(&self, tree: &ID, store: &str, tips: Vec<ID>)
    -> Result<()>;

// Diff computation for incremental updates
async fn get_entries_between_tips(&self, tree: &ID, store: &str, old_tips: &[ID], new_tips: &[ID])
    -> Result<Vec<Entry>>;
```

### SQL Schema

**cached_rows Table:**

```sql
CREATE TABLE IF NOT EXISTS cached_rows (
    tree_id TEXT NOT NULL,
    store_name TEXT NOT NULL,
    uuid TEXT NOT NULL,
    data TEXT NOT NULL,
    is_tombstone INTEGER NOT NULL DEFAULT 0,
    last_modified_entry_id TEXT NOT NULL,
    last_modified_height INTEGER NOT NULL,
    PRIMARY KEY (tree_id, store_name, uuid)
);

CREATE INDEX IF NOT EXISTS idx_cached_rows_lookup
    ON cached_rows(tree_id, store_name);
```

**cache_tips Table:**

```sql
CREATE TABLE IF NOT EXISTS cache_tips (
    tree_id TEXT NOT NULL,
    store_name TEXT NOT NULL,
    tip_id TEXT NOT NULL,
    PRIMARY KEY (tree_id, store_name, tip_id)
);

CREATE INDEX IF NOT EXISTS idx_cache_tips_lookup
    ON cache_tips(tree_id, store_name);
```

The composite key `(tree_id, store_name, uuid)` ensures the same UUID in different stores remains isolated.

## Cache Maintenance

### Cold Start: `build_table_cache_from_entries()`

When no cache exists, build it by walking all entries from tips in reverse height order:

```rust,ignore
async fn build_table_cache_from_entries(&self, subtree_name: &str, tips: &[ID]) -> Result<()> {
    let backend = self.db.backend()?;
    let tree_id = self.db.root_id();

    // Clear existing cache
    backend.clear_cached_rows(tree_id, subtree_name).await?;

    // Get all entries reachable from tips (sorted by height ascending)
    let entries = backend.get_store_from_tips(tree_id, subtree_name, tips).await?;

    // Track seen UUIDs - first occurrence in reverse order wins
    let mut seen: HashSet<String> = HashSet::new();

    // Process in reverse order (highest height first)
    for entry in entries.into_iter().rev() {
        let ops: Vec<TableRowOp> = deserialize(entry.data(subtree_name))?;

        for op in ops {
            if seen.contains(&op.uuid) { continue; }
            seen.insert(op.uuid.clone());

            let cached_row = CachedRow { /* from op */ };
            backend.upsert_cached_row(tree_id, subtree_name, &op.uuid, &cached_row).await?;
        }
    }

    // Update cached tips
    backend.set_cached_tips(tree_id, subtree_name, tips.to_vec()).await?;
    Ok(())
}
```

**Memory Efficiency**: Cold start processes entries one at a time - it does not require holding the full table state in memory.

### Incremental Update: `apply_cache_diff()`

When cache exists but tips have changed, update incrementally:

```rust,ignore
async fn apply_cache_diff(&self, subtree_name: &str, old_tips: &[ID], new_tips: &[ID]) -> Result<()> {
    let backend = self.db.backend()?;
    let tree_id = self.db.root_id();

    // Get only entries reachable from new_tips but not old_tips
    let diff_entries = backend
        .get_entries_between_tips(tree_id, subtree_name, old_tips, new_tips)
        .await?;

    // Process entries in ascending order (sorted by height, ID)
    for (idx, entry) in diff_entries.into_iter().enumerate() {
        let ops: Vec<TableRowOp> = deserialize(entry.data(subtree_name))?;

        for op in ops {
            // Get existing cached row for LWW comparison
            let existing = backend.get_cached_row(tree_id, subtree_name, &op.uuid).await?;

            // LWW: Apply if no existing or new entry wins
            let should_apply = match &existing {
                None => true,
                Some(cached) => (entry_height, &entry_id) > (cached.height, &cached.entry_id),
            };

            if should_apply {
                let cached_row = CachedRow { /* from op */ };
                backend.upsert_cached_row(tree_id, subtree_name, &op.uuid, &cached_row).await?;
            }
        }
    }

    // Update cached tips
    backend.set_cached_tips(tree_id, subtree_name, new_tips.to_vec()).await?;
    Ok(())
}
```

This is O(diff) instead of O(n), making tip changes efficient.

## Cache Rebuild Coordination

### Problem: Concurrent Rebuild Races

When multiple transactions concurrently access a stale cache, they could all attempt to rebuild simultaneously, causing race conditions and wasted work.

### Solution: CacheRebuildCoordinator

The `CacheRebuildCoordinator` ensures only one task rebuilds the cache at a time:

```rust,ignore
/// Key for cache rebuild coordination: (tree_id, store_name)
type CacheKey = (ID, String);

/// State for a single cache rebuild operation.
struct RebuildState {
    in_progress: bool,
    notifier: watch::Sender<u64>,  // Generation counter
}

/// Coordinates cache rebuilds to prevent concurrent rebuilds.
pub struct CacheRebuildCoordinator {
    rebuilds: Mutex<HashMap<CacheKey, RebuildState>>,
}

impl CacheRebuildCoordinator {
    /// Attempt to start a cache rebuild.
    /// Returns Some(RebuildGuard) if this task should rebuild.
    /// Returns None if another task is rebuilding (caller should wait).
    pub async fn try_start_rebuild(&self, tree_id: &ID, store: &str) -> Option<RebuildGuard>;

    /// Wait for any in-progress rebuild to complete.
    pub async fn wait_for_rebuild(&self, tree_id: &ID, store: &str);
}
```

### RebuildGuard: RAII Pattern

The `RebuildGuard` ensures the rebuild is marked complete even if the rebuilding task panics:

```rust,ignore
/// RAII guard that marks the rebuild as complete when dropped.
pub struct RebuildGuard {
    coordinator: Arc<CacheRebuildCoordinator>,
    key: CacheKey,
}

impl RebuildGuard {
    /// Manually complete the rebuild (preferred for async cleanup).
    pub async fn complete(self) {
        self.coordinator.complete_rebuild(&self.key).await;
        std::mem::forget(self);  // Prevent Drop
    }
}

impl Drop for RebuildGuard {
    fn drop(&mut self) {
        // Spawn task to complete rebuild asynchronously
        let coordinator = Arc::clone(&self.coordinator);
        let key = self.key.clone();
        tokio::spawn(async move {
            coordinator.complete_rebuild(&key).await;
        });
    }
}
```

### Usage in get_full_state_cached()

```rust,ignore
if !cache_valid {
    let coordinator = backend.cache_rebuild_coordinator().clone();

    // Try to acquire rebuild lock
    if let Some(guard) = coordinator.try_start_rebuild(tree_id, subtree_name).await {
        // We're the rebuilder - perform the rebuild
        self.build_table_cache_from_entries(subtree_name, &current_tips).await?;
        guard.complete().await;
    } else {
        // Another task is rebuilding - wait for completion
        coordinator.wait_for_rebuild(tree_id, subtree_name).await;
    }
}
```

### Watch Channel Notification

Waiting tasks use Tokio's `watch` channel to be notified when the rebuild completes:

1. Rebuilder acquires lock, `in_progress = true`
2. Waiters call `wait_for_rebuild()`, subscribe to watch channel
3. Rebuilder completes, increments generation counter via `notifier.send_modify()`
4. Waiters receive notification, wake up, can now use valid cache

## Conflict Resolution

Table uses Last-Write-Wins (LWW) at the row level:

```text
Winner = max(height, entry_id)

- height: Entry's position in DAG (higher = later)
- entry_id: Content-addressed ID (deterministic tie-breaker)
```

**Example with concurrent writes:**

```text
Entry A (height=5, id=aaa): set("user1", {name: "Alice"})
Entry B (height=5, id=bbb): set("user1", {name: "Alicia"})

Winner: Entry B (same height, "bbb" > "aaa")
Result: user1 = {name: "Alicia"}
```

This ensures all replicas converge to the same state regardless of the order entries are received.

### Cold Start vs Incremental

- **Cold start**: Entries processed in reverse height order. First occurrence per UUID wins (which is the LWW winner due to sort order).

- **Incremental**: Entries processed in forward height order with explicit LWW comparison against existing cached values.

Both approaches produce the same final state.

## Performance Characteristics

| Operation      | Warm Cache      | Cold Cache               | Historical |
| -------------- | --------------- | ------------------------ | ---------- |
| `get(key)`     | O(1)            | O(n) build, then O(1)    | O(n) walk  |
| `search(pred)` | O(rows)         | O(n) build, then O(rows) | O(n) walk  |
| Tip change     | O(diff entries) | N/A                      | N/A        |
| Write          | O(1)            | O(1)                     | N/A        |

Where:
- n = number of entries in the store's DAG
- diff entries = entries between old and new tips

### Comparison with Doc-Based Stores

| Aspect          | Table (Row-Native)             | DocStore (Doc-Based)         |
| --------------- | ------------------------------ | ---------------------------- |
| Entry contains  | Row operations                 | Full serialized CRDT         |
| CRDT granularity| Per-row LWW                    | Per-document merge           |
| Read without cache | Walk backwards, streaming   | Compute full CRDT state      |
| Memory requirement | O(1) per read               | O(table size)                |
| Cache rebuild   | O(n) entries, streaming        | O(n) entries, full Doc in memory |
| Tip change cost | O(diff) incremental            | Full rebuild                 |

## Implementation Status

All core functionality is implemented:

- Row-ops storage format (`TableRowOp`, `RowOpKind`)
- O(1) cache lookup path (`get_single_cached_row`)
- Incremental cache updates (`apply_cache_diff`)
- Cold start cache build (`build_table_cache_from_entries`)
- Historical read fallback (`build_state_from_row_ops`)
- Concurrent rebuild coordination (`CacheRebuildCoordinator`)
- Backend diff computation (`get_entries_between_tips`)
- Proper LWW heights using `entry.subtree_height()` in cached row metadata

## Future Considerations

### SQL Query Pushdown

Push predicates to SQL for indexed columns:

```sql
SELECT uuid, data FROM cached_rows
WHERE tree_id = ? AND store_name = ?
  AND json_extract(data, '$.status') = 'active'
```

This would enable efficient filtering without loading all rows for `search()`.

### Streaming Cold Start

For very large tables, stream entries from storage without loading all entry metadata at once:

```rust,ignore
async fn build_cache_streaming(&self, tips: &[ID]) -> Result<()> {
    let mut stream = backend.stream_entries_from_tips(tree_id, store, tips);
    while let Some(entry) = stream.next().await {
        // Process entry...
    }
}
```

### Partial Cache (Hot Rows)

For extremely large tables, maintain cache for frequently accessed rows only:

- Track access frequency per row
- Evict cold rows from cache
- Fall back to walk for cache misses

### Backend-Level Materialization

Move cache updates into `backend.put()` so rows are materialized as entries are ingested, not lazily on read:

```rust,ignore
// In backend.put():
if is_table_store(&subtree.name) {
    for op in parse_table_ops(&subtree.data) {
        self.apply_row_op_lww(&op, entry.id(), entry.height()).await?;
    }
}
```

This would eliminate the lazy update overhead on first read after writes.
