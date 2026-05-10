//! Pure DAG extraction: turn a [`Database`] into a serializable [`Graph`].
//!
//! Everything in this module is independent of the HTTP layer and can be
//! exercised directly from tests.
//!
//! # Schema versioning
//!
//! The visualizer's wire format uses a `schema_version` field on the top-level
//! graph payload and a `kind` discriminator on both nodes and edges. This lets
//! future versions of Eidetica add new link types (for example, IPLD object
//! references or cross-database pointers) without breaking existing UI clients
//! that fetched a graph from an older server.
//!
//! Bump [`SCHEMA_VERSION`] whenever the existing variants change in a way that
//! is not strictly additive. Adding a new variant to [`Node`] or [`Edge`] is
//! additive and does not require a version bump.
//!
//! # Future extension points
//!
//! When new kinds of links become representable, extend the enums:
//!
//! - [`Node`] gains variants like `IpldObject { id, schema, … }` or
//!   `ExternalDatabase { id, … }`.
//! - [`Edge`] gains variants like `IpldLink { from, to, schema, path }` or
//!   `CrossDatabase { from, to_database, to_entry }`.
//!
//! No graph code outside this module needs to change; the frontend renders
//! unknown variants with a fallback style.

use std::collections::{BTreeSet, HashSet};

use eidetica::{Database, Entry, ID};
use serde::Serialize;

/// Wire-format version for the graph payload.
///
/// See module-level docs for when to bump this.
pub const SCHEMA_VERSION: u32 = 1;

/// A typed DAG node.
///
/// Tagged with `kind` so the JSON wire format is forward-compatible. Today
/// only [`Node::Entry`] is emitted; future kinds will be added here as new
/// link types become representable.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// A first-party Eidetica [`Entry`] in the database being visualized.
    Entry {
        /// Full entry ID (CID string).
        id: String,
        /// Truncated form for display, e.g. the first 8 chars of the CID.
        short_id: String,
        /// DAG height (longest path from the root).
        height: u64,
        /// True for the database root entry.
        is_root: bool,
        /// True if the entry is a current tip of the main tree.
        is_tip: bool,
        /// Names of subtrees this entry contributes data or parents to.
        subtrees: Vec<String>,
        /// Authentication signature info, opaque-but-displayable.
        sig: serde_json::Value,
    },
    // Future variants:
    //   IpldObject { id: String, short_id: String, schema: Option<String>, … }
    //   ExternalDatabase { id: String, name: Option<String>, … }
    //   ExternalEntry { id: String, database_id: String, … }
}

/// A typed DAG edge.
///
/// All edges point from a child to its parent in the DAG. Frontend renders
/// arrows accordingly. New variants may be added without breaking existing
/// clients (see module-level docs).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Edge {
    /// Child → parent edge in the main tree DAG.
    TreeParent {
        /// Child entry ID.
        from: String,
        /// Parent entry ID.
        to: String,
    },
    /// Child → parent edge inside a named subtree DAG.
    SubtreeParent {
        from: String,
        to: String,
        /// The subtree this edge belongs to.
        subtree: String,
    },
    // Future variants:
    //   IpldLink { from: String, to: String, schema: Option<String>, path: String }
    //   CrossDatabase { from: String, to_database: String }
    //   CrossEntry    { from: String, to_database: String, to_entry: String }
}

/// Serializable summary of a database, returned by the `database/:id` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseSummary {
    pub schema_version: u32,
    /// Database root ID.
    pub id: String,
    /// Truncated database root ID for display.
    pub short_id: String,
    /// Display name (if the settings store has one), otherwise `None`.
    pub name: Option<String>,
    /// Number of entries currently in the database.
    pub entry_count: usize,
    /// Number of current tree tips.
    pub tip_count: usize,
    /// Sorted list of distinct subtrees seen across all entries.
    pub subtrees: Vec<String>,
}

/// The full graph payload returned by the `graph` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct Graph {
    pub schema_version: u32,
    /// Database root ID this graph is for.
    pub database_id: String,
    /// Distinct subtree names seen in the graph, sorted.
    pub subtrees: Vec<String>,
    /// All nodes in the graph.
    pub nodes: Vec<Node>,
    /// All edges in the graph.
    pub edges: Vec<Edge>,
}

/// Detail payload for a single entry, returned by the `entries/:id` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct EntryDetail {
    pub schema_version: u32,
    pub id: String,
    pub short_id: String,
    pub root: Option<String>,
    pub is_root: bool,
    pub is_tip: bool,
    pub height: u64,
    pub tree_parents: Vec<String>,
    pub subtrees: Vec<SubtreeDetail>,
    pub sig: serde_json::Value,
    /// Size of the canonical DAG-CBOR encoding, in bytes.
    pub canonical_byte_size: usize,
}

/// Per-subtree detail embedded inside [`EntryDetail`].
#[derive(Debug, Clone, Serialize)]
pub struct SubtreeDetail {
    pub name: String,
    /// Subtree DAG height (None means inherits the tree's height).
    pub height: Option<u64>,
    pub parents: Vec<String>,
    /// The entry's payload bytes for this subtree, base64-encoded.
    /// `None` if the entry participates in the subtree but stores no payload.
    pub data_base64: Option<String>,
    /// Size of the payload in bytes (matches the decoded length of `data_base64`).
    pub data_byte_size: usize,
    /// Best-effort UTF-8 view of the data, or `None` if the bytes don't decode
    /// cleanly. Subtree payloads in current Eidetica stores are JSON strings,
    /// so this is usually populated.
    pub data_text: Option<String>,
}

/// Truncate a CID for display. Same convention used elsewhere in the dashboard.
fn short(id: &str) -> String {
    if id.len() <= 12 {
        id.to_string()
    } else {
        // CIDs are base32-lowercase; use a 12-char prefix.
        id[..12].to_string()
    }
}

/// Build the full DAG payload for `database`.
pub async fn build_graph(database: &Database) -> Result<Graph, eidetica::Error> {
    let entries = database.get_all_entries().await?;
    let tips: HashSet<ID> = database.get_tips().await?.into_iter().collect();

    let mut subtrees: BTreeSet<String> = BTreeSet::new();
    let mut nodes = Vec::with_capacity(entries.len());
    let mut edges = Vec::new();

    for entry in &entries {
        let entry_id = entry.id();
        let entry_id_str = entry_id.to_string();
        let entry_subtrees = entry.subtrees();

        for st in &entry_subtrees {
            subtrees.insert(st.clone());
        }

        let sig_value = serde_json::to_value(&entry.sig)
            .unwrap_or(serde_json::Value::String("<unserializable>".to_string()));

        nodes.push(Node::Entry {
            short_id: short(&entry_id_str),
            id: entry_id_str.clone(),
            height: entry.height(),
            is_root: entry.is_root(),
            is_tip: tips.contains(&entry_id),
            subtrees: entry_subtrees.clone(),
            sig: sig_value,
        });

        // Tree-DAG parents.
        if let Ok(parents) = entry.parents() {
            for parent in parents {
                edges.push(Edge::TreeParent {
                    from: entry_id_str.clone(),
                    to: parent.to_string(),
                });
            }
        }

        // Per-subtree parents.
        for st in &entry_subtrees {
            if let Ok(parents) = entry.subtree_parents(st) {
                for parent in parents {
                    edges.push(Edge::SubtreeParent {
                        from: entry_id_str.clone(),
                        to: parent.to_string(),
                        subtree: st.clone(),
                    });
                }
            }
        }
    }

    Ok(Graph {
        schema_version: SCHEMA_VERSION,
        database_id: database.root_id().to_string(),
        subtrees: subtrees.into_iter().collect(),
        nodes,
        edges,
    })
}

/// Build a per-database summary.
pub async fn build_summary(database: &Database) -> Result<DatabaseSummary, eidetica::Error> {
    let entries = database.get_all_entries().await?;
    let tips = database.get_tips().await?;
    let name = database.get_name().await.ok();

    let mut subtrees: BTreeSet<String> = BTreeSet::new();
    for entry in &entries {
        for st in entry.subtrees() {
            subtrees.insert(st);
        }
    }

    let id_str = database.root_id().to_string();
    Ok(DatabaseSummary {
        schema_version: SCHEMA_VERSION,
        short_id: short(&id_str),
        id: id_str,
        name,
        entry_count: entries.len(),
        tip_count: tips.len(),
        subtrees: subtrees.into_iter().collect(),
    })
}

/// Build the detail view for a single entry inside `database`.
pub async fn build_entry_detail(
    database: &Database,
    entry_id: &ID,
) -> Result<EntryDetail, eidetica::Error> {
    use base64ct::{Base64, Encoding};

    let entry: Entry = database.get_entry(entry_id.clone()).await?;
    let tips: HashSet<ID> = database.get_tips().await?.into_iter().collect();

    let id_str = entry_id.to_string();
    let tree_parents = entry
        .parents()
        .map(|ps| ps.into_iter().map(|p| p.to_string()).collect())
        .unwrap_or_default();

    let mut subtree_details = Vec::new();
    for name in entry.subtrees() {
        let parents = entry
            .subtree_parents(&name)
            .map(|ps| ps.into_iter().map(|p| p.to_string()).collect())
            .unwrap_or_default();
        let height = entry.subtree_height(&name).ok();
        let (data_base64, data_byte_size, data_text) = match entry.data(&name) {
            Ok(bytes) => {
                let size = bytes.len();
                let b64 = Base64::encode_string(bytes);
                let text = std::str::from_utf8(bytes).ok().map(|s| s.to_string());
                (Some(b64), size, text)
            }
            Err(_) => (None, 0, None),
        };

        subtree_details.push(SubtreeDetail {
            name,
            height,
            parents,
            data_base64,
            data_byte_size,
            data_text,
        });
    }

    let sig_value = serde_json::to_value(&entry.sig)
        .unwrap_or(serde_json::Value::String("<unserializable>".to_string()));

    let canonical_byte_size = entry.canonical_bytes().map(|b| b.len()).unwrap_or(0);

    Ok(EntryDetail {
        schema_version: SCHEMA_VERSION,
        short_id: short(&id_str),
        id: id_str,
        root: entry.root().map(|r| r.to_string()),
        is_root: entry.is_root(),
        is_tip: tips.contains(entry_id),
        height: entry.height(),
        tree_parents,
        subtrees: subtree_details,
        sig: sig_value,
        canonical_byte_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_truncates_long_strings() {
        assert_eq!(short("abcdefghijklmnop"), "abcdefghijkl");
    }

    #[test]
    fn short_passes_through_short_strings() {
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }

    #[test]
    fn tree_parent_edge_serializes_with_kind_tag() {
        let edge = Edge::TreeParent {
            from: "child".to_string(),
            to: "parent".to_string(),
        };
        let v = serde_json::to_value(&edge).unwrap();
        assert_eq!(v["kind"], "tree_parent");
        assert_eq!(v["from"], "child");
        assert_eq!(v["to"], "parent");
    }

    #[test]
    fn subtree_parent_edge_carries_subtree_name() {
        let edge = Edge::SubtreeParent {
            from: "child".to_string(),
            to: "parent".to_string(),
            subtree: "messages".to_string(),
        };
        let v = serde_json::to_value(&edge).unwrap();
        assert_eq!(v["kind"], "subtree_parent");
        assert_eq!(v["subtree"], "messages");
    }

    #[test]
    fn entry_node_serializes_all_display_fields() {
        let node = Node::Entry {
            id: "abcd".into(),
            short_id: "abcd".into(),
            height: 3,
            is_root: false,
            is_tip: true,
            subtrees: vec!["users".into()],
            sig: serde_json::json!({"key": {}}),
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["kind"], "entry");
        assert_eq!(v["height"], 3);
        assert_eq!(v["is_tip"], true);
        assert_eq!(v["subtrees"], serde_json::json!(["users"]));
    }

    #[test]
    fn graph_payload_includes_schema_version() {
        let g = Graph {
            schema_version: SCHEMA_VERSION,
            database_id: "x".into(),
            subtrees: vec![],
            nodes: vec![],
            edges: vec![],
        };
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["schema_version"], SCHEMA_VERSION);
    }
}

#[cfg(test)]
mod integration_tests {
    //! End-to-end tests that drive [`build_graph`] and [`build_entry_detail`]
    //! against an actual in-memory [`eidetica::Instance`]. These run as part
    //! of `cargo test -p eidetica-bin` and don't depend on any test fixtures.

    use super::*;
    use eidetica::{
        Instance,
        backend::database::InMemory,
        crdt::Doc,
        store::{DocStore, Table},
    };

    /// Spin up an InMemory instance, create a default user, build a database,
    /// and return both so the test can drive transactions against it.
    async fn fixture() -> (Instance, eidetica::Database) {
        let instance = Instance::open(Box::new(InMemory::new())).await.unwrap();
        instance.create_user("test", None).await.unwrap();
        let mut user = instance.login_user("test", None).await.unwrap();
        let key = user.get_default_key().unwrap();
        let mut settings = Doc::new();
        settings.set("name", "viz_test");
        let db = user.create_database(settings, &key).await.unwrap();
        (instance, db)
    }

    #[tokio::test]
    async fn graph_includes_root_and_tracks_tips() {
        let (_instance, db) = fixture().await;

        // Initial graph: just the root entry, which is also the tip.
        let g = build_graph(&db).await.unwrap();
        assert_eq!(g.schema_version, SCHEMA_VERSION);
        assert_eq!(g.nodes.len(), 1);
        let Node::Entry {
            is_root,
            is_tip,
            height,
            ..
        } = &g.nodes[0];
        assert!(*is_root);
        assert!(*is_tip);
        assert_eq!(*height, 0);
    }

    #[tokio::test]
    async fn graph_emits_tree_parent_edges_for_each_commit() {
        let (_instance, db) = fixture().await;

        // Commit two transactions on top of the root.
        for value in ["one", "two"] {
            let txn = db.new_transaction().await.unwrap();
            let store = txn.get_store::<DocStore>("notes").await.unwrap();
            store.set("k", value).await.unwrap();
            txn.commit().await.unwrap();
        }

        let g = build_graph(&db).await.unwrap();
        // Root + 2 commits.
        assert_eq!(g.nodes.len(), 3);

        let tree_edges: Vec<_> = g
            .edges
            .iter()
            .filter(|e| matches!(e, Edge::TreeParent { .. }))
            .collect();
        // Each non-root commit has exactly one tree parent.
        assert_eq!(tree_edges.len(), 2);

        // Exactly one node should currently be the tip.
        let tip_count = g
            .nodes
            .iter()
            .filter(|n| matches!(n, Node::Entry { is_tip: true, .. }))
            .count();
        assert_eq!(tip_count, 1);

        // The "notes" subtree should be discoverable.
        assert!(g.subtrees.iter().any(|s| s == "notes"));
    }

    #[tokio::test]
    async fn graph_emits_subtree_parent_edges_with_named_subtree() {
        let (_instance, db) = fixture().await;

        // Two consecutive commits to the same subtree create a subtree-parent
        // edge between the second commit and the first.
        for v in 0..2 {
            let txn = db.new_transaction().await.unwrap();
            let table = txn.get_store::<Table<String>>("rows").await.unwrap();
            table.insert(format!("v{v}")).await.unwrap();
            txn.commit().await.unwrap();
        }

        let g = build_graph(&db).await.unwrap();
        let subtree_edges: Vec<_> = g
            .edges
            .iter()
            .filter_map(|e| match e {
                Edge::SubtreeParent { subtree, .. } => Some(subtree.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            subtree_edges.contains(&"rows"),
            "expected at least one subtree_parent edge for 'rows'; got {subtree_edges:?}"
        );
    }

    #[tokio::test]
    async fn entry_detail_returns_subtree_payloads() {
        let (_instance, db) = fixture().await;

        let txn = db.new_transaction().await.unwrap();
        let store = txn.get_store::<DocStore>("notes").await.unwrap();
        store.set("greeting", "hello").await.unwrap();
        let entry_id = txn.commit().await.unwrap();

        let detail = build_entry_detail(&db, &entry_id).await.unwrap();
        assert_eq!(detail.id, entry_id.to_string());
        assert!(detail.is_tip);
        assert!(!detail.is_root);
        let notes = detail
            .subtrees
            .iter()
            .find(|s| s.name == "notes")
            .expect("notes subtree must be present");
        assert!(notes.data_byte_size > 0);
        assert!(
            notes.data_text.is_some(),
            "notes payload should decode as UTF-8"
        );
    }
}
