# Identity Guide

Manage all your device keys in a single Identity Database and grant database access by identity instead of individual keys.

## Quick Start

Create an identity, share its root ID, and use it to access databases:

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, backend::database::Sqlite};
# use eidetica::auth::types::{Permission, PermissionBounds};
# use eidetica::crdt::Doc;
# use eidetica::store::DocStore;
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
#
// Create an identity — the root ID is your stable address
let identity = user.create_identity("personal", &default_key).await?;
let my_id = identity.root_id().clone();

// Share my_id with collaborators — they add a single delegation
// instead of tracking each device key individually

// Open delegated databases directly through the identity
// let db = identity.open_database(&target_root_id).await?;
# Ok(())
# }
```

## What is an Identity?

An Identity Database is a regular Eidetica database whose `_settings.auth` contains all of a user's device keys. Its root ID serves as a stable, shareable address.

**Key properties:**

- **Stable address**: One root ID represents you across all devices and databases
- **Centralized key management**: Add or revoke device keys in one place
- **Delegation**: Other databases grant access to your identity rather than individual keys — when you add a new device, every delegating database recognizes it automatically

```text
Identity Database (root: sha256:abc123...)
├── _settings.auth
│   ├── global                        → AuthKey(Read, Active)
│   ├── keys
│   │   ├── "Ed25519:laptop_key..."   → AuthKey(Admin(0), Active)
│   │   ├── "Ed25519:phone_key..."    → AuthKey(Write(10), Active)
│   │   └── "Ed25519:old_tablet..."   → AuthKey(Admin(0), Revoked)
│   └── delegations
└── (optional stores for profile data)
```

## Creating an Identity

`create_identity` creates a new Identity Database with global read access and registers it under a local name. The given key is added as `Admin(0)`.

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, backend::database::Sqlite};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
#
let personal = user.create_identity("personal", &default_key).await?;

// The root ID is the shareable identity address
let my_identity = personal.root_id();

// Generate a ticket for sharing with another device or user
let ticket = personal.ticket();
# Ok(())
# }
```

You can create multiple identities for different contexts:

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, backend::database::Sqlite};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
#
let personal = user.create_identity("personal", &default_key).await?;
let work = user.create_identity("work", &default_key).await?;

// Each identity has a distinct root ID
assert_ne!(personal.root_id(), work.root_id());
# Ok(())
# }
```

## Adding a New Device

When a user sets up a new device, they don't create a new identity — they join the existing one. The new device obtains a `DatabaseTicket` (out-of-band or from an existing device) and calls `register_identity`, which sends a bootstrap request for the specified permission level.

<!-- Code block ignored: Requires sync between two instances with HTTP transport -->

```rust,ignore
// On the NEW device: register with a ticket from the existing device
let device_key = user.add_private_key(Some("work_laptop")).await?;
let auth_key = AuthKey::active(Some("work_laptop"), Permission::Admin(0));
user.register_identity("personal", &ticket, &device_key, &auth_key).await?;

// The identity is tracked locally as Pending
let identity = user.get_identity("personal").await?;
assert!(identity.is_none()); // Still waiting for approval

// On an EXISTING device: approve the bootstrap request
let pending = sync.pending_bootstrap_requests().await?;
let (request_id, _request) = &pending[0];
user.approve_bootstrap_request(&sync, request_id, &approving_key).await?;

// Back on the new device: after syncing, get_identity transitions Pending → Active
let identity = user.get_identity("personal").await?;
assert!(identity.is_some());
```

**How the flow works:**

1. `register_identity` syncs the identity database, sends a bootstrap request, and records the identity locally as **Pending**
2. `get_identity` returns `None` while the identity is Pending
3. An existing device with Admin access approves the bootstrap request, which adds the new key to the identity's auth settings
4. After the new device syncs the updated identity database, `get_identity` detects the key is now present and transitions the status to **Active**, returning `Some(Identity)`

## Managing Keys

Once you have an `Identity`, key management methods live directly on it:

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, backend::database::Sqlite};
# use eidetica::auth::crypto::PublicKey;
# use eidetica::auth::types::{AuthKey, KeyStatus, Permission};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
# let identity = user.create_identity("personal", &default_key).await?;
#
// Add a new device key with Write permissions
let phone_key = PublicKey::random();
identity.add_key(
    &phone_key,
    AuthKey::active(Some("phone"), Permission::Write(10)),
).await?;

// List all keys in this identity
let auth = identity.keys().await?;
let key = auth.get_key_by_pubkey(&phone_key)?;
assert_eq!(key.permissions(), &Permission::Write(10));
assert_eq!(key.name(), Some("phone"));

// Revoke a compromised or retired key
identity.revoke_key(&phone_key).await?;

let auth = identity.keys().await?;
let key = auth.get_key_by_pubkey(&phone_key)?;
assert_eq!(key.status(), &KeyStatus::Revoked);
# Ok(())
# }
```

Device keys do not all need the same permission level. At least one key must have `Admin` access to manage the identity database, but other device keys can have lower permissions depending on the trust level of the device.

## Using Identity for Database Access

### Setting Up Delegation

`as_delegation` produces a `DelegatedTreeRef` that you add to a target database's auth settings. Any key in the identity can then access the target database, with permissions clamped by the delegation bounds.

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, Database, backend::database::Sqlite};
# use eidetica::auth::crypto::PrivateKey;
# use eidetica::auth::types::{Permission, PermissionBounds};
# use eidetica::crdt::Doc;
# use eidetica::database::DatabaseKey;
# use eidetica::store::SettingsStore;
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
# let identity = user.create_identity("personal", &default_key).await?;
#
# // Create a target database with a separate admin key
# let admin_key = PrivateKey::generate();
# let target_db = Database::create(&instance, admin_key.clone(), Doc::new()).await?;
#
// Create a delegation reference from the identity
let delegation = identity.as_delegation(PermissionBounds {
    max: Permission::Write(10),
    min: None,
}).await?;

// Add it to the target database's auth settings
# let admin_db_key = DatabaseKey::new(admin_key);
# let target_db = Database::open(instance.clone(), target_db.root_id(), admin_db_key).await?;
let txn = target_db.new_transaction().await?;
txn.get_settings()?.add_delegated_tree(delegation).await?;
txn.commit().await?;
# Ok(())
# }
```

### Opening a Delegated Database

Once delegation is set up, `Identity::open_database` opens the target database directly:

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, Database, backend::database::Sqlite};
# use eidetica::auth::crypto::PrivateKey;
# use eidetica::auth::types::{Permission, PermissionBounds};
# use eidetica::crdt::Doc;
# use eidetica::database::DatabaseKey;
# use eidetica::store::{DocStore, SettingsStore};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
# let identity = user.create_identity("personal", &default_key).await?;
# let admin_key = PrivateKey::generate();
# let target_db = Database::create(&instance, admin_key.clone(), Doc::new()).await?;
# let target_root_id = target_db.root_id().clone();
# let delegation = identity.as_delegation(PermissionBounds {
#     max: Permission::Write(10),
#     min: None,
# }).await?;
# let admin_db_key = DatabaseKey::new(admin_key);
# let target_db_admin = Database::open(instance.clone(), &target_root_id, admin_db_key).await?;
# let txn = target_db_admin.new_transaction().await?;
# txn.get_settings()?.add_delegated_tree(delegation).await?;
# txn.commit().await?;
#
// Open the target database directly through the identity
let db = identity.open_database(&target_root_id).await?;

// Write through the delegation
let txn = db.new_transaction().await?;
let store = txn.get_store::<DocStore>("data").await?;
store.set("hello", "world").await?;
txn.commit().await?;
# Ok(())
# }
```

No manual `SigKey::Delegation` construction is needed — `Identity::open_database` calls `Database::find_sigkeys` internally, which discovers both direct keys and single-hop delegation paths.

The older two-step approach via `user.identity_key()` and `user.open_database_with_key()` remains available for advanced use cases where you need more control.

## Listing and Removing Identities

```rust
# extern crate eidetica;
# extern crate tokio;
# use eidetica::{Instance, backend::database::Sqlite};
#
# #[tokio::main]
# async fn main() -> eidetica::Result<()> {
# let backend = Sqlite::in_memory().await?;
# let instance = Instance::open(Box::new(backend)).await?;
# instance.create_user("alice", None).await?;
# let mut user = instance.login_user("alice", None).await?;
# let default_key = user.get_default_key()?;
# user.create_identity("personal", &default_key).await?;
# user.create_identity("work", &default_key).await?;
#
// List all locally registered identities
let identities = user.identities().await?;
assert_eq!(identities.len(), 2);

// Get the root ID of a named identity
let personal_id = user.identity_id("personal").await?;
assert!(personal_id.is_some());

// Remove an identity from local tracking
// (The underlying database is NOT deleted)
user.remove_identity("work").await?;

let identities = user.identities().await?;
assert_eq!(identities.len(), 1);

// The identity database itself still exists
let work_id = user.identity_id("work").await?;
assert!(work_id.is_none());
# Ok(())
# }
```

## Troubleshooting

**`get_identity` returns `None`**: The bootstrap request has not been approved yet, or the approval has not synced to this device. Sync the identity database and try again.

**`NoKeyInIdentity` error from `identity_key`**: None of the user's local private keys exist in the named identity's auth settings. This happens when looking up an identity that belongs to a different user, or before a bootstrap request has been approved.

**`NoSigKeyFound` error from `open_database_with_key`**: The given key has no access to the target database — neither directly nor through delegation. Verify that the identity's delegation has been added to the target database's auth settings.

**`IdentityAlreadyExists` error**: An identity with this name is already registered locally. Use `get_identity` to retrieve the existing one, or `remove_identity` first if you want to re-register.

## See Also

- [Identity Database Design](../design/identity_database.md) — Architecture and API reference
- [Authentication Guide](authentication_guide.md) — Delegation, permissions, and key management
- [Synchronization Guide](synchronization_guide.md) — Syncing databases across devices
- [Bootstrapping](bootstrap.md) — How new devices gain access to databases
