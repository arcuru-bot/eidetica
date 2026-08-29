# Historical Store-state cache

Historical current-state reads use derived namespaces in the shared Store-state record substrate.
The transaction's immutable Store snapshot identifies the source, and the Store-state descriptor identifies the representation.

The default opaque materializer folds Store deltas in canonical historical order, serializes the Store's CRDT type, and publishes one record at the reserved opaque key.
A cold read builds and publishes the namespace; a warm read resolves the ready namespace and reads that record.
The runtime is generic over the Store CRDT type and does not assume `Doc`.

Materialization writes an invisible staging namespace first.
A failed fold, serialization, staging write, or publish leaves no ready partial namespace.
Published derived namespaces are immutable.

Clearing the CRDT cache removes derived namespaces as well as the legacy cache entries.
The deletion predicate selects only the `Derived` lifecycle, so authoritative records remain byte-for-byte unchanged.
The next historical read rebuilds from immutable Entries.

The legacy SQL `crdt_cache_v2` table and InMemory LRU are no longer the active local historical materialization path, but remain available for current service compatibility.
Remote historical cache calls continue using the existing service protocol until record paging is implemented.

`Table` continues to request opaque whole-`Doc` state in this slice.
Row projection and lazy point/range reads are not implemented.
