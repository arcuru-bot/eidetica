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
//! Internally a snapshot is a sorted, deduplicated set of DAG tips. The
//! sorted+deduped invariant is enforced at every construction path; equality
//! and hashing are set-equal as a consequence.
//!
//! The user-facing name is `Snapshot`; `tips` is the structural noun used
//! inside the data-structure layer. Local variables and accessors still
//! talk about "tips" — `for tip in snapshot.tips()` reads naturally.

use serde::{Deserialize, Serialize};

use crate::{
    Result,
    backend::errors::BackendError,
    entry::ID,
    instance::backend::Backend,
};

/// Identifier for a database state — a sorted, deduplicated set of DAG tips.
///
/// Serialization is transparent (the wire form is a JSON/DAG-CBOR array of IDs,
/// identical to a `Vec<ID>`). Deserialization normalizes via `Snapshot::new`,
/// so unsorted or duplicated input is canonicalized on read.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Default)]
pub struct Snapshot(Vec<ID>);

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
    pub const EMPTY: Snapshot = Snapshot(Vec::new());

    /// Construct a snapshot from a vector of tips.
    ///
    /// The tips are sorted and deduplicated; the resulting snapshot satisfies
    /// the canonical invariant regardless of input order.
    pub fn new(mut tips: Vec<ID>) -> Self {
        tips.sort();
        tips.dedup();
        Self(tips)
    }

    /// Borrow the tips as a sorted, deduplicated slice.
    pub fn tips(&self) -> &[ID] {
        &self.0
    }

    /// Consume the snapshot and return the underlying tips.
    pub fn into_tips(self) -> Vec<ID> {
        self.0
    }

    /// Returns true if this snapshot contains no tips.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of tips in this snapshot.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns the database root that all tips in this snapshot belong to.
    ///
    /// Walks each tip's stored `tree.root` (recovering it from the entry itself
    /// for root entries) and asserts they all agree. Errors if the snapshot is
    /// empty or if its tips span multiple databases (a malformed snapshot).
    ///
    /// The check is intentionally always-verified — silently returning the
    /// first tip's root would hide a real bug class. If a profiler later finds
    /// a hot caller, an explicit fast-path variant can be added; until then,
    /// the verified semantics are the right default.
    pub async fn root(&self, backend: &Backend) -> Result<ID> {
        if self.0.is_empty() {
            return Err(BackendError::EmptyEntryList {
                operation: "Snapshot::root".to_string(),
            }
            .into());
        }

        let mut common: Option<ID> = None;
        for tip in &self.0 {
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
        &self.0
    }
}

impl<'a> IntoIterator for &'a Snapshot {
    type Item = &'a ID;
    type IntoIter = std::slice::Iter<'a, ID>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
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
    }
}
