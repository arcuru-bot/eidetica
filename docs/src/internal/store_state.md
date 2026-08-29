# Store state

Stores define how their CRDT state is represented for reading.
The default `StoreStateModel` is an opaque projection: ordered Store deltas are folded with the Store's `CRDT` implementation and the serialized result is stored at one reserved record key.
This default applies to any Store data type and does not assume `Doc`.

Backends persist Store state as opaque byte-keyed records grouped into namespaces.
Keys use unsigned lexicographic byte order, point reads address one key, and scans use half-open ranges with an exclusive continuation key.
The backend does not parse record keys or values and does not know Store types.

A namespace has one lifecycle:

- **Derived** namespaces materialize a historical Entry snapshot, are immutable after publication, and can be cleared and rebuilt.
- **Authoritative** namespaces contain durable current state and are not eligible for cache clearing.
- **Staging** namespaces are unpublished build state and are invisible to record readers.

A materializer creates staging, writes record chunks, then publishes atomically.
Aborting or failing publication leaves no ready namespace and no staging namespace behind.
Two materializers can derive the same target concurrently; publication resolves that race to one shared namespace rather than failing the loser.
Projection descriptors identify the Store-owned representation and its version, so different representations and historical sources do not collide.

Remote record operations are explicitly unsupported until the service protocol exposes bounded paging and staging.
