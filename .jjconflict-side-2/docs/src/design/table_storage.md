# Table Store: Row-Native Storage

> **Status: Partially Implemented**
>
> Core row-ops storage format is implemented. Entries store `Vec<TableRowOp>`
> instead of serialized Doc. Cold start cache build and historical reads work.
>
> **Not yet implemented:**
> - Incremental cache updates (`apply_diff`) - currently does full rebuild
> - `get_entries_between_tips` backend method for computing diffs

The Table store uses a row-native storage model where individual rows are materialized in the backend and incrementally maintained as the DAG evolves. This enables O(1) single-row lookups without loading the entire table into memory.

## Table of Contents

- [Overview](#overview)
- [Problem Statement](#problem-statement)
- [Design Goals](#design-goals)
- [Architecture](#architecture)
- [Entry Format](#entry-format)
- [Read Path](#read-path)
- [Write Path](#write-path)
- [Cache Maintenance](#cache-maintenance)
- [Cold Start](#cold-start)
- [Conflict Resolution](#conflict-resolution)
- [SQL Schema](#sql-schema)
- [Performance Characteristics](#performance-characteristics)
- [Comparison with Doc-Based Stores](#comparison-with-doc-based-stores)
- [Concurrency Requirements](#concurrency-requirements)
- [Future Considerations](#future-considerations)

## Overview

Table stores row data directly in the backend with CRDT metadata for conflict resolution. Unlike Doc-based stores that serialize all data into a single CRDT document, Table treats each row as an independent unit with its own Last-Write-Wins (LWW) semantics.

**Key Properties:**

- **O(1) single-row lookups**: `get(key)` fetches one row from storage
- **Incremental cache updates**: Tip changes apply only diff entries, not full rebuild
- **Memory efficient**: No need to load full table state into memory
- **Row-level LWW**: Conflicts resolved per-row using (height, entry_id)

## Problem Statement

The previous Doc-based approach for Table stores has fundamental limitations:

1. **Full state required for any read**: Computing CRDT state requires merging all Docs from all entries in the DAG, loading everything into memory.

2. **Cache rebuild requires full Doc**: Even with caching, rebuilding the cache requires constructing the complete merged Doc in memory first.

3. **Tables larger than memory impossible**: If the table doesn't fit in memory, it cannot be queried at all.

4. **Mismatch between storage and access patterns**: Table wants row-level access but uses document-level CRDT, forcing unnecessary work.

## Design Goals

### Primary Goals

- **O(1) single-row reads**: Fetch one row without loading others
- **Incremental updates**: Tip changes don't require full recomputation
- **Support large tables**: Tables larger than available memory
- **Correct CRDT semantics**: LWW conflict resolution across concurrent writes

### Non-Goals

- **Query pushdown**: `search()` still loads matching rows into memory
- **Transactional isolation**: Reads see committed state at tips
- **Historical state caching**: Only current tips are cached

## Architecture

### Components

```text
┌─────────────────────────────────────────────────────────────┐
│                      Table<T>::get(key)                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                  Check local staged data                    │
│                  (Transaction's pending writes)             │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│              Is this a current-tips read?                   │
│  ┌────────────────────────────────────────────────────┐     │
│  │  Yes: Check/update cache, read from cache          │     │
│  │  No:  Historical read, walk backwards from tips    │     │
│  └────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
                              │
            ┌─────────────────┴─────────────────┐
            ▼                                   ▼
┌───────────────────────┐           ┌───────────────────────┐
│   Current Tips Read   │           │   Historical Read     │
│                       │           │                       │
│  cached_tips match?   │           │  Walk backwards from  │
│  Yes → read cache     │           │  tips until row found │
│  No  → update cache   │           │  (no cache update)    │
│        then read      │           │                       │
└───────────────────────┘           └───────────────────────┘
```

### Data Model

**Entries** contain row operations (not serialized Doc):

```rust
pub struct TableRowOp {
    pub uuid: String,
    pub kind: RowOpKind,
}

pub enum RowOpKind {
    Set { data: String },  // Serialized row JSON
    Delete,                // Tombstone
}
```

**Cached rows** store materialized state with CRDT metadata:

```rust
pub struct CachedRow {
    pub data: String,              // Serialized row JSON
    pub is_tombstone: bool,        // Deleted flag
    pub entry_id: ID,              // Entry that wrote this value
    pub height: usize,             // Height of that entry
}
```

**Cache tips** track which tips the cache represents:

```rust
// Per (tree_id, store_name): the tips this cache was computed from
cache_tips: Vec<ID>
```

## Entry Format

Table entries store a list of row operations in `SubTreeNode.data`:

```rust
// When Table::insert/set/delete is called:
// 1. Create TableRowOp for the operation
// 2. Add to list of ops for this subtree in the transaction
// 3. On commit, serialize ops list into SubTreeNode.data

// Entry structure for Table stores:
Entry {
    tree: TreeNode { ... },
    subtrees: vec![
        SubTreeNode {
            name: "users",  // Table name
            parents: [...], // Subtree parents (previous tips)
            data: serialize(&vec![
                TableRowOp { uuid: "abc", kind: Set { data: "{...}" } },
                TableRowOp { uuid: "def", kind: Delete },
            ]),
        }
    ],
    sig: ...,
}
```

Each entry records the row operations performed, not the full table state.

## Read Path

### Current Tips Read (Common Case)

```rust
pub async fn get(&self, key: &str) -> Result<T> {
    // 1. Check local staged data first
    if let Some(value) = self.check_staged_data(key)? {
        return Ok(value);
    }

    // 2. Get current tips
    let current_tips = self.backend.get_store_tips(tree_id, store).await?;

    // 3. Is this a current-tips read?
    if self.transaction_tips != current_tips {
        // Historical read - walk backwards
        return self.read_historical(key, &self.transaction_tips).await;
    }

    // 4. Check cache validity
    let cached_tips = self.backend.get_cached_tips(tree_id, store).await?;

    if cached_tips != Some(current_tips.clone()) {
        // Cache stale - update incrementally
        self.update_cache(&cached_tips, &current_tips).await?;
    }

    // 5. Read from cache
    match self.backend.get_cached_row(tree_id, store, key).await? {
        Some(row) if !row.is_tombstone => {
            serde_json::from_str(&row.data).map_err(Into::into)
        }
        _ => Err(Error::KeyNotFound),
    }
}
```

### Historical Read

For reads at non-current tips, walk backwards through entries:

```rust
async fn read_historical(&self, key: &str, tips: &[ID]) -> Result<T> {
    // Walk entries backwards from tips (reverse height order)
    let mut entries = self.backend.get_store_from_tips(tree_id, store, tips).await?;
    entries.sort_by(|a, b| {
        // Reverse sort: highest height first, then entry_id
        (b.height(), b.id()).cmp(&(a.height(), a.id()))
    });

    for entry in entries {
        let ops: Vec<TableRowOp> = deserialize(&entry.subtree_data(store))?;
        for op in ops {
            if op.uuid == key {
                // First match in reverse order = LWW winner
                return match op.kind {
                    RowOpKind::Set { data } => serde_json::from_str(&data),
                    RowOpKind::Delete => Err(Error::KeyNotFound),
                };
            }
        }
    }

    Err(Error::KeyNotFound)
}
```

## Write Path

Writes stage row operations in the transaction, committed as an entry:

```rust
pub async fn set(&self, key: &str, value: &T) -> Result<()> {
    let data = serde_json::to_string(value)?;
    let op = TableRowOp {
        uuid: key.to_string(),
        kind: RowOpKind::Set { data },
    };
    self.transaction.stage_table_op(store, op).await
}

pub async fn delete(&self, key: &str) -> Result<()> {
    let op = TableRowOp {
        uuid: key.to_string(),
        kind: RowOpKind::Delete,
    };
    self.transaction.stage_table_op(store, op).await
}

// On commit:
// - Staged ops become SubTreeNode.data
// - Entry is created with proper parents
// - Cache is NOT updated (lazy update on next read)
```

## Cache Maintenance

### Incremental Update

When `cached_tips != current_tips`, update incrementally:

```rust
async fn update_cache(
    &self,
    cached_tips: &Option<Vec<ID>>,
    current_tips: &[ID],
) -> Result<()> {
    match cached_tips {
        None => {
            // Cold start - build from scratch
            self.build_cache_cold(current_tips).await
        }
        Some(old_tips) => {
            // Incremental update
            self.apply_diff(old_tips, current_tips).await
        }
    }
}

async fn apply_diff(&self, old_tips: &[ID], new_tips: &[ID]) -> Result<()> {
    // 1. Find entries reachable from new_tips but not from old_tips
    let diff_entries = self.backend
        .get_entries_between_tips(tree_id, store, old_tips, new_tips)
        .await?;

    // 2. Sort by (height, entry_id) ascending - process in causal order
    diff_entries.sort_by(|a, b| (a.height(), a.id()).cmp(&(b.height(), b.id())));

    // 3. Apply each entry's ops using LWW
    for entry in diff_entries {
        let ops: Vec<TableRowOp> = deserialize(&entry.subtree_data(store))?;
        let entry_height = entry.height();
        let entry_id = entry.id();

        for op in ops {
            self.apply_row_op_lww(&op, entry_id, entry_height).await?;
        }
    }

    // 4. Update cached tips
    self.backend.set_cached_tips(tree_id, store, new_tips.to_vec()).await
}

async fn apply_row_op_lww(
    &self,
    op: &TableRowOp,
    entry_id: &ID,
    entry_height: usize,
) -> Result<()> {
    let existing = self.backend.get_cached_row(tree_id, store, &op.uuid).await?;

    let should_apply = match existing {
        None => true,
        Some(ref row) => (entry_height, entry_id) > (row.height, &row.entry_id),
    };

    if should_apply {
        let cached_row = match &op.kind {
            RowOpKind::Set { data } => CachedRow {
                data: data.clone(),
                is_tombstone: false,
                entry_id: entry_id.clone(),
                height: entry_height,
            },
            RowOpKind::Delete => CachedRow {
                data: String::new(),
                is_tombstone: true,
                entry_id: entry_id.clone(),
                height: entry_height,
            },
        };
        self.backend.upsert_cached_row(tree_id, store, &op.uuid, &cached_row).await?;
    }

    Ok(())
}
```

## Cold Start

When no cache exists, build it by walking backwards from tips:

```rust
async fn build_cache_cold(&self, tips: &[ID]) -> Result<()> {
    // Get all entries reachable from tips
    let entries = self.backend.get_store_from_tips(tree_id, store, tips).await?;

    // Track seen UUIDs - first occurrence in reverse order wins
    let mut seen: HashSet<String> = HashSet::new();

    // Sort in reverse order (highest height first)
    entries.sort_by(|a, b| (b.height(), b.id()).cmp(&(a.height(), a.id())));

    for entry in entries {
        let ops: Vec<TableRowOp> = deserialize(&entry.subtree_data(store))?;

        for op in ops {
            if seen.contains(&op.uuid) {
                continue; // Already have the winning value
            }
            seen.insert(op.uuid.clone());

            let cached_row = match &op.kind {
                RowOpKind::Set { data } => CachedRow {
                    data: data.clone(),
                    is_tombstone: false,
                    entry_id: entry.id().clone(),
                    height: entry.height(),
                },
                RowOpKind::Delete => CachedRow {
                    data: String::new(),
                    is_tombstone: true,
                    entry_id: entry.id().clone(),
                    height: entry.height(),
                },
            };
            self.backend.upsert_cached_row(tree_id, store, &op.uuid, &cached_row).await?;
        }
    }

    self.backend.set_cached_tips(tree_id, store, tips.to_vec()).await
}
```

Note: Cold start processes entries one at a time - it does not require holding the full table state in memory.

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

## SQL Schema

### cached_rows Table

```sql
CREATE TABLE IF NOT EXISTS cached_rows (
    tree_id TEXT NOT NULL,
    store_name TEXT NOT NULL,
    uuid TEXT NOT NULL,
    data TEXT NOT NULL,
    is_tombstone INTEGER NOT NULL DEFAULT 0,
    entry_id TEXT NOT NULL,
    height INTEGER NOT NULL,
    PRIMARY KEY (tree_id, store_name, uuid)
);

CREATE INDEX IF NOT EXISTS idx_cached_rows_lookup
    ON cached_rows(tree_id, store_name);
```

### cache_tips Table

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

## Performance Characteristics

| Operation | Warm Cache | Cold Cache | Historical |
|-----------|------------|------------|------------|
| `get(key)` | O(1) | O(n) build, then O(1) | O(n) walk |
| `search(pred)` | O(rows) | O(n) build, then O(rows) | O(n) walk |
| Tip change | O(diff entries) | N/A | N/A |
| Write | O(1) | O(1) | N/A |

Where:
- n = number of entries in the store's DAG
- diff entries = entries between old and new tips

**Key improvements over Doc-based approach:**

| Aspect | Doc-Based | Row-Native |
|--------|-----------|------------|
| Cache rebuild | O(n) entries, full Doc in memory | O(diff) entries, streaming |
| Cold start | Full CRDT merge in memory | Streaming walk, no full state |
| Large tables | Limited by memory | Limited by storage only |
| Tip change cost | Full rebuild | Incremental diff |

## Comparison with Doc-Based Stores

Table's row-native storage differs from DocStore and YDoc:

| Aspect | Table (Row-Native) | DocStore/YDoc (Doc-Based) |
|--------|-------------------|---------------------------|
| Entry contains | Row operations | Full serialized CRDT |
| CRDT granularity | Per-row LWW | Per-document merge |
| Read without cache | Walk backwards | Compute full CRDT state |
| Memory requirement | O(1) per read | O(table size) |
| Conflict resolution | Row-level LWW | Document CRDT semantics |

DocStore and YDoc continue to use the Doc-based approach because their access patterns benefit from document-level CRDT semantics (nested structures, collaborative editing).

## Concurrency Requirements

The [Doc-based caching design](table_caching.md#concurrency-limitations) has several concurrency gaps: non-atomic rebuild, TOCTOU races causing thundering herd, and inconsistent locking across backends. The row-native design must address these.

### Atomic Cache Updates

Cache updates (both incremental and cold-start) must be atomic with respect to concurrent readers. A reader must never observe a partially-updated cache.

**SQL backend**: Wrap the entire `apply_diff` or `build_cache_cold` sequence in a single SQL transaction. This ensures that the row upserts and tip updates are committed together or not at all. Concurrent readers either see the old complete cache or the new complete cache.

```sql
BEGIN;
-- Apply row ops (upserts/deletes)
-- Update cache_tips
COMMIT;
```

**In-memory backend**: Use a single lock acquisition for the full update. The in-memory backend consolidates core data under a single `RwLock<InMemoryInner>`; row cache state should follow the same pattern rather than using separate locks for `cached_rows` and `cached_tips`.

### Preventing Thundering Herd

When the cache is stale, only one reader should perform the rebuild. Other concurrent readers should either wait for the rebuild to complete or fall back to direct computation.

Possible approaches:

- **Rebuild lock per (tree_id, store_name)**: A `Mutex` or `tokio::sync::Mutex` that serializes rebuild attempts. The first reader acquires the lock and rebuilds; others block on the lock and then read the freshly-built cache.
- **Optimistic check after lock**: After acquiring the rebuild lock, re-check `cached_tips == current_tips` (another thread may have completed the rebuild while waiting). Skip rebuild if cache is now valid.
- **Fallback under contention**: If the rebuild lock is held, fall back to `get_full_state()` instead of blocking. This trades redundant CRDT computation for lower latency.

### Tips Consistency

The row-native design uses `set_cached_tips` as the final step of cache updates. This ordering is critical: rows must be fully populated before tips are set, because tips serve as the validity signal. A reader that sees matching tips assumes the rows are complete.

For the SQL backend, wrapping in a SQL transaction handles this automatically. For the in-memory backend, a single lock acquisition for both rows and tips ensures the same guarantee.

### Backend Consistency Between Implementations

Both backends must provide the same atomicity guarantees at the `BackendImpl` trait level:

- `clear_cached_rows` must clear both rows and tips atomically (the in-memory backend currently only clears rows)
- `set_cached_tips` must only be callable after rows are fully populated
- All cache operations for a given `(tree_id, store_name)` must be serializable

Consider adding a single `rebuild_cache` method to `BackendImpl` that accepts the full set of rows and tips, allowing each backend to implement the atomic swap in its own way (SQL transaction, single lock acquisition, etc.).

## Future Considerations

### Indexed Search

Push predicates to SQL for indexed columns:

```sql
SELECT uuid, data FROM cached_rows
WHERE tree_id = ? AND store_name = ?
  AND json_extract(data, '$.status') = 'active'
```

### Streaming Cold Start

For very large tables, stream entries from storage without loading all entry metadata:

```rust
async fn build_cache_streaming(&self, tips: &[ID]) -> Result<()> {
    let mut stream = self.backend.stream_entries_from_tips(tree_id, store, tips);
    // Process entries as they arrive...
}
```

### Partial Cache

For extremely large tables, maintain cache for "hot" rows only:

- Track access frequency per row
- Evict cold rows from cache
- Fall back to walk for cache misses

### Backend-Level Materialization

Move cache updates into `backend.put()` so rows are materialized as entries are ingested, not lazily on read:

```rust
// In backend.put():
if is_table_store(&subtree.name) {
    for op in parse_table_ops(&subtree.data) {
        self.apply_row_op_lww(&op, entry.id(), entry.height()).await?;
    }
}
```

This would eliminate the lazy update overhead on first read after writes.
