# Store Type Registry

## Problem Statement

Eidetica now records the type identifier and configuration of every subtree in the `_index` registry. However, both `Database::get_store_viewer::<T>` and `Transaction::get_store::<T>` still require the caller to know the concrete `Store` type at compile time. Internal components (CLI, sync daemons, automation) frequently need to enumerate existing subtrees, open them dynamically, or provide generic tooling (e.g., migration helpers or inspectors). Today that means the caller must hard-code knowledge of which subtree names use `DocStore`, which use `Table`, and so on, which is brittle and prevents third-party store implementations from integrating cleanly.

We need a centralized way to register the `Store` types shipping with the binary (and, optionally, ones provided by extensions) so that Eidetica can instantiate the right type at runtime based on the `_index` metadata.

## Goals

- Provide a single registry where store types (DocStore, Table, YDoc, future custom stores) register their type identifiers, default configuration, and constructor functions.
- Allow both read-only (`Database`) and transactional (`Transaction`) flows to open a subtree merely by name, letting the registry select the concrete store implementation.
- Keep registration extensible so downstream applications can add custom types without patching core code.
- Preserve type safety for callers who know the expected type, while offering an ergonomic dynamic handle for callers that simply need “whatever store is registered here”.
- Integrate with `_index` so metadata written at commit time exactly matches what we know how to re-open later.

## Non-Goals

- Enforcing schema validation or migrations. The registry only wires type IDs to constructors; validation remains inside the store implementation.
- Dynamic loading of external code. Registrations happen in-process (likely at startup) rather than loading shared libraries at runtime.
- Removing the existing typed APIs. `get_store::<DocStore>` stays available for callers that prefer compile-time type selection.

## Proposed Architecture

### Components

1. **`StoreRegistry` (new module `store::registry`)**
   - Holds an `Arc<RwLock<HashMap<&'static str, StoreRegistration>>>`.
   - Provides `register(StoreRegistration)` and lookup methods.
   - Exposed via `StoreRegistry::global()` (a `OnceCell`-backed singleton) while also allowing custom registries to be passed into an `Instance` for embedded use cases.

2. **`StoreRegistration`**
   - Captures:
     - `type_id`: matches what `Store::type_id()` stores in `_index`.
     - `default_config`: optionally overrides the `Store`’s current `default_config()` to centralize configuration defaults.
     - `fn open_with_tx(&Transaction, &str, &SubtreeInfo) -> Result<BoxedStoreHandle>`.
     - `fn open_viewer(&Database, &str, &SubtreeInfo) -> Result<BoxedStoreHandle>`.
   - Builder helpers make registering a `Store` implementation ergonomic:
     ```rust
     StoreRegistration::for_store::<DocStore>()
         .with_viewer(|db, name, info| {
             let txn = db.new_transaction()?;
             DocStore::from_config(&txn, name.to_owned(), &info.config).map(BoxedStoreHandle::from)
         })
         .register();
     ```

3. **`BoxedStoreHandle`**
   - Thin wrapper around `Box<dyn Any + Send>` plus metadata (`type_id`, subtree name, config snapshot).
   - Implements helpers like `fn downcast<T: Store + 'static>(self) -> Result<T, Self>` and `fn type_id(&self) -> &str`.
   - Allows registry consumers (CLI, admin APIs) to inspect stores without knowing the concrete type, while preserving strong typing for those who opt to downcast.

4. **Instance/Database integration**
   - `Instance::new` initializes the registry with built-in store types (DocStore, Table, SettingsStore, `_index`, YDoc when the feature flag is enabled).
   - `Database::open_registered_store(name: &str) -> Result<BoxedStoreHandle>`:
     1. Reads `SubtreeInfo` from `_index` using the existing `IndexStore`.
     2. Looks up the registration by `info.type_id`.
     3. Invokes the registration’s viewer factory, returning a `BoxedStoreHandle`.
   - `Transaction::open_registered_store(name: &str) -> Result<BoxedStoreHandle>` follows the same flow but uses `open_with_tx`.
   - Existing typed APIs delegate to the registry for `_index` bookkeeping so there is only one place that knows about type IDs after refactor.

### Data Flow

1. **Registration (startup)**
   - During crate initialization (e.g., inside `store::mod`), we call:
     ```rust
     pub fn register_builtin_stores(registry: &StoreRegistry) {
         registry.register(StoreRegistration::for_store::<DocStore>());
         registry.register(StoreRegistration::for_store::<Table<serde_json::Value>>());
         // …
     }
     ```
   - Applications embedding Eidetica may call `StoreRegistry::global().register(...)` to add custom stores before opening databases.

2. **Commit path (already in place)**
   - When `Transaction::get_store::<T>` is called, the transaction records `(type_id, default_config)` for `_index` registration if the subtree is new.
   - After the refactor, `get_store::<T>` simply delegates to `StoreRegistry::lookup(T::type_id())` to fetch defaults, ensuring the registry is source of truth.

3. **Open path**
   - The caller asks `Database::open_registered_store("users")`.
   - The database fetches metadata from `_index`.
   - The registry resolves `type_id = "docstore:v1"` and invokes the doc store factory, which calls `DocStore::from_config`.
   - The caller receives a `BoxedStoreHandle`. If they know they expect DocStore they call `handle.downcast::<DocStore>()?`; a generic inspector can surface metadata without downcasting.

### API Surface

```rust
pub struct StoreRegistry { /* … */ }

impl StoreRegistry {
    pub fn global() -> &'static StoreRegistry;
    pub fn register(&self, registration: StoreRegistration);
    pub fn lookup(&self, type_id: &str) -> Option<StoreRegistration>;
    pub fn open_with_tx(&self, tx: &Transaction, name: &str, info: &SubtreeInfo)
        -> Result<BoxedStoreHandle>;
    pub fn open_viewer(&self, db: &Database, name: &str, info: &SubtreeInfo)
        -> Result<BoxedStoreHandle>;
}

pub struct StoreRegistration {
    pub type_id: &'static str;
    pub default_config: &'static str;
    pub tx_factory: fn(&Transaction, &str, &SubtreeInfo) -> Result<BoxedStoreHandle>;
    pub viewer_factory: fn(&Database, &str, &SubtreeInfo) -> Result<BoxedStoreHandle>;
}

pub struct BoxedStoreHandle {
    pub type_id: &'static str,
    pub subtree: String,
    inner: Box<dyn Any + Send>,
}

impl BoxedStoreHandle {
    pub fn downcast<T: Store + 'static>(self) -> Result<T, BoxedStoreHandle>;
    pub fn as_type_id(&self) -> &str;
}
```

### Implementation Plan

1. **Introduce the registry module**
   - Create `StoreRegistry`, `StoreRegistration`, and `BoxedStoreHandle`.
   - Add tests covering registration, lookup, downcasting, and thread safety.
2. **Register built-in store types**
   - Move DocStore/Table/YDoc/SettingsStore registration into a common helper invoked from `lib.rs`.
   - Update `Transaction::get_store` to consult the registry for default config/type IDs.
3. **Expose dynamic open APIs**
   - Add `Database::open_registered_store` and `Transaction::open_registered_store`.
   - Update CLI/internal tooling to enumerate subtrees via `IndexStore` and open them dynamically.
4. **Optional: typed helpers**
   - Provide `Database::get_store_by_name<T>()` wrappers that verify `_index` matches `T::type_id()` before calling `get_store::<T>`, giving better error messages.

Phases 1–3 can merge independently; once dynamic open APIs ship, existing code remains unaffected until it opts into the new API.

## Benefits

- **Single source of truth** for store metadata eliminates drift between `_index` contents and runtime constructors.
- **Extensibility**: Applications and plugins can register new stores (e.g., specialized CRDTs) without modifying Eidetica core.
- **Better tooling**: Admin consoles, debug CLIs, and sync daemons can discover and open all subtrees generically, unlocking schema introspection and migrations.
- **Safety**: Downcasting helpers ensure callers can assert the runtime type matches expectations, surfacing mismatches early.
- **Testability**: A deterministic registry makes it easy to set up integration tests that register mock stores or temporarily override constructors.

## Drawbacks / Risks

- **Type erasure overhead**: Wrapping stores in `Box<dyn Any + Send>` introduces a small allocation and downcasting step. This should be negligible compared to CRDT operations but is still extra work on the hot path.
- **Initialization order**: Plugins must register types before opening databases. Misordered registration could lead to “unknown type ID” errors until we add guardrails (e.g., requiring explicit `StoreRegistry` handles instead of global singletons).
- **Version skew**: Older databases might store type IDs for stores no longer registered in the binary. We need clear error messages and potentially migration tooling to handle this.
- **Generic stores like `Table<T>`**: Tables depend on the record type `T`. The registry proposal assumes either a canonical default like `serde_json::Value` or that callers using typed tables will continue to use the existing generic API. We should document this limitation and consider future schema metadata to enable richer table instantiation.
