> ⚠️ **Status: Implemented (Superseded)**
>
> This design is implemented but has known limitations. Cache rebuild requires
> holding the full table state in memory, defeating the goal of supporting
> tables larger than memory. See [Table Store: Row-Native Storage](table_storage.md)
> for the replacement design.

# Table Store Caching

The Table store uses a SQL-backed row cache to provide O(1) single-row lookups without loading the entire table into memory. This enables efficient reads from tables of any size.

## Table of Contents

- [Overview](#overview)
- [Problem Statement](#problem-statement)
- [Design Goals](#design-goals)
- [Architecture](#architecture)
- [Cache Validity Model](#cache-validity-model)
- [Read Path](#read-path)
- [Cache Rebuild](#cache-rebuild)
- [SQL Schema](#sql-schema)
- [Performance Characteristics](#performance-characteristics)
- [Future Considerations](#future-considerations)

## Overview

The Table store cache is a materialized view of CRDT state stored directly in SQL. Each row in the cache corresponds to one row in the Table store, keyed by UUID.

**Key Properties:**

- **O(1) single-row lookups**: `get(key)` fetches one row from SQL, not the full table
- **Tips-based validity**: Cache is valid iff `cached_tips == current_tips`
- **Lazy rebuild**: Cache rebuilds on read when stale, not on write
- **Memory efficient**: Only the requested row loads into memory

## Problem Statement

Without caching, every Table read operation requires:

1. **Full CRDT computation**: Traverse the DAG from root to tips, merging all entries
2. **Full Doc construction**: Build the entire table state in memory
3. **Single value extraction**: Return just the requested row

This has complexity O(D × M) where D is DAG depth and M is merge points. For large tables:

- Tables with millions of rows require loading everything for a single lookup
- Tables that don't fit in memory cannot be queried at all
- Repeated reads recompute the same CRDT state

## Design Goals

### Primary Goals

- **O(1) single-row lookups**: Fetch one row without loading the table
- **Support large tables**: Tables larger than available memory
- **Correct invalidation**: Never serve stale data
- **Lazy rebuild**: Defer computation until needed

### Non-Goals

- **Incremental updates**: Any change triggers full rebuild (simplicity over optimization)
- **Write-through caching**: Writes don't update cache (only reads trigger rebuild)
- **Query optimization**: `search()` still loads all rows from cache

## Architecture

### Components

```text
┌─────────────────────────────────────────────────────────────┐
│                      Table<T>::get(key)                     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Check local staged data                  │
│                    (Transaction's pending changes)          │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│              ensure_table_cache_valid()                     │
│  ┌────────────────────────────────────────────────────┐     │
│  │  Compare cached_tips vs current_tips               │     │
│  │  If mismatch → rebuild_table_cache()               │     │
│  └────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│              get_single_cached_row(key)                     │
│                    O(1) SQL lookup                          │
└─────────────────────────────────────────────────────────────┘
```

### Key Structs

**CachedRow** stores a single cached row:

```rust,ignore
pub struct CachedRow {
    pub data: String,                    // Serialized JSON
    pub is_tombstone: bool,              // Deleted flag
    pub last_modified_entry_id: ID,      // Provenance tracking
    pub last_modified_height: usize,     // Entry height
}
```

### BackendImpl Methods

The cache requires 6 new methods on `BackendImpl`:

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
```

### Transaction Helpers

Transaction provides cache management methods:

- `is_table_cache_valid()` - Check if cache tips match current tips
- `get_single_cached_row()` - Fetch one row from cache
- `ensure_table_cache_valid()` - Rebuild if stale, return whether cache usable

## Cache Validity Model

The cache is a materialized view of CRDT state at a specific set of tips.

**Key Insight**: CRDT state is fully determined by the tips. Two databases with identical tips have identical state.

**Validity Check**:

```text
cached_tips == current_tips  →  Cache is valid
cached_tips != current_tips  →  Cache is stale, rebuild needed
```

This is an exact set comparison. If any tip differs, the entire cache is invalid.

**Why Not Incremental Updates?**

Incremental updates would require:
- Tracking which rows each entry modifies
- Handling CRDT merge semantics correctly
- Complex logic for concurrent modifications

Full rebuild is simpler and correct by construction. The cost is acceptable because:
- Reads are typically more frequent than writes
- Rebuild cost is amortized across many reads
- First read after write pays the rebuild cost once

**Historical Transactions**

Transactions at historical tips (not current database tips) bypass the cache entirely. The cache only represents the current state.

## Read Path

### Table::get(key)

```rust,ignore
pub async fn get(&self, key: impl AsRef<str>) -> Result<T> {
    // 1. Check local staged data
    if let Some(value) = self.check_local_data(key)? {
        return Ok(value);
    }

    // 2. Ensure cache is valid (rebuilds if stale)
    if self.atomic_op.ensure_table_cache_valid(&self.name).await? {
        // 3. O(1) single-row lookup from cache
        match self.atomic_op.get_single_cached_row(&self.name, key).await? {
            Some(row) if !row.is_tombstone => {
                return serde_json::from_str(&row.data)?;
            }
            _ => return Err(KeyNotFound),
        }
    }

    // 4. Fall back to full CRDT for historical transactions
    let doc = self.atomic_op.get_full_state(&self.name).await?;
    // ... extract from doc
}
```

### Table::search(predicate)

`search()` still uses `get_full_state_cached()` which:
1. Validates cache
2. Loads all cached rows
3. Reconstructs Doc
4. Filters by predicate

This avoids CRDT recomputation but still loads all rows.

## Cache Rebuild

When tips don't match, `rebuild_table_cache()` executes:

```rust,ignore
async fn rebuild_table_cache(&self, subtree_name: &str, doc: &Doc, tips: &[ID]) -> Result<()> {
    let backend = self.db.backend()?;
    let tree_id = self.db.root_id();

    // 1. Clear existing cache
    backend.clear_cached_rows(tree_id, subtree_name).await?;

    // 2. Cache each row from computed Doc
    for (uuid, value) in doc.iter() {
        if let Some(text) = value.as_text() {
            let cached_row = CachedRow {
                data: text.to_string(),
                is_tombstone: false,
                last_modified_entry_id: tips.first().cloned().unwrap_or_default(),
                last_modified_height: 0,
            };
            backend.upsert_cached_row(tree_id, subtree_name, uuid, &cached_row).await?;
        }
    }

    // 3. Update cached tips
    backend.set_cached_tips(tree_id, subtree_name, tips.to_vec()).await?;

    Ok(())
}
```

## SQL Schema

### cached_rows Table

Stores one row per Table entry:

```sql
CREATE TABLE IF NOT EXISTS cached_rows (
    tree_id TEXT NOT NULL,
    store_name TEXT NOT NULL,
    uuid TEXT NOT NULL,
    data TEXT NOT NULL,
    is_tombstone BIGINT NOT NULL DEFAULT 0,
    last_modified_entry_id TEXT NOT NULL,
    last_modified_height BIGINT NOT NULL,
    PRIMARY KEY (tree_id, store_name, uuid)
);

CREATE INDEX IF NOT EXISTS idx_cached_rows_tree_store
    ON cached_rows(tree_id, store_name);
```

### cache_tips Table

Tracks which tips the cache was computed from:

```sql
CREATE TABLE IF NOT EXISTS cache_tips (
    tree_id TEXT NOT NULL,
    store_name TEXT NOT NULL,
    tip_id TEXT NOT NULL,
    PRIMARY KEY (tree_id, store_name, tip_id)
);

CREATE INDEX IF NOT EXISTS idx_cache_tips_tree_store
    ON cache_tips(tree_id, store_name);
```

**Store Isolation**: The composite key `(tree_id, store_name, uuid)` ensures the same UUID in different stores remains isolated.

## Performance Characteristics

| Operation | Warm Cache | Cold/Stale Cache |
|-----------|------------|------------------|
| `get(key)` | O(1) | O(n) rebuild, then O(1) |
| `search(pred)` | O(n) | O(n) rebuild, then O(n) |
| Write (insert/set/delete) | O(1) | O(1) |

**Trade-offs**:

- **Lazy invalidation**: Writes are fast, first read after write pays rebuild cost
- **Full rebuild**: Simple and correct, but O(n) on cache miss
- **Memory efficient**: Only requested rows load into memory for `get()`

**When Cache Shines**:

- Read-heavy workloads with occasional writes
- Large tables where memory is constrained
- Point lookups more common than full scans

**When Cache Doesn't Help**:

- Write-heavy workloads (constant invalidation)
- Frequent `search()` operations (still loads all rows)
- Historical queries (bypass cache entirely)

## Future Considerations

### Incremental Updates

For linear tip extensions (single new entry extending single tip), the cache could update incrementally:

1. Detect linear extension pattern
2. Apply only the new entry's changes to cache
3. Update cached tips

This would make common write patterns O(1) instead of O(n) for cache update.

### SQL-Based Filtering

`search()` could push predicates to SQL for indexed columns:

```sql
SELECT uuid, data FROM cached_rows
WHERE tree_id = ? AND store_name = ? AND json_extract(data, '$.field') = ?
```

This would enable efficient filtering without loading all rows.

### Per-Row Provenance

The `last_modified_entry_id` and `last_modified_height` fields enable future optimizations:

- Partial cache rebuilds based on entry provenance
- Conflict detection at row granularity
- Audit trails for data changes
