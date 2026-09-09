# Store state

Stores define how their CRDT state is represented for reading.
The default `StoreStateModel` is an opaque projection: ordered Store deltas are folded with the Store's `CRDT` implementation and the serialized result is stored at one reserved record key.
This default applies to any Store data type and does not assume `Doc`.

Backends persist each Store state as an opaque byte-keyed record set.
Keys use unsigned lexicographic byte order, point reads address one key, and scans use half-open ranges with an exclusive continuation key.
The backend does not parse record keys or values and does not know Store types.

A record set has one lifecycle:

- **Derived** record sets materialize a historical Entry snapshot, are immutable after publication, and can be cleared and rebuilt.
- **Authoritative** record sets contain durable current state and are not eligible for cache clearing.
- **Staging** builds are unpublished private state and are invisible to record readers.

A materializer creates a private build, writes record chunks, then publishes atomically.
Aborting or failing publication leaves no published record set and no private build behind.
Two materializers can derive the same target concurrently; publication resolves that race to one shared record set rather than failing the loser.
Projection descriptors identify the Store-owned record format and its version, so different formats and historical sources do not collide.

Remote record operations are explicitly unsupported until the service protocol exposes bounded paging and staging.
