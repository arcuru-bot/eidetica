> 📋 **Status: Implemented**
>
> Delegation primitives (DelegatedTreeRef, DelegationStep, PermissionBounds, DelegationResolver) are implemented and tested.
> The Identity Database is implemented as a convenience layer on top of these primitives.

# Identity Database Design

An Identity Database is a regular Eidetica database whose `_settings.auth` contains all of a user's device keys. Its root ID serves as a stable, shareable identity reference. Other databases delegate to it via `DelegatedTreeRef` instead of tracking individual device keys.

There's no special setup in the Tree structure for it, it is a natural extension/overlay on top of the Delegated Authentication capability of the Database structure. A user could build an Identity Database today by manually creating a database, adding their device keys to its auth settings, and configuring `DelegatedTreeRef` entries in target databases. This document describes a set of convenience methods on `User` that manage named identities, automating the creation, tracking, and device-key management of these databases.

Throughout this document, "device key" refers to a private/public keypair that a User associates with a specific device. The natural usage pattern is one key per device, keeping private keys local to the device that generated them and never transferring them over the network. A user may have more than one key on a single device, but the typical case is a 1:1 mapping between devices and keys.

## Problem Statement

In the current system, granting a user access to a database requires adding each of their device keys individually to that database's `_settings.auth`. This creates several problems:

1. **Key proliferation**: When a user has multiple devices, every database they access must list every device key separately.
2. **No stable identity**: A user's identity is a collection of unrelated public keys with no inherent connection between them.
3. **Difficult key management**: Adding or revoking a device key requires updating every database the user has access to.
4. **No shareable address**: There is no single value another user can reference to mean "this person."

## Goals

- Provide a stable, shareable identity reference (a single database root ID) that represents a user across all their devices.
- Allow database administrators to grant access to an identity rather than individual keys.
- Enable device key management (add/remove) in one place, with automatic propagation through delegation.
- Build on the existing delegation primitives without new storage or protocol concepts.
- Nestable, enabling groups/teams of arbitrary depth using multiple hops.

## Non-Goals

- Replacing the User system (the Identity Database is managed _by_ a User, not a replacement for User).
- Cross-instance identity federation (identity databases sync like any other database).
- Human-readable identity naming.
- Standardized profile/metadata storage within the Identity Database (intended for future work, but the schema and API for storing names, avatars, etc. are out of scope for this document).

## Concept

### Identity as a Database

An Identity Database is a regular `Database` with specific conventions:

- Its `_settings.auth` contains entries for each device key the user controls.
- Its root ID (`ID`) is the user's stable identity address.
- It may contain additional metadata (display name, avatar, etc.) in its data stores. Standardizing the schema for this metadata is out of scope for this document but is intended as future work.

```text
Identity Database (root: sha256:abc123...)
├── _settings.auth
│   ├── global                   → AuthKey(Read, Active)
│   ├── keys
│   │   ├── "Ed25519:laptop_key..."  → AuthKey(Admin(0), Active)
│   │   ├── "Ed25519:phone_key..."   → AuthKey(Write(10), Active)
│   │   └── "Ed25519:old_tablet..."  → AuthKey(Admin(0), Revoked)
│   └── delegations
└── profile (DocStore)
    ├── display_name: "Alice"
    └── bio: "..."
```

Device keys do not all need the same permission level. At least one key must have `Admin` access to manage the identity database, but other device keys can have lower permissions depending on the trust level of the device. For example, a phone might only have `Write` access while a primary workstation has `Admin`.

It is advisable to set a global read permission on the identity database so that any peer needing to verify a delegation can read the identity database's auth settings without requiring explicit access.

### Delegation Instead of Key Distribution

When a database wants to grant access to a user, it adds a single `DelegatedTreeRef` pointing to the user's Identity Database:

```text
Target Database _settings.auth
├── "Ed25519:admin_key..." → AuthKey(Admin(0), Active)
└── delegations
    └── "sha256:abc123..."  → DelegatedTreeRef {
            permission_bounds: { max: Write(10), min: None },
            tree: { root: "sha256:abc123...", tips: [...] }
        }
```

Any device key in the Identity Database can now write to the target database, with permissions clamped by the delegation bounds. When Alice adds a new device, she only updates her Identity Database and all delegating databases automatically recognize the new key.

If a device key only has Read permissions in the Identity Database, it will continue to only have Read permissions on the target database unless the target database sets the min permission bound.

## User Workflow

### Creating an Identity

<!-- Code block ignored: Requires full Instance and User setup -->

```rust,ignore
// Create a new identity on this device
let default_key = user.get_default_key()?;
let personal = user.create_identity("personal", &default_key).await?;

// The root ID is the shareable identity address
let my_identity = personal.root_id(); // "sha256:abc123..."
```

### Joining an Identity from Another Device

An identity database is a shared, synced database. When a user sets up a new device, they don't create a new identity — they bootstrap into the existing one. The user obtains a `DatabaseTicket` for the identity database (e.g. from another device or out-of-band) and calls `register_identity`, which sends a bootstrap request for the specified permission level.

<!-- Code block ignored: Requires full Instance and User setup with sync enabled -->

```rust,ignore
// On the NEW device: register with a ticket from the existing device
let device_key = user.add_private_key(Some("work_laptop")).await?;
// Add the DatabaseTicket obtained from an existing device
let ticket: DatabaseTicket = "eidetica:?db=sha256:abc...&pr=iroh:endpointABC...".parse()?;
user.register_identity(
    "personal",
    &ticket,
    &device_key,
    &AuthKey::active(None, Permission::Admin(0)),
).await?;

// Check on the bootstrap request status — returns None while pending
let identity = user.get_identity("personal").await?;
assert!(identity.is_none()); // still waiting for approval

// On an EXISTING device: approve the bootstrap request
let pending = user.pending_bootstrap_requests(&sync)?;
user.approve_bootstrap_request(&sync, &pending[0].0, &approving_key)?;

// Back on the new device: after approval, get returns the identity
let identity = user.get_identity("personal").await?;
assert!(identity.is_some());
```

### Managing Keys

Once you have an `Identity`, key management methods live directly on it:

<!-- Code block ignored: Requires full Instance and User setup -->

```rust,ignore
let personal = user.get_identity("personal").await?.unwrap();

// Add a key from a device that already has admin access
personal.add_key(&new_device_pubkey, AuthKey::active(None, Permission::Write(10))).await?;

// List all keys in this identity (returns AuthSettings)
let auth = personal.keys().await?;

// Revoke a compromised key
personal.revoke_key(&old_device_pubkey).await?;
```

### Sharing Identity

<!-- Code block ignored: Requires full Instance and User setup -->

```rust,ignore
// Alice shares her identity address with Bob
let alice_identity = personal.root_id(); // "sha256:abc123..."

// Generate a ticket for sharing with another device or user
let ticket = personal.ticket();
```

### Granting Access to a Database

`Identity::as_delegation(bounds)` produces a `DelegatedTreeRef` pointing to this identity, which can be added to any target database's auth settings:

<!-- Code block ignored: Requires full Instance and User setup -->

```rust,ignore
let personal = user.get_identity("personal").await?.unwrap();

// Create a delegation reference from this identity
let delegation = personal.as_delegation(PermissionBounds {
    max: Permission::Write(10),
    min: None,
}).await?;

// Add it to the target database's settings
let target_settings = target_db.get_settings().await?;
target_settings.add_delegated_tree(delegation).await?;
```

### Opening a Database Through Identity

Once delegation is set up, `Identity::open_database` opens the target database directly:

<!-- Code block ignored: Requires full Instance and User setup with delegation -->

```rust,ignore
let identity = user.get_identity("personal").await?.unwrap();

// Open the target database — delegation path is discovered automatically
let db = identity.open_database(&target_root_id).await?;
```

`Identity::open_database` calls `Database::find_sigkeys` internally, which discovers both direct keys and single-hop delegation paths. No manual `SigKey::Delegation` construction is needed.

The older two-step approach via `identity_key` and `open_database_with_key` remains available for advanced use cases.

## API Surface

### User Methods (Identity Lifecycle)

`User` manages the lifecycle of named identities: creating, registering, retrieving, and listing them.

| Method                                                 | Description                                                                                                                                                                                                                                                                |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `User::create_identity(name, key_id)`                  | Creates a new Identity Database with global read access and registers it locally under `name`. The given key is added as `Admin(0)`. Returns an `Identity`.                                                                                                                |
| `User::register_identity(name, ticket, key, auth_key)` | Registers a named identity from a `DatabaseTicket` and sends a bootstrap request for the given key with the requested `AuthKey` permissions. Uses the Instance's sync system internally. The identity is tracked locally in a pending state until the request is approved. |
| `User::get_identity(name)`                             | Returns `Some(Identity)` if the name is registered and access has been granted, `None` if the name is unknown or the bootstrap request is still pending.                                                                                                                   |
| `User::identity_id(name)`                              | Returns the root ID of the named identity (the shareable address). Available immediately after `register_identity`, even before approval.                                                                                                                                  |
| `User::identity_key(name)`                             | Returns the local key associated with the named identity.                                                                                                                                                                                                                  |
| `User::identities()`                                   | Returns a `Doc` containing all locally registered identity names mapped to their tracking data (root ID and status).                                                                                                                                                       |
| `User::remove_identity(name)`                          | Removes a named identity from the user's local tracking. Does not delete the underlying database.                                                                                                                                                                          |

### Identity Type

`Identity` wraps a `Database` and carries the signing key needed to open databases that delegate to it. The underlying `Database` is accessible via `Deref` for full database operations.

```rust,ignore
pub struct Identity {
    database: Database,
    key_id: PublicKey,
    signing_key: PrivateKey,
    name: String,
    user_database: Database,
}
```

### Identity Methods (Key Management)

| Method                                   | Description                                                                                                |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `Identity::add_key(pubkey, auth_key)`    | Adds a key to this identity's auth settings with the specified `AuthKey` (name, permission level, status). |
| `Identity::revoke_key(pubkey)`           | Revokes a key in this identity's auth settings.                                                            |
| `Identity::keys()`                       | Returns an `AuthSettings` snapshot of all keys in this identity with their status and permissions.         |
| `Identity::set_key(key_id, signing_key)` | Changes which local key this identity uses. Persists to the user database and updates in-memory state.     |

### Identity Methods (Delegation)

| Method                            | Description                                                                                                                                                                         |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Identity::as_delegation(bounds)` | Returns a `DelegatedTreeRef` pointing to this identity with the given `PermissionBounds`. The result can be passed to `SettingsStore::add_delegated_tree()` on any target database. |
| `Identity::ticket()`              | Returns a `DatabaseTicket` for this identity database (sync, infallible), suitable for sharing with other devices or users.                                                         |

### Identity Methods (Database Access)

| Method                             | Description                                                                                                                                              |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Identity::open_database(root_id)` | Opens a database that delegates to this identity. Discovers the delegation SigKey automatically and opens the database using the identity's signing key. |

### Identity Methods (Accessors)

| Method                 | Description                                                          |
| ---------------------- | -------------------------------------------------------------------- |
| `Identity::root_id()`  | Returns the root ID of this identity (the shareable address).        |
| `Identity::key_id()`   | Returns the public key associated with this identity on this device. |
| `Identity::name()`     | Returns the tracking name of this identity.                          |
| `Identity::database()` | Returns a reference to the underlying `Database`.                    |

## Storage Conventions

### Identity Database Tracking

The identity database itself is a regular synced database — it lives in the Instance's storage alongside all other databases and syncs with peers like any other database. The source of truth for an identity is the identity database's own `_settings.auth`, not any local User state.

For convenience, the local User maintains a name-to-ID mapping in a DocStore on the user's private database. This is purely a local index for fast lookup and human-friendly naming:

```text
User Database
└── identities (DocStore)
    ├── "personal": { root_id: "sha256:abc123...", status: "active" }
    └── "work":     { root_id: "sha256:def456...", status: "pending" }
```

This mapping is populated when a user creates a new identity (`create_identity`, immediately active) or registers one via ticket (`register_identity`, initially pending). `get_identity(name)` looks up the root ID, checks that the status is active (i.e. bootstrap has been approved and the database is accessible), and opens the corresponding database from the Instance. `identities()` lists all locally registered names with their status.

## Key Management Implications

### Device Key Revocation

When a device key is revoked in the Identity Database:

1. **Existing entries remain valid**: Entries signed before revocation were valid at the time of signing.
2. **New entries are eventually rejected**: Once the identity database's revocation has synced to a peer, that peer's `DelegationResolver` loads the current auth settings and sees the key as `Revoked`, rejecting new entries signed by it.
3. **Propagation is not instant**: Revocation only takes effect on peers that have synced the updated identity database. Until then, peers with stale state will still accept entries from the revoked key. This is an inherent property of eventually-consistent delegation.

> **Known limitation**: The current implementation always resolves delegation against the identity database's _current_ auth settings, ignoring the specific tips claimed in `SigKey::Delegation`. This means revocation takes effect retroactively on the local node — the resolver does not evaluate permissions at the state the signer actually observed. The correct behavior (resolving against the auth snapshot at the claimed tips) is tracked as a known issue. See the [Delegated Authentication](delegated_authentication.md) design document for details.

### Key Rotation

To rotate a device key:

1. Add the new key to the Identity Database (`identity.add_key()`).
2. Switch the local identity to use the new key (`user.set_identity_key()`).
3. Revoke the old key in the Identity Database (`identity.revoke_key()`).
4. Future entries use the new key; past entries remain valid.

### Recovery

If all device keys are lost, the Identity Database becomes inaccessible (no key can write to it). Recovery options:

- **Pre-configured recovery key**: Store an offline recovery key in the Identity Database's auth settings.
- **Social recovery**: Future feature where trusted contacts can authorize a new key.
- **New identity**: Create a new Identity Database and have database admins update their delegations.

## Sync Considerations

### Cross-Device Sync

The Identity Database syncs like any other database:

- A new device gains access to an identity by bootstrapping into the identity database. An existing device with admin access approves the request, adding the new device's key to the identity's auth settings.
- Once a device has access, it syncs the identity database and receives updates through normal sync.
- When Alice adds a new device key on her laptop, her phone receives the update through normal sync.
- The new device can then access all databases that delegate to Alice's identity.

### Delegation Tip Evolution

As the Identity Database changes (keys added/revoked), its tips advance. This affects delegation:

1. **Entries signed with old tips**: The `DelegationResolver` validates that claimed tips are ancestors of current tips. Old entries remain valid because their claimed tips are ancestors of the current state.
2. **Target database delegation refs**: The `DelegatedTreeRef` in the target database stores tips at the time the delegation was configured. These are used as a baseline; the resolver accepts entries with tips that are equal to or descended from the stored tips.
3. **Tip refresh**: Database admins can periodically update the `DelegatedTreeRef` tips to point to more recent Identity Database state. This is optional — old tips remain valid.

## Relationship to Existing Delegation

The Identity Database concept maps directly to existing primitives:

| Concept             | Primitive                                                  |
| ------------------- | ---------------------------------------------------------- |
| Identity address    | Database root `ID`                                         |
| Device keys         | `_settings.auth` entries                                   |
| Granting access     | `DelegatedTreeRef` in target DB                            |
| Permission control  | `PermissionBounds` (max/min)                               |
| Entry signing       | `SigKey::Delegation { path, hint }`                        |
| Validation          | `DelegationResolver::resolve_delegation_path_with_depth()` |
| Permission clamping | `clamp_permission()`                                       |

No new storage formats, wire protocols, or CRDT types are needed. The Identity Database is a usage pattern built on delegation.

## Future Considerations

- **Identity metadata**: Profile information (display name, avatar) stored in the Identity Database's data stores.
- **Identity discovery**: A naming system that maps human-readable names to Identity Database root IDs.
- **Delegation groups**: An Identity Database could itself delegate to other Identity Databases, forming organizational hierarchies (team → members' identities).
- **Selective delegation**: Per-store delegation bounds, allowing fine-grained access control within a database.
