//! Integration tests for the Identity Database feature

use eidetica::{
    Result,
    auth::{
        crypto::{PrivateKey, PublicKey},
        types::{AuthKey, KeyStatus, Permission, PermissionBounds},
    },
    crdt::Doc,
    database::DatabaseKey,
    store::DocStore,
    user::{IdentityStatus, TrackedIdentity, UserError},
};

use super::helpers::{login_user, setup_instance};

// ===== CREATION TESTS =====

#[tokio::test]
async fn test_create_identity_returns_active_identity() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;

    // Should have a root ID
    assert!(!identity.root_id().as_str().is_empty());

    // Should be accessible via Deref (Database methods)
    let _tips = identity.get_tips().await?;

    Ok(())
}

#[tokio::test]
async fn test_create_identity_has_global_read_permission() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;
    let auth = identity.keys().await?;

    // Should have global read permission
    let global = auth.get_global_key()?;
    assert_eq!(global.permissions(), &Permission::Read);
    assert_eq!(global.status(), &KeyStatus::Active);

    Ok(())
}

#[tokio::test]
async fn test_create_identity_has_admin_key() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    let default_key = user.get_default_key()?;
    let identity = user.create_identity("personal", &default_key).await?;
    let auth = identity.keys().await?;

    // The default key should be Admin(0)
    let key = auth.get_key_by_pubkey(&default_key)?;
    assert_eq!(key.permissions(), &Permission::Admin(0));
    assert_eq!(key.status(), &KeyStatus::Active);

    Ok(())
}

#[tokio::test]
async fn test_create_identity_duplicate_name_fails() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    user.create_identity("personal", &default_key).await?;
    let result = user.create_identity("personal", &default_key).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            eidetica::Error::User(UserError::IdentityAlreadyExists { .. })
        ),
        "Expected IdentityAlreadyExists, got: {err:?}"
    );

    Ok(())
}

// ===== RETRIEVAL TESTS =====

#[tokio::test]
async fn test_get_identity_returns_some_for_active() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let created = user.create_identity("personal", &default_key).await?;
    let created_root_id = created.root_id().clone();

    let retrieved = user.get_identity("personal").await?;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().root_id(), &created_root_id);

    Ok(())
}

#[tokio::test]
async fn test_get_identity_returns_none_for_unknown() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    let result = user.get_identity("nonexistent").await?;
    assert!(result.is_none());

    Ok(())
}

// ===== KEY MANAGEMENT TESTS =====

#[tokio::test]
async fn test_identity_add_key_revoke_key_lifecycle() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;

    // Add a new key
    let new_pubkey = PublicKey::random();
    identity
        .add_key(
            &new_pubkey,
            AuthKey::active(Some("phone"), Permission::Write(10)),
        )
        .await?;

    // Verify key exists
    let auth = identity.keys().await?;
    let key = auth.get_key_by_pubkey(&new_pubkey)?;
    assert_eq!(key.permissions(), &Permission::Write(10));
    assert_eq!(key.status(), &KeyStatus::Active);
    assert_eq!(key.name(), Some("phone"));

    // Revoke the key
    identity.revoke_key(&new_pubkey).await?;

    // Verify revoked
    let auth = identity.keys().await?;
    let key = auth.get_key_by_pubkey(&new_pubkey)?;
    assert_eq!(key.status(), &KeyStatus::Revoked);

    Ok(())
}

// ===== DELEGATION TESTS =====

#[tokio::test]
async fn test_identity_as_delegation_produces_valid_ref() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;
    let bounds = PermissionBounds {
        max: Permission::Write(10),
        min: None,
    };
    let delegation = identity.as_delegation(bounds.clone()).await?;

    // Verify delegation ref
    assert_eq!(delegation.tree.root, *identity.root_id());
    assert!(!delegation.tree.tips.is_empty());
    assert_eq!(delegation.permission_bounds, bounds);

    Ok(())
}

#[tokio::test]
async fn test_identity_delegation_enables_writes_to_target_db() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    // Create identity
    let default_key = user.get_default_key()?;
    let identity = user.create_identity("personal", &default_key).await?;

    // Create target database with a separate admin key (not in user's keyring)
    let admin_signing_key = PrivateKey::generate();
    let target_db =
        eidetica::Database::create(&instance, admin_signing_key.clone(), Doc::new()).await?;

    // Add delegation from identity to target database
    let delegation = identity
        .as_delegation(PermissionBounds {
            max: Permission::Write(10),
            min: None,
        })
        .await?;

    let admin_db_key = DatabaseKey::new(admin_signing_key.clone());
    let target_db_authed =
        eidetica::Database::open(instance.clone(), target_db.root_id(), admin_db_key).await?;
    let txn = target_db_authed.new_transaction().await?;
    txn.get_settings()?.add_delegated_tree(delegation).await?;
    txn.commit().await?;

    // Open target database via delegation using the streamlined API
    let key = user.identity_key("personal").await?;
    let delegated_db = user
        .open_database_with_key(target_db.root_id(), &key)
        .await?;

    // Write data via delegation
    let txn = delegated_db.new_transaction().await?;
    let store = txn.get_store::<DocStore>("data").await?;
    store.set("hello", "world").await?;
    txn.commit().await?;

    // Verify data was written
    let viewer = target_db_authed
        .get_store_viewer::<DocStore>("data")
        .await?;
    let value = viewer.get_string("hello").await?;
    assert_eq!(value, "world");

    Ok(())
}

// ===== TRACKING TESTS =====

#[tokio::test]
async fn test_remove_identity_removes_tracking() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;
    let root_id = identity.root_id().clone();

    // Remove tracking
    user.remove_identity("personal").await?;

    // get_identity returns None
    let result = user.get_identity("personal").await?;
    assert!(result.is_none());

    // But the underlying database still exists and is accessible
    let db = user.open_database(&root_id).await?;
    assert_eq!(db.root_id(), &root_id);

    Ok(())
}

#[tokio::test]
async fn test_remove_identity_unknown_name_fails() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    let result = user.remove_identity("nonexistent").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            eidetica::Error::User(UserError::IdentityNotFound { .. })
        ),
        "Expected IdentityNotFound, got: {err:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_identities_lists_all_with_correct_status() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    user.create_identity("personal", &default_key).await?;
    user.create_identity("work", &default_key).await?;

    let identities = user.identities().await?;
    assert_eq!(identities.len(), 2);

    // Both should be active
    for (_, value) in identities.iter() {
        let tracked = TrackedIdentity::try_from(value).unwrap();
        assert_eq!(tracked.status, IdentityStatus::Active);
    }

    // Both names should be present
    let names: Vec<&str> = identities.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"personal"));
    assert!(names.contains(&"work"));

    Ok(())
}

#[tokio::test]
async fn test_identity_id_returns_root_id() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;
    let expected_id = identity.root_id().clone();

    let id = user.identity_id("personal").await?;
    assert_eq!(id, Some(expected_id));

    Ok(())
}

#[tokio::test]
async fn test_identity_id_returns_none_for_unknown() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let user = login_user(&instance, "alice", None).await;

    let id = user.identity_id("nonexistent").await?;
    assert!(id.is_none());

    Ok(())
}

// ===== MULTIPLE IDENTITIES TESTS =====

#[tokio::test]
async fn test_create_multiple_identities() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let personal = user.create_identity("personal", &default_key).await?;
    let work = user.create_identity("work", &default_key).await?;

    // Different root IDs
    assert_ne!(personal.root_id(), work.root_id());

    Ok(())
}

// ===== OPEN DATABASE WITH KEY TESTS =====

#[tokio::test]
async fn test_open_database_with_key_direct_access() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    // Create a database (user's key is directly in auth settings)
    let default_key = user.get_default_key()?;
    let db = user.create_database(Doc::new(), &default_key).await?;

    // open_database_with_key should work for direct keys too
    let opened = user
        .open_database_with_key(db.root_id(), &default_key)
        .await?;

    // Write to verify it works
    let txn = opened.new_transaction().await?;
    let store = txn.get_store::<DocStore>("data").await?;
    store.set("key", "value").await?;
    txn.commit().await?;

    let viewer = db.get_store_viewer::<DocStore>("data").await?;
    let value = viewer.get_string("key").await?;
    assert_eq!(value, "value");

    Ok(())
}

#[tokio::test]
async fn test_open_database_with_key_no_access_fails() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let user = login_user(&instance, "alice", None).await;

    // Create a database with a separate admin key (alice has no access)
    let admin_key = PrivateKey::generate();
    let db = eidetica::Database::create(&instance, admin_key, Doc::new()).await?;

    let default_key = user.get_default_key()?;
    let result = user
        .open_database_with_key(db.root_id(), &default_key)
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, eidetica::Error::User(UserError::NoSigKeyFound { .. })),
        "Expected NoSigKeyFound, got: {err:?}"
    );

    Ok(())
}

// ===== IDENTITY KEY TESTS =====

#[tokio::test]
async fn test_identity_key_returns_correct_key() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    user.create_identity("personal", &default_key).await?;

    let found_key = user.identity_key("personal").await?;
    assert_eq!(found_key, default_key);

    Ok(())
}

#[tokio::test]
async fn test_identity_key_unknown_name_fails() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let user = login_user(&instance, "alice", None).await;

    let result = user.identity_key("nonexistent").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            eidetica::Error::User(UserError::IdentityNotFound { .. })
        ),
        "Expected IdentityNotFound, got: {err:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_identity_key_no_local_key_fails() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;

    // Create identity with a key that won't be in bob's key manager
    let alice_key = user.get_default_key()?;
    let identity = user.create_identity("personal", &alice_key).await?;
    let identity_root = identity.root_id().clone();

    // Create a different user (bob) and manually track alice's identity
    instance.create_user("bob", None).await?;
    let bob = login_user(&instance, "bob", None).await;

    // Manually track Alice's identity in Bob's user database
    let tx = bob.user_database().new_transaction().await?;
    let identities_store = tx.get_store::<DocStore>("identities").await?;
    let tracked = TrackedIdentity {
        root_id: identity_root.clone(),
        status: IdentityStatus::Active,
    };
    identities_store.set("alice-identity", tracked).await?;
    tx.commit().await?;

    // Bob has no key in Alice's identity
    let result = bob.identity_key("alice-identity").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            eidetica::Error::User(UserError::NoKeyInIdentity { .. })
        ),
        "Expected NoKeyInIdentity, got: {err:?}"
    );

    Ok(())
}

// ===== TICKET TESTS =====

#[tokio::test]
async fn test_identity_ticket_contains_root_id() -> Result<()> {
    let instance = setup_instance().await;
    instance.create_user("alice", None).await?;
    let mut user = login_user(&instance, "alice", None).await;
    let default_key = user.get_default_key()?;

    let identity = user.create_identity("personal", &default_key).await?;
    let ticket = identity.ticket();

    assert_eq!(ticket.database_id(), identity.root_id());

    Ok(())
}
