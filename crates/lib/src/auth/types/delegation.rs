//! Delegation system types for authentication
//!
//! Defines references to delegated databases for the cross-database delegation system.
//! A delegation is a `Snapshot` of the delegated database's state plus the permission
//! bounds that clamp keys derived from that database.

use serde::{Deserialize, Serialize};

use super::permissions::PermissionBounds;
use crate::Snapshot;
use crate::crdt::{CRDTError, Doc, doc::Value};
use crate::entry::ID;

/// Delegated tree reference stored in main tree's _settings.auth.
///
/// The delegation is identified externally by the delegated database's root ID
/// (used as the storage key in `AuthSettings`); the root is therefore not
/// duplicated inside the reference itself. To recover the root from the snapshot
/// for verification, call `snapshot.root(backend).await`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DelegatedTreeRef {
    /// Permission bounds for keys derived from this delegated tree.
    #[serde(rename = "permission-bounds")]
    pub permission_bounds: PermissionBounds,
    /// Snapshot of the delegated tree at the point of delegation.
    pub snapshot: Snapshot,
}

// ==================== Doc Conversions ====================

impl From<DelegatedTreeRef> for Value {
    fn from(dtref: DelegatedTreeRef) -> Value {
        Value::Doc(Doc::from(dtref))
    }
}

impl From<DelegatedTreeRef> for Doc {
    fn from(dtref: DelegatedTreeRef) -> Doc {
        let mut doc = Doc::atomic();
        doc.set("permission_bounds", dtref.permission_bounds);
        let mut tips_doc = Doc::new();
        for (i, tip) in dtref.snapshot.tips().iter().enumerate() {
            tips_doc.set(i.to_string(), tip.to_string());
        }
        doc.set("snapshot", tips_doc);
        doc
    }
}

impl TryFrom<&Doc> for DelegatedTreeRef {
    type Error = crate::Error;

    fn try_from(doc: &Doc) -> crate::Result<Self> {
        let pb_doc = match doc.get("permission_bounds") {
            Some(Value::Doc(d)) => d,
            _ => {
                return Err(CRDTError::ElementNotFound {
                    key: "permission_bounds".to_string(),
                }
                .into());
            }
        };
        let permission_bounds = PermissionBounds::try_from(pb_doc)?;

        // New wire format keys the snapshot under "snapshot"; legacy format
        // nested it under "tree" with a denormalized "root" alongside "tips".
        // The legacy root is ignored — content-addressing recovers it.
        let snapshot = if let Some(Value::Doc(snap_doc)) = doc.get("snapshot") {
            tips_from_indexed_doc(snap_doc)?
        } else if let Some(Value::Doc(tree_doc)) = doc.get("tree") {
            match tree_doc.get("tips") {
                Some(Value::Doc(tips_doc)) => tips_from_indexed_doc(tips_doc)?,
                _ => Snapshot::EMPTY,
            }
        } else {
            return Err(CRDTError::ElementNotFound {
                key: "snapshot".to_string(),
            }
            .into());
        };

        Ok(DelegatedTreeRef {
            permission_bounds,
            snapshot,
        })
    }
}

/// Parse a `{"0": "tip0", "1": "tip1", ...}` doc into a Snapshot.
fn tips_from_indexed_doc(doc: &Doc) -> crate::Result<Snapshot> {
    let mut entries: Vec<(usize, &str)> = doc
        .iter()
        .filter_map(|(k, v)| {
            let idx: usize = k.parse().ok()?;
            let s = v.as_text()?;
            Some((idx, s))
        })
        .collect();
    entries.sort_by_key(|(idx, _)| *idx);
    let tips: Vec<ID> = entries
        .into_iter()
        .map(|(_, s)| ID::parse(s))
        .collect::<crate::Result<Vec<_>>>()?;
    Ok(Snapshot::new(tips))
}
