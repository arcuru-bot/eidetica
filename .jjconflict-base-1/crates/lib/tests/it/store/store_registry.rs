use crate::helpers::setup_tree;
use eidetica::store::{DocStore, Store};

fn register_data_subtree(tree: &eidetica::Database) {
    let op = tree.new_transaction().expect("failed to start transaction");
    let data_store = op
        .get_store::<DocStore>("data")
        .expect("failed to open DocStore");
    data_store
        .set("initial_key", "initial_value")
        .expect("failed to write data");
    op.commit().expect("commit failed");
}

#[test]
fn database_open_registered_store_returns_docstore() {
    let (_instance, tree) = setup_tree();
    register_data_subtree(&tree);

    let handle = tree
        .open_registered_store("data")
        .expect("failed to open registry store");
    assert_eq!(handle.type_id(), DocStore::type_id());

    let doc_store: DocStore = handle.downcast().expect("downcast to DocStore");
    assert_eq!(
        doc_store.get("initial_key").unwrap().as_text(),
        Some("initial_value")
    );
}

#[test]
fn transaction_open_registered_store_supports_writes() {
    let (_instance, tree) = setup_tree();
    register_data_subtree(&tree);

    let tx = tree.new_transaction().expect("failed to start transaction");
    let handle = tx
        .open_registered_store("data")
        .expect("failed to open registry store");
    let doc_store: DocStore = handle.downcast().expect("downcast DocStore");
    doc_store
        .set("dynamic_key", "dynamic_value")
        .expect("write failed");
    tx.commit().expect("commit failed");

    let viewer = tree
        .get_store_viewer::<DocStore>("data")
        .expect("open viewer");
    assert_eq!(
        viewer.get("dynamic_key").unwrap().as_text(),
        Some("dynamic_value")
    );
}
