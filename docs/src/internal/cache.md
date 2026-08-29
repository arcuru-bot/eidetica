# Historical Store-state cache

Historical current-state reads use derived record sets in the Store-state cache.
Immutable Store tips identify the source, and the format descriptor identifies the representation.

The default opaque builder folds Store deltas in canonical historical order, serializes the Store's CRDT type, and publishes one record at the reserved opaque key.
A cold read builds and publishes the record set; a warm read resolves the published record set and reads that record.
The runtime is generic over the Store CRDT type and does not assume `Doc`.

Cached-state construction writes into a private build first.
A failed fold, serialization, private write, or publish leaves no partial published record set.
Published derived record sets are immutable.

Clearing the CRDT cache removes derived record sets as well as the legacy cache entries.
The deletion predicate selects only the `Derived` lifecycle, so authoritative records remain byte-for-byte unchanged.
The next historical read rebuilds from immutable Entries.

The legacy SQL `crdt_cache_v2` table and InMemory LRU are no longer the active local historical materialization path, but remain available for current service compatibility.
Remote historical cache calls continue using the existing service protocol until record paging is implemented.

Historical `Table` state uses the `eidetica/table/rows` format, with one
record per logical row keyed by its UTF-8 primary key. Opening a Table handle
reads no rows. Point reads fetch one record, and ordered iteration uses bounded
pages with exclusive continuation while transaction-local changes overlay the
cached record set.

The existing service protocol still uses opaque whole-state cache calls. Remote
record paging and authoritative Table records are not provided here.
