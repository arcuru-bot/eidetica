> 📋 **Status: Stub**
>
> This document is a placeholder for a full design covering the semantics, limitations, and known issues of delegated authentication.

# Delegated Authentication

<!-- FIXME: This is a stub. Expand into a full design document covering the topics below. -->

Delegated authentication allows a database to grant permissions to keys in another database's `_settings.auth` via `DelegatedTreeRef`. This document covers the precise semantics of delegation resolution, known limitations, and planned improvements.

## Known Issues

### Tip-Based Auth Snapshot Resolution

**Status**: Not implemented. Tracked by `FIXME(security)` in `crates/lib/src/auth/validation/entry.rs`.

When an entry is signed via `SigKey::Delegation`, it records the tips of the delegated tree at the time of signing. The intended behavior is for the `DelegationResolver` to evaluate permissions against the auth settings snapshot at those claimed tips — reflecting the state the signer actually observed.

The current implementation does not do this. Instead:

1. `validate_tip_ancestry` only checks that claimed tips exist as entries in the backend (not even scoped to the correct tree).
2. The resolver then loads the delegated tree's _current_ auth settings regardless of claimed tips.

**Consequences**:

- Key revocations take effect retroactively on the local node, even for entries that were validly signed before the revocation.
- A key that was valid at signing time may fail validation if the identity database has since revoked it and synced to the validating peer.
- Conversely, a key added _after_ signing would be accepted if it exists in the current state.

**Correct behavior**: The resolver should reconstruct the auth settings as of the claimed tips and evaluate permissions against that snapshot. This requires walking the delegated tree's history to build the settings state at the specified point.

### Sync Ordering and Unverified Entries

**Status**: Not implemented. Tracked by `TODO` in `crates/lib/src/backend/mod.rs`.

Entries signed via delegation require the delegated tree to be present locally for validation. During sync, the delegated tree may arrive after the entries that reference it. Currently those entries are stored as `Failed` with no retry.

**Planned fix**: Add an `Unverified` entry status and a re-verification pass that runs when new trees become available, promoting entries whose delegation paths are now resolvable.

### Deep Nested Delegation Discovery

**Status**: Partial. Tracked by `FIXME` in `crates/lib/src/database/mod.rs`.

`Database::find_sigkeys` discovers delegation paths for signing, but only traverses one level of delegation. Deeply nested chains (identity delegates to group, group delegates to org) are not yet discovered automatically.

## Topics to Cover

<!-- FIXME: Expand each of these into full sections -->

- Delegation resolution algorithm (step-by-step walkthrough)
- Permission clamping semantics across multi-hop chains
- Tip ancestry validation: what "valid tips" means and edge cases
- Revocation semantics: retroactive vs. point-in-time evaluation
- Interaction with global permissions in delegated trees
- Sync ordering guarantees (or lack thereof) for delegation dependencies
- Security model and threat analysis
