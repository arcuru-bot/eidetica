//! A backend written before the record-based Store-state substrate keeps
//! working: its default record methods report "unsupported", and transaction
//! reads fall back to folding history instead of failing.
//!
//! `Recordless` wraps a real engine and forwards every entry-addressed method,
//! deliberately leaving the record methods at their defaults — the shape of an
//! old custom [`BackendImpl`](eidetica::backend::BackendImpl).

use std::any::Any;

use eidetica::{
    Database, Instance, NewUser, Result, Snapshot,
    auth::crypto::generate_keypair,
    backend::database::InMemory,
    backend::{BackendImpl, CacheScope, InstanceMetadata, InstanceSecrets, VerificationStatus},
    crdt::Doc,
    entry::{Entry, ID},
    store::DocStore,
};

/// A pre-record-substrate backend: full entry storage, no record support.
struct Recordless<B>(B);

#[async_trait::async_trait]
impl<B: BackendImpl> BackendImpl for Recordless<B> {
    async fn get(&self, id: &ID) -> Result<Entry> {
        self.0.get(id).await
    }

    async fn get_verification_status(&self, id: &ID) -> Result<VerificationStatus> {
        self.0.get_verification_status(id).await
    }

    async fn put(&self, entry: Entry) -> Result<()> {
        self.0.put(entry).await
    }

    async fn update_verification_status(
        &self,
        id: &ID,
        verification_status: VerificationStatus,
    ) -> Result<()> {
        self.0
            .update_verification_status(id, verification_status)
            .await
    }

    async fn get_entries_by_verification_status(
        &self,
        status: VerificationStatus,
    ) -> Result<Vec<ID>> {
        self.0.get_entries_by_verification_status(status).await
    }

    async fn snapshot(&self, tree: &ID) -> Result<Snapshot> {
        self.0.snapshot(tree).await
    }

    async fn store_snapshot(&self, tree: &ID, store: &str) -> Result<Snapshot> {
        self.0.store_snapshot(tree, store).await
    }

    async fn store_snapshot_at(
        &self,
        tree: &ID,
        store: &str,
        main_snapshot: &Snapshot,
    ) -> Result<Snapshot> {
        self.0.store_snapshot_at(tree, store, main_snapshot).await
    }

    async fn all_roots(&self) -> Result<Vec<ID>> {
        self.0.all_roots().await
    }

    async fn find_merge_base(
        &self,
        tree: &ID,
        store: &str,
        entry_ids: &[ID],
    ) -> Result<Option<ID>> {
        self.0.find_merge_base(tree, store, entry_ids).await
    }

    fn as_any(&self) -> &dyn Any {
        self.0.as_any()
    }

    async fn get_tree(&self, tree: &ID) -> Result<Vec<Entry>> {
        self.0.get_tree(tree).await
    }

    async fn get_store(&self, tree: &ID, store: &str) -> Result<Vec<Entry>> {
        self.0.get_store(tree, store).await
    }

    async fn get_tree_from_tips(&self, tree: &ID, tips: &[ID]) -> Result<Vec<Entry>> {
        self.0.get_tree_from_tips(tree, tips).await
    }

    async fn store_at(&self, tree: &ID, store: &str, snapshot: &Snapshot) -> Result<Vec<Entry>> {
        self.0.store_at(tree, store, snapshot).await
    }

    async fn get_cached_crdt_state(
        &self,
        scope: &CacheScope,
        entry_id: &ID,
        store: &str,
    ) -> Result<Option<Vec<u8>>> {
        self.0.get_cached_crdt_state(scope, entry_id, store).await
    }

    async fn cache_crdt_state(
        &self,
        scope: CacheScope,
        entry_id: &ID,
        store: &str,
        state: Vec<u8>,
    ) -> Result<()> {
        self.0.cache_crdt_state(scope, entry_id, store, state).await
    }

    async fn clear_crdt_cache(&self) -> Result<()> {
        self.0.clear_crdt_cache().await
    }

    async fn get_sorted_store_parents(
        &self,
        tree_id: &ID,
        entry_id: &ID,
        store: &str,
    ) -> Result<Vec<ID>> {
        self.0
            .get_sorted_store_parents(tree_id, entry_id, store)
            .await
    }

    async fn get_path_from_to(
        &self,
        tree_id: &ID,
        store: &str,
        from_id: Option<&ID>,
        to_ids: &[ID],
    ) -> Result<Vec<ID>> {
        self.0
            .get_path_from_to(tree_id, store, from_id, to_ids)
            .await
    }

    async fn get_instance_metadata(&self) -> Result<Option<InstanceMetadata>> {
        self.0.get_instance_metadata().await
    }

    async fn set_instance_metadata(&self, metadata: &InstanceMetadata) -> Result<()> {
        self.0.set_instance_metadata(metadata).await
    }

    async fn get_instance_secrets(&self) -> Result<Option<InstanceSecrets>> {
        self.0.get_instance_secrets().await
    }

    async fn set_instance_secrets(&self, secrets: &InstanceSecrets) -> Result<()> {
        self.0.set_instance_secrets(secrets).await
    }

    // Record-based Store-state methods keep their defaults, which report
    // `StoreStateStorageUnsupported` — exactly what an old custom backend does.
}

/// Writes land and reads come back through real transactions even though the
/// backend has no record substrate: materialization folds history instead of
/// failing on the unsupported capability. A genuine storage failure must still
/// surface — this backend forwards every entry call, so only the record
/// capability is missing, and the reads below would fail if the fallback
/// swallowed real errors.
#[tokio::test]
async fn old_backend_without_record_support_reads_through_history() {
    let (instance, _admin) = Instance::create_backend(
        Box::new(Recordless(InMemory::new())),
        NewUser::passwordless("admin"),
    )
    .await
    .unwrap();
    let (private_key, _) = generate_keypair();
    let database = Database::create(&instance, private_key, Doc::new())
        .await
        .unwrap();

    for (key, value) in [("alpha", "1"), ("beta", "2")] {
        let tx = database.new_transaction().await.unwrap();
        let store = tx.get_store::<DocStore>("data").await.unwrap();
        store.set(key, value).await.unwrap();
        tx.commit().await.unwrap();
    }

    // Cold read, then a second read over the same history.
    for _ in 0..2 {
        let viewer = database.get_store_viewer::<DocStore>("data").await.unwrap();
        assert_eq!(viewer.get_string("alpha").await.unwrap(), "1");
        assert_eq!(viewer.get_string("beta").await.unwrap(), "2");
    }
}
