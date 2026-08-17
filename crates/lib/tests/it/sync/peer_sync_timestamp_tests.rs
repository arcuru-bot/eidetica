//! `PeerInfo::last_successful_sync` tracks the last sync that actually moved data.
//!
//! The field is the freshness signal a caller ranks peers on — a peer that has
//! served something recently is the one most likely to answer next — so it has
//! to advance on every path that can succeed, and stay unset on a peer that has
//! only ever been registered.

use eidetica::{Result, auth::Permission, crdt::Doc, entry::ID, sync::Sync, testing::Cluster};

use super::helpers::{cluster_put, cluster_shared_database};

/// The timestamp `sync` holds for the peer identified by `peer_pubkey`.
async fn last_successful_sync(sync: &Sync, peer_pubkey: &eidetica::PublicKey) -> Option<String> {
    sync.get_peer_info(peer_pubkey)
        .await
        .expect("peer info lookup failed")
        .expect("peer is not registered")
        .last_successful_sync
}

/// A registered peer that has never been synced with carries no timestamp.
#[tokio::test]
async fn registered_peer_has_no_sync_timestamp() -> Result<()> {
    let net = Cluster::builder().peers(2).build().await?;

    let peer0 = net.peer(0).sync();
    let peer1_pubkey = net.peer(1).sync().get_device_pubkey()?;
    peer0.register_peer(&peer1_pubkey, Some("peer1")).await?;

    assert_eq!(
        last_successful_sync(peer0, &peer1_pubkey).await,
        None,
        "registration alone is not a sync"
    );
    Ok(())
}

/// Bootstrapping a database records a successful sync against the peer served it.
#[tokio::test]
async fn bootstrap_records_sync_timestamp() -> Result<()> {
    let mut net = Cluster::builder().peers(2).build().await?;
    let peer0_pubkey = net.peer(0).sync().get_device_pubkey()?;

    // Peer 1 bootstraps the shared database from peer 0.
    let (_room, _dbs) = cluster_shared_database(&mut net, "Bootstrap Timestamp").await?;

    assert!(
        last_successful_sync(net.peer(1).sync(), &peer0_pubkey)
            .await
            .is_some(),
        "bootstrap pulled a whole database and must count as a successful sync"
    );
    Ok(())
}

/// A tree exchange records a sync, and a later exchange advances the timestamp.
#[tokio::test]
async fn exchange_advances_sync_timestamp() -> Result<()> {
    let mut net = Cluster::builder().peers(2).build().await?;
    let peer0_pubkey = net.peer(0).sync().get_device_pubkey()?;

    let (room, _dbs) = cluster_shared_database(&mut net, "Exchange Timestamp").await?;
    let after_bootstrap = last_successful_sync(net.peer(1).sync(), &peer0_pubkey)
        .await
        .expect("bootstrap recorded a timestamp");

    net.exchange(1, 0, &room).await?;
    let after_exchange = last_successful_sync(net.peer(1).sync(), &peer0_pubkey)
        .await
        .expect("exchange recorded a timestamp");

    assert_ne!(
        after_bootstrap, after_exchange,
        "a second successful sync must advance the timestamp, not leave the first one standing"
    );
    Ok(())
}

/// Pushing entries to a peer records a successful sync against the receiver.
///
/// This is the background engine's send path rather than the caller-driven one:
/// the entry is queued by the sync-on-commit callback and only reaches the wire
/// when the queue is flushed.
#[tokio::test]
async fn entry_push_records_sync_timestamp() -> Result<()> {
    let mut net = Cluster::builder().peers(2).build().await?;
    let peer1_pubkey = net.peer(1).sync().get_device_pubkey()?;

    let (room, dbs) = cluster_shared_database(&mut net, "Push Timestamp").await?;
    net.auto_sync(0, 1, &room).await?;
    net.flush_all().await?;

    let before = last_successful_sync(net.peer(0).sync(), &peer1_pubkey).await;

    cluster_put(&dbs[0], "pushed", "value").await?;
    net.flush(0).await?;

    let after = last_successful_sync(net.peer(0).sync(), &peer1_pubkey)
        .await
        .expect("a delivered entry batch is a successful sync");
    assert_ne!(
        before.as_deref(),
        Some(after.as_str()),
        "the push must record its own timestamp"
    );
    Ok(())
}

/// A sync that fails leaves the last successful timestamp untouched.
#[tokio::test]
async fn failed_sync_leaves_timestamp_untouched() -> Result<()> {
    let mut net = Cluster::builder().peers(2).build().await?;
    let peer0_pubkey = net.peer(0).sync().get_device_pubkey()?;

    let (_room, _dbs) = cluster_shared_database(&mut net, "Failed Timestamp").await?;
    let recorded = last_successful_sync(net.peer(1).sync(), &peer0_pubkey)
        .await
        .expect("bootstrap recorded a timestamp");

    // A database only peer 1 holds cannot be synced from peer 0.
    let key1 = net.peer(1).key_id().clone();
    let mut settings = Doc::new();
    settings.set("name", "Peer 1 Only");
    let local = net
        .peer_mut(1)
        .user_mut()
        .create_database(settings, &key1)
        .await?;
    let unknown: ID = local.root_id().clone();

    let result = net
        .peer(1)
        .sync()
        .sync_tree_with_peer_auth(&peer0_pubkey, &unknown, None, None, Some(Permission::Read))
        .await;
    assert!(result.is_err(), "syncing an unknown database must fail");

    assert_eq!(
        last_successful_sync(net.peer(1).sync(), &peer0_pubkey).await,
        Some(recorded),
        "a failed sync must not advance the timestamp"
    );
    Ok(())
}
