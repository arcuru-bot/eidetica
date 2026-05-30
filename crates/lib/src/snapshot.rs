//! Snapshot — an immutable identifier of a database state at a point in time.
//!
//! A `Snapshot` is the set of tip entry IDs that fully identifies the state
//! of a `Database`. Content-addressing makes the mapping bijective: given a
//! snapshot, the entries (and therefore all reachable content) are uniquely
//! determined.
//!
//! Use a `Snapshot` to pin a read view, anchor a transaction, or describe
//! a state transition (e.g. `WriteEvent { from, to }`).
//!
//! Internally a snapshot is a sorted, deduplicated set of DAG tips plus an
//! optional cached database root. The sorted+deduped invariant is enforced
//! at every construction path; equality and hashing depend only on the tips
//! (the root is a derived value).
//!
//! The user-facing name is `Snapshot`; `tips` is the structural noun used
//! inside the data-structure layer. Local variables and accessors still
//! talk about "tips" — `for tip in snapshot.tips()` reads naturally.

use serde::{Deserialize, Serialize, Serializer};

use crate::{Result, backend::errors::BackendError, entry::ID, instance::backend::Backend};

/// Identifier for a database state — a sorted, deduplicated set of DAG tips
/// with an optional cached database root.
///
/// Serialization is transparent (the wire form is a JSON/DAG-CBOR array of IDs,
/// identical to a `Vec<ID>`). The cached root is not part of the wire format:
/// callers that know the root populate it at construction time, callers that
/// only see wire data leave it `None` and resolve via [`Snapshot::root`] when
/// needed. Deserialization normalizes tips via `Snapshot::new`, so unsorted or
/// duplicated input is canonicalized on read.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Cached database root. Populated when the constructor knew it; absent
    /// after deserialize and after rootless construction. Not serialized.
    root: Option<ID>,
    /// Sorted, deduplicated set of tip IDs.
    tips: Vec<ID>,
}

impl Serialize for Snapshot {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.tips.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Snapshot {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let tips = Vec::<ID>::deserialize(deserializer)?;
        Ok(Self::new(tips))
    }
}

impl Snapshot {
    /// A snapshot containing no tips — the state of a database with no entries.
    pub const EMPTY: Snapshot = Snapshot {
        root: None,
        tips: Vec::new(),
    };

    /// Construct a snapshot from a vector of tips, with no cached root.
    ///
    /// Tips are sorted and deduplicated. Prefer [`Snapshot::for_database`]
    /// when the database root is known at construction time — it lets callers
    /// of [`Snapshot::root`] avoid a backend round-trip.
    pub fn new(mut tips: Vec<ID>) -> Self {
        tips.sort();
        tips.dedup();
        Self { root: None, tips }
    }

    /// Construct a snapshot bound to a known database root.
    ///
    /// The root is cached; [`Snapshot::root`] returns it without I/O.
    /// Tips are sorted and deduplicated.
    pub fn for_database(root: ID, mut tips: Vec<ID>) -> Self {
        tips.sort();
        tips.dedup();
        Self {
            root: Some(root),
            tips,
        }
    }

    /// Borrow the tips as a sorted, deduplicated slice.
    pub fn tips(&self) -> &[ID] {
        &self.tips
    }

    /// Consume the snapshot and return the underlying tips.
    pub fn into_tips(self) -> Vec<ID> {
        self.tips
    }

    /// Returns true if this snapshot contains no tips.
    pub fn is_empty(&self) -> bool {
        self.tips.is_empty()
    }

    /// Number of tips in this snapshot.
    pub fn len(&self) -> usize {
        self.tips.len()
    }

    /// Cached database root, if the snapshot was constructed with one.
    ///
    /// Returns `None` for snapshots built via [`Snapshot::new`] or restored
    /// from wire data. Use [`Snapshot::root`] to resolve unconditionally.
    pub fn known_root(&self) -> Option<&ID> {
        self.root.as_ref()
    }

    /// Borrow the cached root or error.
    ///
    /// Used by APIs that require the snapshot to carry its root (backend
    /// query methods). Callers holding a wire-restored snapshot must
    /// resolve the root via [`Snapshot::root`] and rebuild with
    /// [`Snapshot::for_database`] before passing it in.
    pub fn require_root(&self) -> Result<&ID> {
        self.root.as_ref().ok_or_else(|| {
            BackendError::TreeIntegrityViolation {
                reason: "Snapshot is missing its cached database root; \
                         construct with Snapshot::for_database or resolve via snapshot.root(backend)"
                    .to_string(),
            }
            .into()
        })
    }

    /// Returns the database root that all tips in this snapshot belong to.
    ///
    /// Returns the cached root when present. Otherwise walks each tip's
    /// stored `tree.root` (one backend read per tip) and asserts they all
    /// agree. Errors if the snapshot is empty or if its tips span multiple
    /// databases (a malformed snapshot).
    ///
    /// The walk is intentionally always-verified — silently returning the
    /// first tip's root would hide a real bug class. Hot callers should
    /// construct snapshots with [`Snapshot::for_database`] so the cached
    /// root is returned immediately without I/O.
    pub async fn root(&self, backend: &Backend) -> Result<ID> {
        if let Some(root) = &self.root {
            return Ok(root.clone());
        }

        if self.tips.is_empty() {
            return Err(BackendError::EmptyEntryList {
                operation: "Snapshot::root".to_string(),
            }
            .into());
        }

        let mut common: Option<ID> = None;
        for tip in &self.tips {
            let entry = backend.get(tip).await?;
            // For root entries Entry::root() returns None — the root is the entry itself.
            let root = entry.root().unwrap_or_else(|| entry.id());
            match &common {
                None => common = Some(root),
                Some(prev) if prev != &root => {
                    return Err(BackendError::TreeIntegrityViolation {
                        reason: format!(
                            "Snapshot tips span multiple databases: tip {tip} belongs to {root}, expected {prev}"
                        ),
                    }
                    .into());
                }
                _ => {}
            }
        }

        Ok(common.expect("snapshot is non-empty"))
    }
}

/// Equality compares tips only — the cached root is derived data, not identity.
impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.tips == other.tips
    }
}

impl Eq for Snapshot {}

/// Hash is over tips only, matching `PartialEq`.
impl std::hash::Hash for Snapshot {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tips.hash(state);
    }
}

impl From<Vec<ID>> for Snapshot {
    fn from(tips: Vec<ID>) -> Self {
        Self::new(tips)
    }
}

impl From<&[ID]> for Snapshot {
    fn from(tips: &[ID]) -> Self {
        Self::new(tips.to_vec())
    }
}

impl<const N: usize> From<[ID; N]> for Snapshot {
    fn from(tips: [ID; N]) -> Self {
        Self::new(tips.to_vec())
    }
}

impl AsRef<[ID]> for Snapshot {
    fn as_ref(&self) -> &[ID] {
        &self.tips
    }
}

impl<'a> IntoIterator for &'a Snapshot {
    type Item = &'a ID;
    type IntoIter = std::slice::Iter<'a, ID>;

    fn into_iter(self) -> Self::IntoIter {
        self.tips.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> ID {
        ID::from_bytes([byte])
    }

    #[test]
    fn empty_const_is_empty() {
        assert!(Snapshot::EMPTY.is_empty());
        assert_eq!(Snapshot::EMPTY.len(), 0);
        assert_eq!(Snapshot::EMPTY.tips(), &[] as &[ID]);
        assert_eq!(Snapshot::EMPTY.known_root(), None);
    }

    #[test]
    fn default_equals_empty() {
        assert_eq!(Snapshot::default(), Snapshot::EMPTY);
    }

    #[test]
    fn new_sorts_input() {
        let a = id(1);
        let b = id(2);
        let c = id(3);
        let unsorted = Snapshot::new(vec![c.clone(), a.clone(), b.clone()]);
        let sorted = Snapshot::new(vec![a, b, c]);
        assert_eq!(unsorted.tips(), sorted.tips());
    }

    #[test]
    fn new_dedups_input() {
        let a = id(1);
        let b = id(2);
        let with_dupes = Snapshot::new(vec![a.clone(), b.clone(), a.clone(), b.clone(), a.clone()]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(with_dupes.len(), 2);
        assert_eq!(with_dupes.tips(), expected.as_slice());
    }

    #[test]
    fn new_leaves_root_unset() {
        let snap = Snapshot::new(vec![id(1)]);
        assert_eq!(snap.known_root(), None);
    }

    #[test]
    fn for_database_caches_root_and_normalizes_tips() {
        let root = id(99);
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::for_database(root.clone(), vec![b.clone(), a.clone(), a.clone()]);
        assert_eq!(snap.known_root(), Some(&root));
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(snap.tips(), expected.as_slice());
    }

    #[test]
    fn equality_ignores_root() {
        let a = id(1);
        let b = id(2);
        let rootless = Snapshot::new(vec![a.clone(), b.clone()]);
        let with_root = Snapshot::for_database(id(99), vec![a, b]);
        assert_eq!(rootless, with_root);
    }

    #[test]
    fn hash_ignores_root() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let a = id(1);
        let b = id(2);
        let rootless = Snapshot::new(vec![a.clone(), b.clone()]);
        let with_root = Snapshot::for_database(id(99), vec![a, b]);
        let mut h1 = DefaultHasher::new();
        rootless.hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        with_root.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn set_equality_holds_via_canonical_form() {
        let a = id(1);
        let b = id(2);
        let s1: Snapshot = vec![a.clone(), b.clone()].into();
        let s2: Snapshot = vec![b, a].into();
        assert_eq!(s1, s2);
    }

    #[test]
    fn hash_matches_for_set_equal_snapshots() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let a = id(1);
        let b = id(2);
        let s1: Snapshot = vec![a.clone(), b.clone()].into();
        let s2: Snapshot = vec![b, a].into();
        let mut h1 = DefaultHasher::new();
        s1.hash(&mut h1);
        let mut h2 = DefaultHasher::new();
        s2.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn from_slice_sorts_and_dedups() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::from(&[b.clone(), a.clone(), a.clone()][..]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(snap.tips(), expected.as_slice());
    }

    #[test]
    fn from_array_sorts_and_dedups() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::from([b.clone(), a.clone(), a.clone()]);
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(snap.tips(), expected.as_slice());
    }

    #[test]
    fn into_tips_returns_sorted_vec() {
        let mut expected = vec![id(1), id(2), id(3)];
        expected.sort();
        let snap = Snapshot::new(vec![id(3), id(1), id(2)]);
        assert_eq!(snap.into_tips(), expected);
    }

    #[test]
    fn iter_yields_sorted_tips() {
        let mut expected = [id(1), id(2), id(3)];
        expected.sort();
        let snap = Snapshot::new(vec![id(3), id(1), id(2)]);
        let collected: Vec<&ID> = (&snap).into_iter().collect();
        let expected_refs: Vec<&ID> = expected.iter().collect();
        assert_eq!(collected, expected_refs);
    }

    #[test]
    fn as_ref_exposes_sorted_slice() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::new(vec![b.clone(), a.clone()]);
        let slice: &[ID] = snap.as_ref();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(slice, expected.as_slice());
    }

    #[test]
    fn serde_roundtrip_preserves_invariant() {
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::new(vec![b, a]);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, snap);
    }

    #[test]
    fn serde_does_not_persist_cached_root() {
        let snap = Snapshot::for_database(id(99), vec![id(1), id(2)]);
        let json = serde_json::to_string(&snap).unwrap();
        let parsed: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.known_root(), None);
        assert_eq!(parsed.tips(), snap.tips());
    }

    #[test]
    fn deserialize_normalizes_unsorted_wire_data() {
        // Simulate wire data written without the sorted invariant
        // (e.g. by an older client). The Snapshot deserializer must canonicalize.
        let a = id(1);
        let b = id(2);
        let canonical = Snapshot::new(vec![a.clone(), b.clone()]);
        let unsorted_json = serde_json::to_string(&vec![b, a]).unwrap();
        let parsed: Snapshot = serde_json::from_str(&unsorted_json).unwrap();
        assert_eq!(parsed, canonical);
    }

    #[test]
    fn deserialize_dedups_wire_data() {
        let a = id(1);
        let b = id(2);
        let canonical = Snapshot::new(vec![a.clone(), b.clone()]);
        let duped_json = serde_json::to_string(&vec![a.clone(), b, a]).unwrap();
        let parsed: Snapshot = serde_json::from_str(&duped_json).unwrap();
        assert_eq!(parsed, canonical);
    }

    #[test]
    fn serializes_as_bare_id_array() {
        // Snapshot must be wire-compatible with Vec<ID> — same JSON shape.
        // This matters for in-place migration of fields that were previously
        // typed `Vec<ID>` (e.g. EntryMetadata.settings_tips).
        let a = id(1);
        let b = id(2);
        let snap = Snapshot::new(vec![a.clone(), b.clone()]);
        let snap_json = serde_json::to_string(&snap).unwrap();
        let vec_json = serde_json::to_string(snap.tips()).unwrap();
        assert_eq!(snap_json, vec_json);
    }

    mod root {
        use std::sync::Arc;

        use super::*;
        use crate::backend::database::InMemory;
        use crate::entry::Entry;
        use crate::instance::backend::Backend;

        fn test_backend() -> Backend {
            Backend::new(Arc::new(InMemory::new()))
        }

        async fn put_root(backend: &Backend) -> ID {
            let entry = Entry::root_builder()
                .set_subtree_data("data", "root-data")
                .build()
                .expect("root entry should build");
            let id = entry.id();
            backend.put_verified(entry).await.unwrap();
            id
        }

        async fn put_child(backend: &Backend, root: &ID, parent: &ID, label: &str) -> ID {
            let entry = Entry::builder(root.clone())
                .add_parent(parent.clone())
                .set_subtree_data("data", label)
                .build()
                .expect("child entry should build");
            let id = entry.id();
            backend.put_verified(entry).await.unwrap();
            id
        }

        #[tokio::test]
        async fn empty_snapshot_errors() {
            let backend = test_backend();
            let err = Snapshot::EMPTY.root(&backend).await.unwrap_err();
            assert!(format!("{err}").contains("Snapshot::root"));
        }

        #[tokio::test]
        async fn single_tip_returns_database_root() {
            let backend = test_backend();
            let root = put_root(&backend).await;
            let child = put_child(&backend, &root, &root, "child").await;

            let snap = Snapshot::from([child]);
            assert_eq!(snap.root(&backend).await.unwrap(), root);
        }

        #[tokio::test]
        async fn root_entry_resolves_to_itself() {
            let backend = test_backend();
            let root = put_root(&backend).await;

            let snap = Snapshot::from([root.clone()]);
            assert_eq!(snap.root(&backend).await.unwrap(), root);
        }

        #[tokio::test]
        async fn agreeing_tips_return_common_root() {
            let backend = test_backend();
            let root = put_root(&backend).await;
            let left = put_child(&backend, &root, &root, "left").await;
            let right = put_child(&backend, &root, &root, "right").await;

            let snap = Snapshot::from([left, right]);
            assert_eq!(snap.root(&backend).await.unwrap(), root);
        }

        #[tokio::test]
        async fn disagreeing_tips_error() {
            let backend = test_backend();
            let root_a = put_root(&backend).await;
            let child_a = put_child(&backend, &root_a, &root_a, "a").await;

            // Distinct database (different root entry, different data).
            let root_b = {
                let entry = Entry::root_builder()
                    .set_subtree_data("data", "different-root-data")
                    .build()
                    .expect("root entry should build");
                let id = entry.id();
                backend.put_verified(entry).await.unwrap();
                id
            };
            let child_b = put_child(&backend, &root_b, &root_b, "b").await;

            let snap = Snapshot::from([child_a, child_b]);
            let err = snap.root(&backend).await.unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("span multiple databases"),
                "expected multi-database error, got: {msg}"
            );
        }

        #[tokio::test]
        async fn cached_root_returned_without_backend_io() {
            // Use a snapshot built with for_database whose tips don't exist
            // in the backend. If the cached path is taken, no read is attempted.
            let backend = test_backend();
            let fabricated_root = id(42);
            let fabricated_tip = id(43);
            let snap = Snapshot::for_database(fabricated_root.clone(), vec![fabricated_tip]);
            assert_eq!(snap.root(&backend).await.unwrap(), fabricated_root);
        }
    }
}
