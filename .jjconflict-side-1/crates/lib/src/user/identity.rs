//! Identity database wrapper
//!
//! An `Identity` wraps a `Database` whose `_settings.auth` contains a user's device keys.
//! Its root ID serves as a stable, shareable identity address. Other databases delegate
//! to it via `DelegatedTreeRef` rather than tracking individual device keys.

use std::ops::Deref;

use crate::{
    Database, Result,
    auth::{
        crypto::{PrivateKey, PublicKey},
        settings::AuthSettings,
        types::{AuthKey, DelegatedTreeRef, PermissionBounds, TreeReference},
    },
    database::DatabaseKey,
    entry::ID,
    store::{DocStore, SettingsStore},
    sync::DatabaseTicket,
    user::{IdentityStatus, TrackedIdentity, UserError},
};

/// A user identity backed by a dedicated database.
///
/// The identity database's `_settings.auth` contains entries for each device key the user
/// controls. The root ID is the user's stable identity address.
///
/// `Identity` dereferences to `Database`, so all database operations are available directly.
/// `Identity` carries the signing key needed to open databases that delegate to it,
/// enabling `identity.open_database(&target_root_id)` without going back through `User`.
#[derive(Debug)]
pub struct Identity {
    database: Database,
    key_id: PublicKey,
    signing_key: PrivateKey,
    name: String,
    user_database: Database,
}

impl Deref for Identity {
    type Target = Database;

    fn deref(&self) -> &Database {
        &self.database
    }
}

impl Identity {
    /// Create an Identity from an existing database with its associated key.
    pub(crate) fn new(
        database: Database,
        key_id: PublicKey,
        signing_key: PrivateKey,
        name: String,
        user_database: Database,
    ) -> Self {
        Self {
            database,
            key_id,
            signing_key,
            name,
            user_database,
        }
    }

    /// The root ID of this identity (the shareable identity address).
    pub fn root_id(&self) -> &ID {
        self.database.root_id()
    }

    /// The public key associated with this identity on this device.
    pub fn key_id(&self) -> &PublicKey {
        &self.key_id
    }

    /// The tracking name of this identity.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// A reference to the underlying database.
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Change which local key this identity uses.
    ///
    /// Updates both the in-memory identity and the persisted tracking entry
    /// in the user's private database. Use after key rotation or adding a new
    /// device key via `add_key`.
    ///
    /// # Arguments
    /// * `key_id` - The new public key
    /// * `signing_key` - The corresponding private key
    pub async fn set_key(&mut self, key_id: PublicKey, signing_key: PrivateKey) -> Result<()> {
        let tx = self.user_database.new_transaction().await?;
        let identities_store = tx.get_store::<DocStore>("identities").await?;
        let updated = TrackedIdentity {
            root_id: self.database.root_id().clone(),
            status: IdentityStatus::Active,
            key_id: Some(key_id.clone()),
        };
        identities_store.set(&self.name, updated).await?;
        tx.commit().await?;

        self.key_id = key_id;
        self.signing_key = signing_key;

        Ok(())
    }

    /// Open a database that delegates to this identity.
    ///
    /// Discovers the delegation SigKey for this identity's key in the target database
    /// and opens it. This is the preferred way to open databases through an identity,
    /// without going back through `User`.
    ///
    /// # Arguments
    /// * `root_id` - The root entry ID of the target database
    ///
    /// # Errors
    /// - Returns `NoSigKeyFound` if no SigKey (direct or delegation) is found for this identity's key
    pub async fn open_database(&self, root_id: &ID) -> Result<Database> {
        let instance = self.database.instance()?;
        let available = Database::find_sigkeys(&instance, root_id, &self.key_id).await?;
        let (sigkey, _perm) =
            available
                .into_iter()
                .next()
                .ok_or_else(|| UserError::NoSigKeyFound {
                    key_id: self.key_id.to_string(),
                    database_id: root_id.clone(),
                })?;
        let key = DatabaseKey::with_identity(self.signing_key.clone(), sigkey);
        Database::open(instance, root_id, key).await
    }

    /// Add a device key to this identity's auth settings.
    ///
    /// # Arguments
    /// * `pubkey` - The public key of the device to add
    /// * `auth_key` - The `AuthKey` specifying permissions and status for this key
    pub async fn add_key(&self, pubkey: &PublicKey, auth_key: AuthKey) -> Result<()> {
        let tx = self.database.new_transaction().await?;
        let settings = SettingsStore::new(&tx)?;
        settings.set_auth_key(pubkey, auth_key).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revoke a device key in this identity's auth settings.
    ///
    /// # Arguments
    /// * `pubkey` - The public key of the device to revoke
    pub async fn revoke_key(&self, pubkey: &PublicKey) -> Result<()> {
        let tx = self.database.new_transaction().await?;
        let settings = SettingsStore::new(&tx)?;
        settings.revoke_auth_key(pubkey).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Get a snapshot of all keys in this identity's auth settings.
    pub async fn keys(&self) -> Result<AuthSettings> {
        let settings = self.database.get_settings().await?;
        settings.auth_snapshot().await
    }

    /// Produce a `DelegatedTreeRef` pointing to this identity.
    ///
    /// The returned reference can be added to any target database's auth settings
    /// via `SettingsStore::add_delegated_tree()`.
    ///
    /// # Arguments
    /// * `bounds` - Permission bounds to apply when delegating to this identity
    pub async fn as_delegation(&self, bounds: PermissionBounds) -> Result<DelegatedTreeRef> {
        let tips = self.database.get_tips().await?;
        Ok(DelegatedTreeRef {
            permission_bounds: bounds,
            tree: TreeReference {
                root: self.database.root_id().clone(),
                tips,
            },
        })
    }

    /// Create a `DatabaseTicket` for this identity database.
    ///
    /// The ticket contains the identity's root ID and can be shared with other
    /// devices or users to bootstrap into this identity.
    pub fn ticket(&self) -> DatabaseTicket {
        DatabaseTicket::new(self.database.root_id().clone())
    }
}
