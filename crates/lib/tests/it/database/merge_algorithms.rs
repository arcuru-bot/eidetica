//! Parent-aware merge algorithm tests
//!
//! This module contains tests for complex merging scenarios including
//! LCA computation, diamond patterns, and parent-aware state resolution.

use eidetica::{Snapshot, crdt::doc::Value, store::DocStore};

use super::helpers::*;
use crate::helpers::*;

#[tokio::test]
async fn test_simple_linear_chain() {
    // Test basic parent-aware merging: A -> B -> C
    let (_instance, tree) = setup_tree().await;

    // Create entry A with initial data
    let op_a = tree.new_transaction().await.unwrap();
    let subtree_a = op_a.get_store::<DocStore>("data").await.unwrap();
    subtree_a.set("counter", "1").await.unwrap();
    subtree_a.set("name", "alice").await.unwrap();
    op_a.commit().await.unwrap();

    // Create entry B as child of A
    let op_b = tree.new_transaction().await.unwrap();
    let subtree_b = op_b.get_store::<DocStore>("data").await.unwrap();
    subtree_b.set("counter", "2").await.unwrap(); // Update counter
    subtree_b.set("age", "25").await.unwrap(); // Add new field
    op_b.commit().await.unwrap();

    // Create entry C as child of B
    let op_c = tree.new_transaction().await.unwrap();
    let subtree_c = op_c.get_store::<DocStore>("data").await.unwrap();
    subtree_c.set("counter", "3").await.unwrap(); // Update counter again
    subtree_c.set("city", "nyc").await.unwrap(); // Add another field
    op_c.commit().await.unwrap();

    // Check the final accumulated state
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Final state should have all fields from the chain:
    // - counter: "3" (latest value from C)
    // - name: "alice" (from A, never overridden)
    // - age: "25" (from B, never overridden)
    // - city: "nyc" (from C)

    match final_state.get("counter").unwrap() {
        Value::Text(v) => assert_eq!(v, "3"),
        _ => panic!("Expected string for counter"),
    }

    match final_state.get("name").unwrap() {
        Value::Text(v) => assert_eq!(v, "alice"),
        _ => panic!("Expected string for name"),
    }

    match final_state.get("age").unwrap() {
        Value::Text(v) => assert_eq!(v, "25"),
        _ => panic!("Expected string for age"),
    }

    match final_state.get("city").unwrap() {
        Value::Text(v) => assert_eq!(v, "nyc"),
        _ => panic!("Expected string for city"),
    }
}

#[tokio::test]
async fn test_criss_cross_merge_converges_from_empty_base() {
    let (_instance, tree) = setup_tree().await;
    let genesis = tree
        .snapshot()
        .await
        .expect("Failed to get genesis snapshot")
        .into_tips()
        .into_iter()
        .next()
        .expect("Tree should have a genesis entry");

    let root1 = tree
        .new_transaction_at(&Snapshot::from([genesis.clone()]))
        .await
        .expect("Failed to create root1 transaction");
    root1
        .get_store::<DocStore>("data")
        .await
        .expect("Failed to get root1 store")
        .set("root1", "value")
        .await
        .expect("Failed to write root1");
    let mut tip1 = root1.commit().await.expect("Failed to commit root1");

    let root2 = tree
        .new_transaction_at(&Snapshot::from([genesis]))
        .await
        .expect("Failed to create root2 transaction");
    root2
        .get_store::<DocStore>("data")
        .await
        .expect("Failed to get root2 store")
        .set("root2", "value")
        .await
        .expect("Failed to write root2");
    let mut tip2 = root2.commit().await.expect("Failed to commit root2");

    for index in 0..2 {
        let txn = tree
            .new_transaction_at(&Snapshot::from([tip1]))
            .await
            .expect("Failed to extend first chain");
        txn.get_store::<DocStore>("data")
            .await
            .expect("Failed to get first chain store")
            .set(format!("c1_{index}"), "value")
            .await
            .expect("Failed to write first chain");
        tip1 = txn.commit().await.expect("Failed to commit first chain");
    }

    for index in 0..2 {
        let txn = tree
            .new_transaction_at(&Snapshot::from([tip2]))
            .await
            .expect("Failed to extend second chain");
        txn.get_store::<DocStore>("data")
            .await
            .expect("Failed to get second chain store")
            .set(format!("c2_{index}"), "value")
            .await
            .expect("Failed to write second chain");
        tip2 = txn.commit().await.expect("Failed to commit second chain");
    }

    let merge1 = tree
        .new_transaction_at(&Snapshot::from([tip1.clone(), tip2.clone()]))
        .await
        .expect("Failed to create first merge");
    merge1
        .get_store::<DocStore>("data")
        .await
        .expect("Failed to get first merge store")
        .set("merge1", "value")
        .await
        .expect("Failed to write first merge");
    merge1.commit().await.expect("Failed to commit first merge");

    let merge2 = tree
        .new_transaction_at(&Snapshot::from([tip1, tip2]))
        .await
        .expect("Failed to create second merge");
    merge2
        .get_store::<DocStore>("data")
        .await
        .expect("Failed to get second merge store")
        .set("merge2", "value")
        .await
        .expect("Failed to write second merge");
    merge2
        .commit()
        .await
        .expect("Failed to commit second merge");

    let viewer = tree
        .get_store_viewer::<DocStore>("data")
        .await
        .expect("Failed to get store viewer");
    let state = viewer
        .get_all()
        .await
        .expect("Criss-cross merge should materialize from the empty base");
    for key in ["root1", "root2", "merge1", "merge2"] {
        assert_eq!(state.get(key), Some(&Value::Text("value".to_string())));
    }
    assert_deterministic_reads(&tree, "data", 3).await;
}

#[tokio::test]
async fn test_caching_consistency() {
    // Test that caching provides consistent results
    let (_instance, tree) = setup_tree().await;

    // Create a simple chain to have some data to cache
    let op_a = tree.new_transaction().await.unwrap();
    let subtree_a = op_a.get_store::<DocStore>("data").await.unwrap();
    subtree_a.set("value", "1").await.unwrap();
    op_a.commit().await.unwrap();

    let op_b = tree.new_transaction().await.unwrap();
    let subtree_b = op_b.get_store::<DocStore>("data").await.unwrap();
    subtree_b.set("value", "2").await.unwrap();
    op_b.commit().await.unwrap();

    let op_c = tree.new_transaction().await.unwrap();
    let subtree_c = op_c.get_store::<DocStore>("data").await.unwrap();
    subtree_c.set("value", "3").await.unwrap();
    op_c.commit().await.unwrap();

    // First read - should compute and cache states
    let viewer1 = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let state1 = viewer1.get_all().await.unwrap();

    // Second read - should use cached states
    let viewer2 = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let state2 = viewer2.get_all().await.unwrap();

    // Third read - should also use cached states
    let viewer3 = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let state3 = viewer3.get_all().await.unwrap();

    // All results should be identical
    assert_eq!(state1, state2);
    assert_eq!(state2, state3);

    // Check the final value
    match state1.get("value").unwrap() {
        Value::Text(v) => assert_eq!(v, "3"),
        _ => panic!("Expected string for value"),
    }
}

#[tokio::test]
async fn test_parent_merge_semantics() {
    // Test that parent states are properly merged
    let (_instance, tree) = setup_tree().await;

    // Create base entry with shared data
    let txn_base = tree.new_transaction().await.unwrap();
    let subtree_base = txn_base.get_store::<DocStore>("data").await.unwrap();
    subtree_base.set("base_field", "base_value").await.unwrap();
    subtree_base.set("shared_field", "original").await.unwrap();
    txn_base.commit().await.unwrap();

    // Create child entry that updates shared field and adds new field
    let op_child = tree.new_transaction().await.unwrap();
    let subtree_child = op_child.get_store::<DocStore>("data").await.unwrap();
    subtree_child.set("shared_field", "updated").await.unwrap();
    subtree_child
        .set("child_field", "child_value")
        .await
        .unwrap();
    op_child.commit().await.unwrap();

    // Check the merged state
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Should have both base and child data, with child overriding shared field
    match final_state.get("base_field").unwrap() {
        Value::Text(v) => assert_eq!(v, "base_value"),
        _ => panic!("Expected string for base_field"),
    }

    match final_state.get("child_field").unwrap() {
        Value::Text(v) => assert_eq!(v, "child_value"),
        _ => panic!("Expected string for child_field"),
    }

    match final_state.get("shared_field").unwrap() {
        Value::Text(v) => assert_eq!(v, "updated"),
        _ => panic!("Expected string for shared_field"),
    }
}

#[tokio::test]
async fn test_deep_chain_performance() {
    // Test that deep chains don't cause stack overflow and use caching effectively
    let (_instance, tree) = setup_tree().await;

    // Create a moderately deep chain (not too deep to avoid long test times)
    const CHAIN_LENGTH: u32 = 50;

    for i in 1..=CHAIN_LENGTH {
        let txn = tree.new_transaction().await.unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("step", i.to_string()).await.unwrap();
        subtree
            .set(format!("step_{i}"), format!("value_{i}"))
            .await
            .unwrap();
        txn.commit().await.unwrap();
    }

    // Read the final state - this should not stack overflow
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Check that we have the final step
    match final_state.get("step").unwrap() {
        Value::Text(v) => assert_eq!(v, &CHAIN_LENGTH.to_string()),
        _ => panic!("Expected string for step"),
    }

    // Check that we have all intermediate steps
    for i in 1..=CHAIN_LENGTH {
        let key = format!("step_{i}");
        let expected = format!("value_{i}");
        match final_state.get(&key).unwrap() {
            Value::Text(v) => assert_eq!(v, &expected),
            _ => panic!("Expected string for {key}"),
        }
    }
}

#[tokio::test]
async fn test_multiple_reads_consistency() {
    // Test that multiple reads of the same data are consistent (deterministic)
    let (_instance, tree) = setup_tree().await;

    // Create some test data
    let txn1 = tree.new_transaction().await.unwrap();
    let subtree1 = txn1.get_store::<DocStore>("data").await.unwrap();
    subtree1.set("key1", "value1").await.unwrap();
    subtree1.set("key2", "value2").await.unwrap();
    txn1.commit().await.unwrap();

    let txn2 = tree.new_transaction().await.unwrap();
    let subtree2 = txn2.get_store::<DocStore>("data").await.unwrap();
    subtree2.set("key1", "updated1").await.unwrap();
    subtree2.set("key3", "value3").await.unwrap();
    txn2.commit().await.unwrap();

    // Read the data multiple times
    let mut results = Vec::new();
    for _ in 0..5 {
        let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
        let state = viewer.get_all().await.unwrap();
        results.push(state);
    }

    // All results should be identical
    for i in 1..results.len() {
        assert_eq!(results[0], results[i], "Read {i} differs from read 0");
    }

    // Check that the expected final state is correct
    let final_state = &results[0];
    match final_state.get("key1").unwrap() {
        Value::Text(v) => assert_eq!(v, "updated1"),
        _ => panic!("Expected string for key1"),
    }

    match final_state.get("key2").unwrap() {
        Value::Text(v) => assert_eq!(v, "value2"),
        _ => panic!("Expected string for key2"),
    }

    match final_state.get("key3").unwrap() {
        Value::Text(v) => assert_eq!(v, "value3"),
        _ => panic!("Expected string for key3"),
    }
}

#[tokio::test]
async fn test_incorrect_parent_merging_would_fail() {
    // This test demonstrates a critical issue that would occur with the incorrect approach
    // of merging parent states directly. It tests a scenario where a complex branching
    // pattern requires proper LCA-based computation to get the correct result.
    //
    // The test creates overlapping field updates across multiple operations, where
    // the incorrect approach would compute parent states with inconsistent orderings
    // and potentially lose or incorrectly merge data.

    let (_instance, tree) = setup_tree().await;

    // Create a sequence of operations that build up a complex state
    // Step 1: Initial state with multiple fields
    let txn1 = tree.new_transaction().await.unwrap();
    let subtree1 = txn1.get_store::<DocStore>("data").await.unwrap();
    subtree1.set("count", "1").await.unwrap();
    subtree1.set("name", "initial").await.unwrap();
    subtree1.set("status", "active").await.unwrap();
    txn1.commit().await.unwrap();

    // Step 2: Update some fields, add new ones
    let txn2 = tree.new_transaction().await.unwrap();
    let subtree2 = txn2.get_store::<DocStore>("data").await.unwrap();
    subtree2.set("count", "2").await.unwrap(); // Update existing
    subtree2.set("category", "type_a").await.unwrap(); // Add new
    txn2.commit().await.unwrap();

    // Step 3: More updates with overlapping and new fields
    let txn3 = tree.new_transaction().await.unwrap();
    let subtree3 = txn3.get_store::<DocStore>("data").await.unwrap();
    subtree3.set("count", "3").await.unwrap(); // Update again
    subtree3.set("name", "updated").await.unwrap(); // Update existing
    subtree3.set("priority", "high").await.unwrap(); // Add new
    txn3.commit().await.unwrap();

    // Step 4: Final operation with more field changes
    let txn4 = tree.new_transaction().await.unwrap();
    let subtree4 = txn4.get_store::<DocStore>("data").await.unwrap();
    subtree4.set("count", "4").await.unwrap(); // Final count update
    subtree4.set("status", "completed").await.unwrap(); // Update status
    subtree4.set("result", "success").await.unwrap(); // Add final field
    txn4.commit().await.unwrap();

    // Clear cache to force computation
    tree.backend()
        .expect("Failed to get backend")
        .clear_crdt_cache()
        .await
        .unwrap();

    // Read the final state - this exercises the complex merge algorithm
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    println!("Final state after complex operations: {final_state:#?}");

    // With the CORRECT LCA-based algorithm, we should get the accumulated state:
    // - All fields from all operations should be present
    // - Latest values should win for updated fields
    // - No data should be lost

    // Verify all expected fields are present
    assert!(final_state.get("count").is_some(), "count field missing");
    assert!(final_state.get("name").is_some(), "name field missing");
    assert!(final_state.get("status").is_some(), "status field missing");
    assert!(
        final_state.get("category").is_some(),
        "category field missing"
    );
    assert!(
        final_state.get("priority").is_some(),
        "priority field missing"
    );
    assert!(final_state.get("result").is_some(), "result field missing");

    // Verify final values are correct (latest values should win)
    match final_state.get("count").unwrap() {
        Value::Text(v) => assert_eq!(v, "4", "count should be final value"),
        _ => panic!("Expected string for count"),
    }

    match final_state.get("name").unwrap() {
        Value::Text(v) => assert_eq!(v, "updated", "name should be updated value"),
        _ => panic!("Expected string for name"),
    }

    match final_state.get("status").unwrap() {
        Value::Text(v) => assert_eq!(v, "completed", "status should be final value"),
        _ => panic!("Expected string for status"),
    }

    match final_state.get("category").unwrap() {
        Value::Text(v) => assert_eq!(v, "type_a", "category should be preserved"),
        _ => panic!("Expected string for category"),
    }

    match final_state.get("priority").unwrap() {
        Value::Text(v) => assert_eq!(v, "high", "priority should be preserved"),
        _ => panic!("Expected string for priority"),
    }

    match final_state.get("result").unwrap() {
        Value::Text(v) => assert_eq!(v, "success", "result should be preserved"),
        _ => panic!("Expected string for result"),
    }

    // The key insight: with the INCORRECT parent-state merging approach:
    // 1. Each parent state would be computed independently with different ancestry
    // 2. When merging parent states, some fields might be lost or incorrectly resolved
    // 3. The order of merging could affect the final result
    // 4. Data integrity would be compromised in complex scenarios
    //
    // With the CORRECT LCA-based approach:
    // 1. All computations start from a shared LCA (common ancestor)
    // 2. Paths from LCA to each tip are applied deterministically
    // 3. All data is preserved and merged consistently
    // 4. Results are deterministic regardless of access patterns

    // Verify deterministic behavior by reading multiple times
    for i in 0..5 {
        let viewer_check = tree.get_store_viewer::<DocStore>("data").await.unwrap();
        let state_check = viewer_check.get_all().await.unwrap();
        assert_eq!(
            final_state, state_check,
            "State should be deterministic on read {i}"
        );
    }

    println!("✓ Complex merge test passed - LCA algorithm preserves all data correctly");
    println!("  This test would likely FAIL with incorrect parent-state merging approach");
}

#[tokio::test]
async fn test_true_diamond_pattern() {
    // This test creates a TRUE diamond pattern that would definitely fail with incorrect
    // parent-state merging. We use the new Tree interface to manually control which tips
    // each operation starts from.
    //
    // Diamond pattern:
    //      A (shared ancestor)
    //     / \
    //    B   C (parallel operations from A)
    //     \ /
    //      D (merge operation sees both B and C as tips)

    let (_instance, tree) = setup_tree().await;

    // Step 1: Create entry A (common ancestor)
    let op_a = tree.new_transaction().await.unwrap();
    let subtree_a = op_a.get_store::<DocStore>("data").await.unwrap();
    subtree_a.set("base", "A").await.unwrap();
    subtree_a.set("shared", "original").await.unwrap();
    subtree_a.set("count", "1").await.unwrap();
    let entry_a_id = op_a.commit().await.unwrap();

    // Verify A is now the only tip
    let tips_after_a = tree.snapshot().await.unwrap().into_tips();
    assert_eq!(tips_after_a.len(), 1, "Should have exactly 1 tip after A");
    assert_eq!(tips_after_a[0], entry_a_id, "A should be the only tip");

    // Step 2: Create two parallel operations that both use A as parent
    // This creates the diamond fork by having both operations start from the same tip

    // Create operation B - starts from A
    let op_b = tree
        .new_transaction_at(&Snapshot::from(std::slice::from_ref(&entry_a_id)))
        .await
        .unwrap();
    let subtree_b = op_b.get_store::<DocStore>("data").await.unwrap();
    subtree_b.set("shared", "from_B").await.unwrap(); // Override shared field
    subtree_b.set("b_specific", "B_data").await.unwrap(); // Add B-specific data
    subtree_b.set("count", "2").await.unwrap(); // Update count

    // Create operation C - also starts from A (same parent!)
    let op_c = tree
        .new_transaction_at(&Snapshot::from(std::slice::from_ref(&entry_a_id)))
        .await
        .unwrap();
    let subtree_c = op_c.get_store::<DocStore>("data").await.unwrap();
    subtree_c.set("shared", "from_C").await.unwrap(); // Override shared field differently
    subtree_c.set("c_specific", "C_data").await.unwrap(); // Add C-specific data
    subtree_c.set("count", "3").await.unwrap(); // Update count differently

    // Commit both operations - this creates the diamond fork
    let entry_b_id = op_b.commit().await.unwrap();
    let entry_c_id = op_c.commit().await.unwrap();

    // Verify we now have a true diamond: both B and C should be tips with A as parent
    let tips_after_fork = tree.snapshot().await.unwrap().into_tips();
    assert_eq!(
        tips_after_fork.len(),
        2,
        "Should have exactly 2 tips after fork"
    );
    assert!(
        tips_after_fork.contains(&entry_b_id),
        "Entry B should be a tip"
    );
    assert!(
        tips_after_fork.contains(&entry_c_id),
        "Entry C should be a tip"
    );

    // Verify parent relationships - both B and C should have A as their only parent
    {
        let backend = tree.backend().expect("Failed to get backend");
        let entry_b = backend.get(&entry_b_id).await.unwrap();
        let entry_c = backend.get(&entry_c_id).await.unwrap();

        assert_eq!(
            entry_b.parents().unwrap(),
            vec![entry_a_id.clone()],
            "B should have A as parent"
        );
        assert_eq!(
            entry_c.parents().unwrap(),
            vec![entry_a_id.clone()],
            "C should have A as parent"
        );
    }

    // Clear cache to force fresh computation
    tree.backend()
        .expect("Failed to get backend")
        .clear_crdt_cache()
        .await
        .unwrap();

    // Step 3: Create merge operation D that automatically gets both B and C as parents
    let op_d = tree.new_transaction().await.unwrap(); // Uses current tips [B, C]
    let subtree_d = op_d.get_store::<DocStore>("data").await.unwrap();
    subtree_d.set("merge_marker", "D_created").await.unwrap();
    subtree_d.set("final_data", "merged").await.unwrap();
    let entry_d_id = op_d.commit().await.unwrap();

    // Verify D has both B and C as parents (the diamond merge)
    {
        let backend = tree.backend().expect("Failed to get backend");
        let entry_d = backend.get(&entry_d_id).await.unwrap();
        let parents = entry_d.parents().unwrap();

        assert_eq!(parents.len(), 2, "D should have exactly 2 parents");
        assert!(parents.contains(&entry_b_id), "D should have B as parent");
        assert!(parents.contains(&entry_c_id), "D should have C as parent");
    }

    // Step 4: Read the final state - this exercises the LCA algorithm on a true diamond!
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    println!("True diamond pattern final state: {final_state:#?}");

    // With the CORRECT merge-base algorithm:
    // 1. get_full_state() will see tips [B, C] from the operation
    // 2. compute_subtree_state_merge_base([B, C]) will be called
    // 3. find_merge_base([B, C]) = A (common ancestor)
    // 4. compute_single_entry_state_recursive(A) gets State(A)
    // 5. Merge path A->B into State(A)
    // 6. Merge path A->C into the result
    // 7. Apply D's local data

    // All fields from all branches should be present
    assert!(
        final_state.get("base").is_some(),
        "base field from A should be present"
    );
    assert!(
        final_state.get("b_specific").is_some(),
        "b_specific field from B should be present"
    );
    assert!(
        final_state.get("c_specific").is_some(),
        "c_specific field from C should be present"
    );
    assert!(
        final_state.get("merge_marker").is_some(),
        "merge_marker from D should be present"
    );
    assert!(
        final_state.get("final_data").is_some(),
        "final_data from D should be present"
    );

    // Check specific values
    match final_state.get("base").unwrap() {
        Value::Text(v) => assert_eq!(v, "A", "base should be from A"),
        _ => panic!("Expected string for base"),
    }

    match final_state.get("b_specific").unwrap() {
        Value::Text(v) => assert_eq!(v, "B_data", "b_specific should be from B"),
        _ => panic!("Expected string for b_specific"),
    }

    match final_state.get("c_specific").unwrap() {
        Value::Text(v) => assert_eq!(v, "C_data", "c_specific should be from C"),
        _ => panic!("Expected string for c_specific"),
    }

    match final_state.get("merge_marker").unwrap() {
        Value::Text(v) => assert_eq!(v, "D_created", "merge_marker should be from D"),
        _ => panic!("Expected string for merge_marker"),
    }

    match final_state.get("final_data").unwrap() {
        Value::Text(v) => assert_eq!(v, "merged", "final_data should be from D"),
        _ => panic!("Expected string for final_data"),
    }

    // For overlapping fields (shared, count), the result should be deterministic
    // The key insight is that with the correct LCA algorithm, the result is always consistent
    assert!(
        final_state.get("shared").is_some(),
        "shared field should be resolved"
    );
    assert!(
        final_state.get("count").is_some(),
        "count field should be resolved"
    );

    // With the INCORRECT parent-state merging approach, this test would fail because:
    // 1. State(B) computed independently: {base:"A", shared:"from_B", count:"2", b_specific:"B_data"}
    // 2. State(C) computed independently: {base:"A", shared:"from_C", count:"3", c_specific:"C_data"}
    // 3. State(D) = merge(State(B), State(C), LocalData(D))
    // 4. The merge of State(B) and State(C) might not work correctly because they were
    //    computed with different algorithms or orderings
    // 5. Field combinations might be incorrect or inconsistent

    // Verify deterministic behavior - the exact same read should always give same result
    for i in 0..3 {
        let viewer_check = tree.get_store_viewer::<DocStore>("data").await.unwrap();
        let state_check = viewer_check.get_all().await.unwrap();
        assert_eq!(
            final_state, state_check,
            "Diamond merge should be deterministic on read {i}"
        );
    }

    println!("✓ TRUE diamond pattern test passed!");
    println!("  Created real diamond DAG: A->B, A->C, [B,C]->D");
    println!("  This test WOULD FAIL with incorrect parent-state merging approach");
    println!("  LCA algorithm correctly handles complex ancestry with proper field merging");
}

/// Test helper functions for complex merge scenarios
#[tokio::test]
async fn test_merge_algorithm_helpers() {
    let (_instance, tree) = setup_tree().await;

    // Test diamond pattern creation helper
    let base_data = &[("foundation", "solid"), ("version", "1.0")];
    let (base_id, branch_b_id, branch_c_id, merge_id) =
        create_diamond_pattern(&tree, base_data).await;

    // Verify diamond structure
    assert_entry_parents(&tree, &branch_b_id, std::slice::from_ref(&base_id)).await;
    assert_entry_parents(&tree, &branch_c_id, std::slice::from_ref(&base_id)).await;
    assert_entry_parents(&tree, &merge_id, &[branch_b_id, branch_c_id]).await;

    // Verify final state contains expected non-conflicting data
    assert_subtree_data(
        &tree,
        "data",
        &[
            ("foundation", "solid"),
            ("version", "1.0"),
            ("b_specific", "B_data"),
            ("c_specific", "C_data"),
            ("merge", "D"),
            ("final", "merged"),
        ],
    )
    .await;

    // Verify that conflicting field "branch" has one of the expected values
    let viewer = tree
        .get_store_viewer::<DocStore>("data")
        .await
        .expect("Failed to get viewer");
    let branch_value = viewer
        .get_string("branch")
        .await
        .expect("Should have branch value");
    assert!(
        branch_value == "B" || branch_value == "C",
        "Branch should be either B or C, got: {branch_value}"
    );

    // Test deterministic reads
    assert_deterministic_reads(&tree, "data", 5).await;

    // Test caching consistency
    assert_caching_consistency(&tree, "data").await;
}

/// Test performance with deep chains
#[tokio::test]
async fn test_merge_performance_with_deep_chains() {
    let (_instance, tree) = setup_tree().await;

    // Create deep chain and verify performance
    assert_deep_operations_performance(&tree, 100).await;

    // Test linear chain creation helper
    let chain_ids = create_linear_chain(&tree, "performance", 20).await;
    assert_eq!(chain_ids.len(), 20);

    // Verify chain structure - each entry should have previous as parent (except first)
    for i in 1..chain_ids.len() {
        assert_entry_parents(&tree, &chain_ids[i], &[chain_ids[i - 1].clone()]).await;
    }

    // Verify final state has all accumulated data
    let viewer = tree
        .get_store_viewer::<DocStore>("performance")
        .await
        .expect("Failed to get viewer");
    let final_state = viewer.get_all().await.expect("Failed to get final state");

    // Should have final step value
    assert_eq!(
        final_state.get("step").unwrap(),
        &Value::Text("19".to_string())
    );

    // Should have all step-specific values
    for i in 0..20 {
        let step_key = format!("step_{i}");
        let expected_value = format!("value_{i}");
        assert_eq!(
            final_state.get(&step_key).unwrap(),
            &Value::Text(expected_value)
        );
    }
}

/// Test merge base finding with shallow divergence (within single batch limit).
///
/// Creates a diamond pattern where tips are ~50 entries away from merge base.
/// This should be resolved in a single batch (batch limit is 100).
#[tokio::test]
async fn test_find_merge_base_shallow_divergence() {
    let (_instance, tree) = setup_tree().await;

    // Create base entry (merge base)
    let txn_base = tree.new_transaction().await.unwrap();
    let subtree_base = txn_base.get_store::<DocStore>("data").await.unwrap();
    subtree_base.set("base", "root").await.unwrap();
    let base_id = txn_base.commit().await.unwrap();

    // Build two chains of 50 entries each from the base
    const CHAIN_DEPTH: usize = 50;

    // Chain A: 50 entries from base
    let mut chain_a_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_a_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_a_step", i.to_string()).await.unwrap();
        chain_a_tip = txn.commit().await.unwrap();
    }

    // Chain B: 50 entries from base (creates diamond)
    let mut chain_b_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_b_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_b_step", i.to_string()).await.unwrap();
        chain_b_tip = txn.commit().await.unwrap();
    }

    // Create merge operation from both tips
    let merge_tips = vec![chain_a_tip.clone(), chain_b_tip.clone()];
    let op_merge = tree
        .new_transaction_at(&Snapshot::from(&merge_tips))
        .await
        .unwrap();
    let subtree_merge = op_merge.get_store::<DocStore>("data").await.unwrap();
    subtree_merge.set("merged", "true").await.unwrap();
    let _merge_id = op_merge.commit().await.unwrap();

    // Verify the final state includes data from both chains
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Should have base data
    assert_eq!(
        final_state.get("base").unwrap(),
        &Value::Text("root".to_string())
    );

    // Should have final chain A step
    assert_eq!(
        final_state.get("chain_a_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have final chain B step
    assert_eq!(
        final_state.get("chain_b_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have merge marker
    assert_eq!(
        final_state.get("merged").unwrap(),
        &Value::Text("true".to_string())
    );

    println!("✓ Shallow divergence test passed - merge base found in single batch");
}

/// Test merge base finding with deep divergence (exceeds single batch limit).
///
/// Creates a diamond pattern where tips are ~150 entries away from merge base.
/// This requires multi-batch continuation (batch limit is 100).
#[tokio::test]
async fn test_find_merge_base_deep_divergence() {
    let (_instance, tree) = setup_tree().await;

    // Create base entry (merge base)
    let txn_base = tree.new_transaction().await.unwrap();
    let subtree_base = txn_base.get_store::<DocStore>("data").await.unwrap();
    subtree_base.set("base", "deep_root").await.unwrap();
    let base_id = txn_base.commit().await.unwrap();

    // Build two chains of 150 entries each from the base (exceeds 100 batch limit)
    const CHAIN_DEPTH: usize = 150;

    // Chain A: 150 entries from base
    let mut chain_a_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_a_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_a_step", i.to_string()).await.unwrap();
        chain_a_tip = txn.commit().await.unwrap();
    }

    // Chain B: 150 entries from base (creates deep diamond)
    let mut chain_b_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_b_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_b_step", i.to_string()).await.unwrap();
        chain_b_tip = txn.commit().await.unwrap();
    }

    // Create merge operation from both tips
    let merge_tips = vec![chain_a_tip.clone(), chain_b_tip.clone()];
    let op_merge = tree
        .new_transaction_at(&Snapshot::from(&merge_tips))
        .await
        .unwrap();
    let subtree_merge = op_merge.get_store::<DocStore>("data").await.unwrap();
    subtree_merge.set("deep_merged", "true").await.unwrap();
    let _merge_id = op_merge.commit().await.unwrap();

    // Verify the final state includes data from both chains
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Should have base data
    assert_eq!(
        final_state.get("base").unwrap(),
        &Value::Text("deep_root".to_string())
    );

    // Should have final chain A step
    assert_eq!(
        final_state.get("chain_a_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have final chain B step
    assert_eq!(
        final_state.get("chain_b_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have merge marker
    assert_eq!(
        final_state.get("deep_merged").unwrap(),
        &Value::Text("true".to_string())
    );

    println!("✓ Deep divergence test passed - merge base found via multi-batch continuation");
}

/// Test merge base finding with very deep chains (multiple batch iterations needed).
///
/// Creates a diamond pattern where tips are ~250 entries away from merge base.
/// This requires 3+ batch iterations (batch limit is 100).
#[tokio::test]
async fn test_find_merge_base_very_deep_chains() {
    let (_instance, tree) = setup_tree().await;

    // Create base entry (merge base)
    let txn_base = tree.new_transaction().await.unwrap();
    let subtree_base = txn_base.get_store::<DocStore>("data").await.unwrap();
    subtree_base.set("base", "very_deep_root").await.unwrap();
    let base_id = txn_base.commit().await.unwrap();

    // Build two chains of 250 entries each from the base (requires 3 batches)
    const CHAIN_DEPTH: usize = 250;

    // Chain A: 250 entries from base
    let mut chain_a_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_a_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_a_step", i.to_string()).await.unwrap();
        chain_a_tip = txn.commit().await.unwrap();
    }

    // Chain B: 250 entries from base (creates very deep diamond)
    let mut chain_b_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_b_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_b_step", i.to_string()).await.unwrap();
        chain_b_tip = txn.commit().await.unwrap();
    }

    // Create merge operation from both tips
    let merge_tips = vec![chain_a_tip.clone(), chain_b_tip.clone()];
    let op_merge = tree
        .new_transaction_at(&Snapshot::from(&merge_tips))
        .await
        .unwrap();
    let subtree_merge = op_merge.get_store::<DocStore>("data").await.unwrap();
    subtree_merge.set("very_deep_merged", "true").await.unwrap();
    let _merge_id = op_merge.commit().await.unwrap();

    // Verify the final state includes data from both chains
    let viewer = tree.get_store_viewer::<DocStore>("data").await.unwrap();
    let final_state = viewer.get_all().await.unwrap();

    // Should have base data
    assert_eq!(
        final_state.get("base").unwrap(),
        &Value::Text("very_deep_root".to_string())
    );

    // Should have final chain A step
    assert_eq!(
        final_state.get("chain_a_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have final chain B step
    assert_eq!(
        final_state.get("chain_b_step").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string())
    );

    // Should have merge marker
    assert_eq!(
        final_state.get("very_deep_merged").unwrap(),
        &Value::Text("true".to_string())
    );

    println!("✓ Very deep chains test passed - merge base found via 3+ batch iterations");
}

/// Test that actually triggers find_merge_base by reading state during merge.
///
/// The previous tests read state AFTER commit when there's only one tip.
/// This test reads state DURING the merge transaction when there are multiple tips,
/// which actually exercises the find_merge_base code path.
#[tokio::test]
async fn test_find_merge_base_actually_called() {
    let (_instance, tree) = setup_tree().await;

    // Create base entry (merge base)
    let txn_base = tree.new_transaction().await.unwrap();
    let subtree_base = txn_base.get_store::<DocStore>("data").await.unwrap();
    subtree_base.set("base", "root").await.unwrap();
    let base_id = txn_base.commit().await.unwrap();

    // Build two chains that exceed the batch limit (100)
    const CHAIN_DEPTH: usize = 150;

    // Chain A
    let mut chain_a_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_a_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_a", i.to_string()).await.unwrap();
        chain_a_tip = txn.commit().await.unwrap();
    }

    // Chain B
    let mut chain_b_tip = base_id.clone();
    for i in 0..CHAIN_DEPTH {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&chain_b_tip)))
            .await
            .unwrap();
        let subtree = txn.get_store::<DocStore>("data").await.unwrap();
        subtree.set("chain_b", i.to_string()).await.unwrap();
        chain_b_tip = txn.commit().await.unwrap();
    }

    // Create merge transaction with BOTH tips
    let merge_tips = vec![chain_a_tip.clone(), chain_b_tip.clone()];
    let op_merge = tree
        .new_transaction_at(&Snapshot::from(&merge_tips))
        .await
        .unwrap();
    let subtree_merge = op_merge.get_store::<DocStore>("data").await.unwrap();

    // THIS is the key: read state DURING the merge transaction
    // This triggers find_merge_base with multiple tips (chain_a_tip, chain_b_tip)
    let state_during_merge = subtree_merge.get_all().await.unwrap();

    // Verify we got data from both chains
    assert_eq!(
        state_during_merge.get("base").unwrap(),
        &Value::Text("root".to_string()),
        "Should have base data"
    );
    assert_eq!(
        state_during_merge.get("chain_a").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string()),
        "Should have chain A data"
    );
    assert_eq!(
        state_during_merge.get("chain_b").unwrap(),
        &Value::Text((CHAIN_DEPTH - 1).to_string()),
        "Should have chain B data"
    );

    // Now commit
    subtree_merge.set("merged", "true").await.unwrap();
    op_merge.commit().await.unwrap();

    println!("✓ find_merge_base actually called and succeeded with deep chains");
}

/// PROBE (throwaway): cost of a sustained criss-cross topology.
///
/// Each round creates two entries whose store parents are BOTH current tips,
/// so no single ancestor dominates the new tip pair and merge-base resolution
/// keeps returning None. Prints entries folded per round.
#[tokio::test]
async fn probe_criss_cross_refold_cost() {
    use eidetica::transaction::probe;

    let (_instance, tree) = setup_tree().await;

    // Seed two independent-ish tips from one root.
    let root = {
        let txn = tree.new_transaction().await.unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        s.set("seed", "0").await.unwrap();
        txn.commit().await.unwrap()
    };

    let mut left = {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&root)))
            .await
            .unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        s.set("left", "0").await.unwrap();
        txn.commit().await.unwrap()
    };
    let mut right = {
        let txn = tree
            .new_transaction_at(&Snapshot::from(std::slice::from_ref(&root)))
            .await
            .unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        s.set("right", "0").await.unwrap();
        txn.commit().await.unwrap()
    };

    const ROUNDS: usize = 40;
    probe::take(&probe::FOLDED_ENTRIES);
    probe::take(&probe::MULTI_TIP_MISSES);
    probe::take(&probe::EMPTY_BASE_REFOLDS);

    let mut total = 0u64;
    for i in 0..ROUNDS {
        let tips = Snapshot::from(vec![left.clone(), right.clone()]);

        // Two concurrent merges of the same tip pair: classic criss-cross.
        let new_left = {
            let txn = tree.new_transaction_at(&tips).await.unwrap();
            let s = txn.get_store::<DocStore>("data").await.unwrap();
            s.set("left", i.to_string()).await.unwrap();
            txn.commit().await.unwrap()
        };
        let new_right = {
            let txn = tree.new_transaction_at(&tips).await.unwrap();
            let s = txn.get_store::<DocStore>("data").await.unwrap();
            s.set("right", i.to_string()).await.unwrap();
            txn.commit().await.unwrap()
        };
        left = new_left;
        right = new_right;

        // Read the merged state at the new tip pair — what any reader does.
        let read_tips = Snapshot::from(vec![left.clone(), right.clone()]);
        let txn = tree.new_transaction_at(&read_tips).await.unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        let state = s.get_all().await.unwrap();
        assert!(state.get("seed").is_some());

        let folded = probe::take(&probe::FOLDED_ENTRIES);
        let misses = probe::take(&probe::MULTI_TIP_MISSES);
        let empty = probe::take(&probe::EMPTY_BASE_REFOLDS);
        total += folded;
        println!(
            "round {i:>3}  entries_in_store={:>4}  folded={folded:>6}  \
             multi_tip_misses={misses}  empty_base_refolds={empty}  cumulative_folded={total}",
            3 + 2 * (i + 1)
        );
    }
    println!("PROBE total folded entries over {ROUNDS} rounds: {total}");
}

/// PROBE (throwaway): sustained criss-cross over two DISJOINT store roots.
///
/// The store is created independently on both sides of a main-tree fork, so
/// the store history has two roots and merge-base resolution returns None
/// every round.
#[tokio::test]
async fn probe_disjoint_root_refold_cost() {
    use eidetica::transaction::probe;

    let (_instance, tree) = setup_tree().await;

    // Seed in a DIFFERENT store, so "data" does not exist yet.
    let seed = {
        let txn = tree.new_transaction().await.unwrap();
        let s = txn.get_store::<DocStore>("other").await.unwrap();
        s.set("seed", "0").await.unwrap();
        txn.commit().await.unwrap()
    };
    let fork = Snapshot::from(std::slice::from_ref(&seed));

    // Two independent first writes to "data": two store roots.
    let mut left = {
        let txn = tree.new_transaction_at(&fork).await.unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        s.set("left", "seed").await.unwrap();
        txn.commit().await.unwrap()
    };
    let mut right = {
        let txn = tree.new_transaction_at(&fork).await.unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        s.set("right", "seed").await.unwrap();
        txn.commit().await.unwrap()
    };

    const ROUNDS: usize = 40;
    probe::take(&probe::FOLDED_ENTRIES);
    probe::take(&probe::MULTI_TIP_MISSES);
    probe::take(&probe::EMPTY_BASE_REFOLDS);

    let mut total = 0u64;
    for i in 0..ROUNDS {
        let tips = Snapshot::from(vec![left.clone(), right.clone()]);
        let new_left = {
            let txn = tree.new_transaction_at(&tips).await.unwrap();
            let s = txn.get_store::<DocStore>("data").await.unwrap();
            s.set("left", i.to_string()).await.unwrap();
            txn.commit().await.unwrap()
        };
        let new_right = {
            let txn = tree.new_transaction_at(&tips).await.unwrap();
            let s = txn.get_store::<DocStore>("data").await.unwrap();
            s.set("right", i.to_string()).await.unwrap();
            txn.commit().await.unwrap()
        };
        left = new_left;
        right = new_right;

        let read_tips = Snapshot::from(vec![left.clone(), right.clone()]);
        let txn = tree.new_transaction_at(&read_tips).await.unwrap();
        let s = txn.get_store::<DocStore>("data").await.unwrap();
        let state = s.get_all().await.unwrap();
        assert!(state.get("left").is_some() && state.get("right").is_some());

        let folded = probe::take(&probe::FOLDED_ENTRIES);
        let misses = probe::take(&probe::MULTI_TIP_MISSES);
        let empty = probe::take(&probe::EMPTY_BASE_REFOLDS);
        total += folded;
        println!(
            "round {i:>3}  store_entries={:>4}  folded={folded:>6}  \
             multi_tip_misses={misses}  empty_base_refolds={empty}  cumulative_folded={total}",
            2 + 2 * (i + 1)
        );
    }
    println!("PROBE(disjoint) total folded entries over {ROUNDS} rounds: {total}");
}

/// PROBE (throwaway): the segmentation law the incremental design rests on.
///
/// A fold over the total entry order is a monoid product, so any contiguous
/// segmentation must give the same answer as the flat left fold:
///     fold(s1 ++ s2 ++ ... ++ sk) == fold(s1) ⊕ fold(s2) ⊕ ... ⊕ fold(sk)
/// with `Doc::default()` as the identity. Checked over a deterministic
/// pseudo-random sequence including nested paths, tombstones and atomic docs.
#[tokio::test]
async fn probe_fold_segmentation_law() {
    use eidetica::crdt::{CRDT, Doc};

    // xorshift, so the sequence is deterministic without a dev-dependency.
    let mut state: u64 = 0x2545F4914F6CDD1D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let keys = ["a", "b", "c", "n.x", "n.y", "n.deep.z"];
    let mut deltas: Vec<Doc> = Vec::new();
    for i in 0..200u64 {
        let r = next();
        let mut d = if r % 17 == 0 {
            Doc::atomic()
        } else {
            Doc::new()
        };
        let k = keys[(r >> 8) as usize % keys.len()];
        if r % 11 == 0 {
            d.remove(k);
        } else {
            d.set(k, format!("v{i}"));
        }
        if r % 5 == 0 {
            let k2 = keys[(r >> 16) as usize % keys.len()];
            d.set(k2, format!("w{i}"));
        }
        deltas.push(d);
    }

    let flat = deltas
        .iter()
        .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());

    // Every prefix-split, plus a few multi-segment splits.
    for cut in 0..=deltas.len() {
        let l = deltas[..cut]
            .iter()
            .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());
        let r = deltas[cut..]
            .iter()
            .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());
        assert_eq!(
            l.merge(&r).unwrap(),
            flat,
            "segmentation law broken at cut {cut}"
        );
    }

    for chunk in [1usize, 2, 3, 7, 16, 64] {
        let product = deltas
            .chunks(chunk)
            .map(|seg| {
                seg.iter()
                    .fold(Doc::default(), |acc, d| acc.merge(d).unwrap())
            })
            .fold(Doc::default(), |acc, seg| acc.merge(&seg).unwrap());
        assert_eq!(
            product, flat,
            "segmentation law broken at chunk size {chunk}"
        );
    }

    println!("PROBE: segmentation law holds over 200 deltas for every prefix cut and chunk size");
}

/// PROBE (throwaway): two things at once.
///
/// 1. Negative control — merging per-tip cached states is NOT a valid way to
///    combine branches: interleaved writes give the wrong answer in either
///    order. (This is why the fix is a segment product, not a state join.)
/// 2. Cost simulation — a fanout-B product tree over the entry order, kept
///    incrementally, versus the flat refold, counting merge operations per
///    round of a sustained criss-cross.
#[tokio::test]
async fn probe_incremental_product_tree_simulation() {
    use eidetica::crdt::{CRDT, Doc};

    // --- 1. negative control -------------------------------------------------
    // Global order e1 < e2 < e3 < e4. Branch A = {e1, e4}, branch B = {e2, e3}.
    let mk = |k: &str, v: &str| {
        let mut d = Doc::new();
        d.set(k, v);
        d
    };
    let (e1, e2, e3, e4) = (mk("k", "a1"), mk("k", "b2"), mk("m", "b3"), mk("m", "a4"));
    let flat = [&e1, &e2, &e3, &e4]
        .into_iter()
        .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());
    let state_a = e1.merge(&e4).unwrap(); // fold of branch A alone
    let state_b = e2.merge(&e3).unwrap(); // fold of branch B alone
    assert_ne!(state_a.merge(&state_b).unwrap(), flat, "A⊕B should differ");
    assert_ne!(state_b.merge(&state_a).unwrap(), flat, "B⊕A should differ");
    println!(
        "PROBE negative control: flat={flat:?} A⊕B={:?} B⊕A={:?}",
        state_a.merge(&state_b).unwrap(),
        state_b.merge(&state_a).unwrap()
    );

    // --- 2. cost simulation --------------------------------------------------
    const B: usize = 8;
    const ROUNDS: usize = 200;

    let mut deltas: Vec<Doc> = Vec::new();
    // levels[0] is deltas; levels[l] holds products of B nodes of level l-1.
    let mut levels: Vec<Vec<Doc>> = Vec::new();
    let mut dirty_from: Vec<usize> = Vec::new(); // per level, first stale index

    let mut flat_merges_total = 0u64;
    let mut tree_merges_total = 0u64;

    for round in 0..ROUNDS {
        // Two concurrent writers each append one entry to the order.
        for side in 0..2 {
            let mut d = Doc::new();
            d.set(if side == 0 { "left" } else { "right" }, round.to_string());
            deltas.push(d);
        }
        let first_new = deltas.len() - 2;

        // Flat refold: one merge per entry in the store.
        let flat_state = deltas
            .iter()
            .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());
        flat_merges_total += deltas.len() as u64;

        // Incremental product tree: recompute only nodes covering new positions.
        let mut tree_merges = 0u64;
        let mut child_len = deltas.len();
        let mut child_dirty = first_new;
        let mut level = 0;
        loop {
            let node_count = child_len.div_ceil(B);
            if levels.len() <= level {
                levels.push(Vec::new());
                dirty_from.push(0);
            }
            let start = child_dirty / B;
            levels[level].resize(node_count, Doc::default());
            for node in start..node_count {
                let lo = node * B;
                let hi = usize::min(lo + B, child_len);
                let product = if level == 0 {
                    deltas[lo..hi]
                        .iter()
                        .fold(Doc::default(), |acc, d| acc.merge(d).unwrap())
                } else {
                    levels[level - 1][lo..hi]
                        .iter()
                        .fold(Doc::default(), |acc, d| acc.merge(d).unwrap())
                };
                tree_merges += (hi - lo) as u64;
                levels[level][node] = product;
            }
            if node_count <= 1 {
                break;
            }
            child_len = node_count;
            child_dirty = start;
            level += 1;
        }
        let top = levels.len() - 1;
        let tree_state = levels[top]
            .iter()
            .fold(Doc::default(), |acc, d| acc.merge(d).unwrap());
        tree_merges += levels[top].len() as u64;
        tree_merges_total += tree_merges;

        assert_eq!(
            tree_state, flat_state,
            "product tree diverged at round {round}"
        );

        if round % 20 == 19 || round < 3 {
            println!(
                "round {round:>4}  entries={:>5}  flat_merges={:>6}  tree_merges={tree_merges:>4}  \
                 cumulative flat={flat_merges_total:>8} tree={tree_merges_total:>7}",
                deltas.len(),
                deltas.len(),
            );
        }
    }
    println!(
        "PROBE simulation: {ROUNDS} rounds, {} entries — flat {flat_merges_total} merges, \
         incremental {tree_merges_total} merges ({:.1}x less)",
        deltas.len(),
        flat_merges_total as f64 / tree_merges_total as f64
    );
}
