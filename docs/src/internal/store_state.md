# Store state

Stores define how their CRDT state is represented for reading.
The default `StoreStateModel` caches state in one opaque record: it folds ordered Store deltas with the Store's `CRDT` implementation and stores the serialized result under a reserved key.
This default applies to any Store data type and does not assume `Doc`.

Backends persist each Store state as an opaque byte-keyed record set.
Keys use unsigned lexicographic byte order, point reads address one key, and scans use half-open ranges with an exclusive continuation key.
The backend does not parse record keys or values and does not know Store types.

`Table` uses the cached record format named `eidetica/table/rows` version 0.
UTF-8 primary keys are record keys and each JSON row is one record value.
Building historical cached state streams canonical `Doc` Entry deltas into a
private build; it does not reconstruct a whole Table. Table mutations are converted
back into the existing canonical `Doc` delta at historical commit, so Entry
payload and wire semantics are unchanged.

Table handles retain only their name and transaction. `get` deserializes one
row, `scan_page` reads bounded deterministic pages, and `search` collects those
pages only because its public return type is a `Vec`. Custom and remote backends
without cached-record support keep the existing whole-state behavior.

A record set has one lifecycle:

- **Derived** record sets materialize historical Entries at fixed Store tips, are immutable after publication, and can be cleared and rebuilt.
- **Authoritative** record sets contain durable current state and are not eligible for cache clearing.
- **Staging** builds are unpublished private state and are invisible to record readers.

Clearing derived state unlinks published record sets and reclaims the generation unlinked by the previous clear. An active reader keeps its view, while a new lookup misses and rebuilds. Authoritative record sets are never selected.

A builder creates private state, writes record chunks, then publishes atomically.
Aborting or failing publication leaves no published record set and no private build behind.
Two materializers can derive the same target concurrently; publication resolves that race to one shared record set rather than failing the loser.
Format descriptors identify the Store-owned record format and version, so cached state from different formats or historical sources cannot collide.

Remote record operations remain unsupported until the service protocol exposes bounded paging and staging.
