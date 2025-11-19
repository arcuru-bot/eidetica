use std::{
    any::Any,
    collections::HashMap,
    fmt,
    marker::PhantomData,
    sync::{Arc, RwLock},
};

use once_cell::sync::Lazy;

use crate::{Database, Result, Transaction};

use super::{Store, StoreError, SubtreeInfo};

type DefaultConfigFn = Arc<dyn Fn() -> String + Send + Sync>;
type StoreTxFactory =
    Arc<dyn Fn(&Transaction, &str, &SubtreeInfo) -> Result<BoxedStoreHandle> + Send + Sync>;
type StoreViewerFactory =
    Arc<dyn Fn(&Database, &str, &SubtreeInfo) -> Result<BoxedStoreHandle> + Send + Sync>;

/// Registry that maps `_index` type identifiers to constructors for Store implementations.
///
/// Registrations are thread-safe and can be extended by embedders prior to opening databases.
#[derive(Clone, Debug)]
pub struct StoreRegistry {
    registrations: Arc<RwLock<HashMap<String, StoreRegistration>>>,
}

impl StoreRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            registrations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a clone of the global registry initialized with all built-in store types.
    pub fn global() -> Self {
        static GLOBAL: Lazy<StoreRegistry> = Lazy::new(|| {
            let registry = StoreRegistry::new();
            super::register_builtin_stores(&registry);
            registry
        });

        GLOBAL.clone()
    }

    /// Register a new store type with the registry.
    pub fn register(&self, registration: StoreRegistration) -> Result<()> {
        let mut guard = self
            .registrations
            .write()
            .expect("StoreRegistry write lock poisoned");

        if guard.contains_key(registration.type_id()) {
            return Err(StoreError::RegistrationConflict {
                type_id: registration.type_id().to_string(),
            }
            .into());
        }

        guard.insert(registration.type_id().to_string(), registration);
        Ok(())
    }

    /// Check if a type identifier has been registered.
    pub fn is_registered(&self, type_id: &str) -> bool {
        self.registrations
            .read()
            .expect("StoreRegistry read lock poisoned")
            .contains_key(type_id)
    }

    /// Retrieve a registration by type identifier.
    pub fn get(&self, type_id: &str) -> Option<StoreRegistration> {
        self.registrations
            .read()
            .expect("StoreRegistry read lock poisoned")
            .get(type_id)
            .cloned()
    }

    /// Open a store handle within the provided transaction using registry metadata.
    pub fn open_with_tx(
        &self,
        tx: &Transaction,
        subtree_name: &str,
        info: &SubtreeInfo,
    ) -> Result<BoxedStoreHandle> {
        let registration = self
            .get(&info.type_id)
            .ok_or_else(|| StoreError::UnknownStoreType {
                type_id: info.type_id.clone(),
            })?;

        registration.open_with_tx(tx, subtree_name, info)
    }

    /// Open a read-only store handle sourced from the provided database.
    pub fn open_viewer(
        &self,
        db: &Database,
        subtree_name: &str,
        info: &SubtreeInfo,
    ) -> Result<BoxedStoreHandle> {
        let registration = self
            .get(&info.type_id)
            .ok_or_else(|| StoreError::UnknownStoreType {
                type_id: info.type_id.clone(),
            })?;

        registration.open_viewer(db, subtree_name, info)
    }
}

/// Metadata required to instantiate a store from the registry.
#[derive(Clone)]
pub struct StoreRegistration {
    type_id: String,
    default_config: DefaultConfigFn,
    tx_factory: StoreTxFactory,
    viewer_factory: StoreViewerFactory,
}

impl fmt::Debug for StoreRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreRegistration")
            .field("type_id", &self.type_id)
            .finish()
    }
}

impl StoreRegistration {
    /// Create a builder for the specified [`Store`] implementation.
    pub fn for_store<T>() -> StoreRegistrationBuilder<T>
    where
        T: Store + 'static,
    {
        StoreRegistrationBuilder::new()
    }

    /// Type identifier recorded for this store.
    pub fn type_id(&self) -> &str {
        &self.type_id
    }

    /// Default configuration for this store type.
    pub fn default_config(&self) -> String {
        (self.default_config)()
    }

    fn open_with_tx(
        &self,
        tx: &Transaction,
        subtree_name: &str,
        info: &SubtreeInfo,
    ) -> Result<BoxedStoreHandle> {
        (self.tx_factory)(tx, subtree_name, info)
    }

    fn open_viewer(
        &self,
        db: &Database,
        subtree_name: &str,
        info: &SubtreeInfo,
    ) -> Result<BoxedStoreHandle> {
        (self.viewer_factory)(db, subtree_name, info)
    }
}

/// Builder helper for [`StoreRegistration`].
pub struct StoreRegistrationBuilder<T>
where
    T: Store + 'static,
{
    default_config: DefaultConfigFn,
    tx_factory: Option<StoreTxFactory>,
    viewer_factory: Option<StoreViewerFactory>,
    _marker: PhantomData<T>,
}

impl<T> StoreRegistrationBuilder<T>
where
    T: Store + 'static,
{
    fn new() -> Self {
        Self {
            default_config: Arc::new(|| T::default_config()),
            tx_factory: None,
            viewer_factory: None,
            _marker: PhantomData,
        }
    }

    /// Override the default configuration generator.
    pub fn with_default_config<F>(mut self, generator: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.default_config = Arc::new(generator);
        self
    }

    /// Override the transactional constructor.
    pub fn with_tx_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn(&Transaction, &str, &SubtreeInfo) -> Result<BoxedStoreHandle> + Send + Sync + 'static,
    {
        self.tx_factory = Some(Arc::new(factory));
        self
    }

    /// Override the viewer constructor.
    pub fn with_viewer_factory<F>(mut self, factory: F) -> Self
    where
        F: Fn(&Database, &str, &SubtreeInfo) -> Result<BoxedStoreHandle> + Send + Sync + 'static,
    {
        self.viewer_factory = Some(Arc::new(factory));
        self
    }

    /// Build the registration entry.
    pub fn build(self) -> StoreRegistration {
        let tx_factory = self.tx_factory.unwrap_or_else(|| {
            Arc::new(|tx, subtree, info| {
                let store = T::new(tx, subtree.to_string())?;
                Ok(BoxedStoreHandle::new(subtree, info.clone(), store))
            })
        });

        let viewer_factory = self.viewer_factory.unwrap_or_else(|| {
            Arc::new(|db, subtree, info| {
                let tx = db.new_transaction()?;
                let store = T::new(&tx, subtree.to_string())?;
                Ok(BoxedStoreHandle::new(subtree, info.clone(), store))
            })
        });

        StoreRegistration {
            type_id: T::type_id().to_string(),
            default_config: self.default_config,
            tx_factory,
            viewer_factory,
        }
    }
}

/// Dynamic store handle that can be downcast into a specific [`Store`] implementation.
pub struct BoxedStoreHandle {
    type_id: String,
    subtree: String,
    info: SubtreeInfo,
    inner: Box<dyn Any>,
}

impl fmt::Debug for BoxedStoreHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxedStoreHandle")
            .field("type_id", &self.type_id)
            .field("subtree", &self.subtree)
            .finish()
    }
}

impl BoxedStoreHandle {
    fn new<T>(subtree: impl Into<String>, info: SubtreeInfo, store: T) -> Self
    where
        T: Any + 'static,
    {
        Self {
            type_id: info.type_id.clone(),
            subtree: subtree.into(),
            info,
            inner: Box::new(store),
        }
    }

    /// The runtime store type identifier derived from `_index`.
    pub fn type_id(&self) -> &str {
        &self.type_id
    }

    /// Subtree name associated with this handle.
    pub fn subtree(&self) -> &str {
        &self.subtree
    }

    /// Stored metadata from `_index`.
    pub fn info(&self) -> &SubtreeInfo {
        &self.info
    }

    /// Downcast the handle into a concrete [`Store`] implementation.
    pub fn downcast<T>(self) -> Result<T>
    where
        T: Store + 'static,
    {
        match self.inner.downcast::<T>() {
            Ok(store) => Ok(*store),
            Err(_) => Err(StoreError::TypeMismatch {
                store: self.subtree.clone(),
                expected: T::type_id().to_string(),
                actual: self.type_id,
            }
            .into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::database::InMemory,
        crdt::Doc,
        instance::LegacyInstanceOps,
        store::{DocStore, Store},
    };

    #[test]
    fn global_registry_contains_docstore() {
        let registry = StoreRegistry::global();
        assert!(registry.is_registered(<DocStore as Store>::type_id()));
    }

    #[test]
    fn registry_opens_docstore_with_transaction() {
        let backend = Box::new(InMemory::new());
        let instance = crate::Instance::open(backend).unwrap();
        instance.add_private_key("TEST_KEY").unwrap();
        let db = instance.new_database(Doc::new(), "TEST_KEY").unwrap();

        let tx = db.new_transaction().unwrap();
        let store = tx.get_store::<DocStore>("data").unwrap();
        store.set("key", "value").unwrap();
        tx.commit().unwrap();

        // Fetch metadata from _index
        let info = {
            let check_tx = db.new_transaction().unwrap();
            let index = check_tx.get_index_store().unwrap();
            index.get_subtree_info("data").unwrap()
        };

        let registry = StoreRegistry::global();
        let handle = registry
            .open_viewer(&db, "data", &info)
            .expect("viewer registry lookup failed");
        assert_eq!(handle.type_id(), <DocStore as Store>::type_id());
        let doc_store: DocStore = handle.downcast().unwrap();
        assert_eq!(doc_store.get("key").unwrap().as_text(), Some("value"));
    }
}
