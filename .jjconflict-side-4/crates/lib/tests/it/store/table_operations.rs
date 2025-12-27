//! Table subtree operation tests
//!
//! This module contains tests for Table subtree functionality including
//! CRUD operations, search functionality, UUID generation, and multiple operations.

use eidetica::store::Table;

use super::helpers::*;
use crate::helpers::*;

#[tokio::test]
async fn test_table_basic_crud_operations() {
    let ctx = TestContext::new().with_database().await;

    // Use helper to create initial record
    let initial_record = TestRecord {
        name: "John Doe".to_string(),
        age: 30,
        email: "john@example.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "test_records",
        std::slice::from_ref(&initial_record),
    )
    .await;
    let primary_key = &keys[0];

    // Test CRUD operations within a transaction
    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    let table = txn
        .get_store::<Table<TestRecord>>("test_records")
        .await
        .expect("Failed to get Table");

    // Test get (should see existing record)
    let retrieved = table
        .get(primary_key)
        .await
        .expect("Failed to get existing record");
    assert_eq!(retrieved, initial_record);

    // Test update/set
    let updated_record = TestRecord {
        name: "John Smith".to_string(),
        age: 31,
        email: "john.smith@example.com".to_string(),
    };
    table
        .set(primary_key, updated_record.clone())
        .await
        .expect("Failed to update record");

    // Verify update within same operation
    let retrieved_updated = table
        .get(primary_key)
        .await
        .expect("Failed to get updated record");
    assert_eq!(retrieved_updated, updated_record);

    // Test insert of new record
    let new_record = TestRecord {
        name: "Jane Doe".to_string(),
        age: 25,
        email: "jane@example.com".to_string(),
    };
    let new_pk = table
        .insert(new_record.clone())
        .await
        .expect("Failed to insert new record");
    assert!(!new_pk.is_empty(), "New primary key should not be empty");

    // Verify new record retrieval
    let retrieved_new = table.get(&new_pk).await.expect("Failed to get new record");
    assert_eq!(retrieved_new, new_record);

    txn.commit().await.expect("Failed to commit transaction");

    // Verify persistence using helper
    assert_table_record(ctx.database(), "test_records", primary_key, &updated_record).await;
    assert_table_record(ctx.database(), "test_records", &new_pk, &new_record).await;
}

#[tokio::test]
async fn test_table_multiple_records() {
    let ctx = TestContext::new().with_database().await;

    // Use helper to create multiple records
    let values = &[10, 20, 30, 40, 50];
    let inserted_keys =
        create_simple_table_operation(ctx.database(), "simple_records", values).await;

    // Verify all records persist after commit
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<SimpleRecord>>("simple_records")
        .await
        .expect("Failed to get Table viewer");

    for (i, key) in inserted_keys.iter().enumerate() {
        let record = viewer
            .get(key)
            .await
            .expect("Failed to get record after commit");
        assert_eq!(record.value, values[i]);
    }
}

#[tokio::test]
async fn test_table_search_functionality() {
    let ctx = TestContext::new().with_database().await;

    // Use helper to create test records
    let records = create_test_records();
    create_table_operation(ctx.database(), "search_records", &records).await;

    // Test search by age using helper
    assert_table_search_count(
        ctx.database(),
        "search_records",
        |record| record.age == 25,
        2,
    )
    .await;

    // Test search by email domain using helper
    assert_table_search_count(
        ctx.database(),
        "search_records",
        |record| record.email.contains("example.com"),
        2,
    )
    .await;

    // Test search by name prefix using helper
    assert_table_search_count(
        ctx.database(),
        "search_records",
        |record| record.name.starts_with('B'),
        1,
    )
    .await;

    // Test search with no matches using helper
    assert_table_search_count(
        ctx.database(),
        "search_records",
        |record| record.age > 100,
        0,
    )
    .await;

    // Test search after commit with detailed verification
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("search_records")
        .await
        .expect("Failed to get Table viewer");

    let age_30_results = viewer
        .search(|record| record.age == 30)
        .await
        .expect("Failed to search after commit");
    assert_eq!(age_30_results.len(), 1);
    assert_eq!(age_30_results[0].1.name, "Bob Smith");
}

#[tokio::test]
async fn test_table_uuid_generation() {
    let ctx = TestContext::new().with_database().await;

    // Generate 100 records to test UUID uniqueness
    let values: Vec<i32> = (1..=100).collect();
    let generated_keys = create_simple_table_operation(ctx.database(), "uuid_test", &values).await;

    // Use helper to verify UUID format and uniqueness
    assert_valid_uuids(&generated_keys);

    // Verify all records are retrievable with their unique keys
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<SimpleRecord>>("uuid_test")
        .await
        .expect("Failed to get Table viewer");

    for key in &generated_keys {
        let record = viewer.get(key).await.expect("Failed to get record by UUID");
        assert!(record.value >= 1 && record.value <= 100);
    }
}

#[tokio::test]
async fn test_table_multiple_operations() {
    let ctx = TestContext::new().with_database().await;

    // Use helper to test multi-operation workflow
    let (key1, key2, key3) = test_table_multi_operations(ctx.database(), "multi_op_test").await;

    // Verify final state
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("multi_op_test")
        .await
        .expect("Failed to get Table viewer");

    // Check updated record
    let final_record1 = viewer
        .get(&key1)
        .await
        .expect("Failed to get final record1");
    assert_eq!(final_record1.name, "Updated User 1");
    assert_eq!(final_record1.age, 21);
    assert_eq!(final_record1.email, "user1@updated.com");

    // Check unchanged record
    let final_record2 = viewer
        .get(&key2)
        .await
        .expect("Failed to get final record2");
    assert_eq!(final_record2.name, "Initial User 2");
    assert_eq!(final_record2.age, 25);
    assert_eq!(final_record2.email, "user2@initial.com");

    // Check new record
    let final_record3 = viewer
        .get(&key3)
        .await
        .expect("Failed to get final record3");
    assert_eq!(final_record3.name, "New User 3");
    assert_eq!(final_record3.age, 30);
    assert_eq!(final_record3.email, "user3@new.com");

    // Verify search across all records
    let all_records = viewer
        .search(|_| true)
        .await
        .expect("Failed to search all records");
    assert_eq!(all_records.len(), 3);
}

#[tokio::test]
async fn test_table_empty_search() {
    let ctx = TestContext::new().with_database().await;
    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");

    {
        let table = txn
            .get_store::<Table<SimpleRecord>>("empty_search_test")
            .await
            .expect("Failed to get Table");

        // Search in empty store
        let results = table
            .search(|_| true)
            .await
            .expect("Failed to search empty store");
        assert_eq!(results.len(), 0);
    }

    txn.commit().await.expect("Failed to commit transaction");

    // Search in empty store after commit
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<SimpleRecord>>("empty_search_test")
        .await
        .expect("Failed to get Table viewer");

    let results = viewer
        .search(|_| true)
        .await
        .expect("Failed to search empty store after commit");
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_empty_table_behavior() {
    let ctx = TestContext::new().with_database().await;

    // Test empty Table behavior
    let table_viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("empty_table")
        .await
        .expect("Failed to get empty Table viewer");

    let empty_search = table_viewer
        .search(|_| true)
        .await
        .expect("Failed to search empty table");
    assert_eq!(empty_search.len(), 0);
}

#[tokio::test]
async fn test_table_delete_basic() {
    let ctx = TestContext::new().with_database().await;

    // Create initial records using helper
    let initial_records = vec![
        TestRecord {
            name: "User 1".to_string(),
            age: 25,
            email: "user1@test.com".to_string(),
        },
        TestRecord {
            name: "User 2".to_string(),
            age: 30,
            email: "user2@test.com".to_string(),
        },
        TestRecord {
            name: "User 3".to_string(),
            age: 35,
            email: "user3@test.com".to_string(),
        },
    ];
    let keys = create_table_operation(ctx.database(), "delete_test", &initial_records).await;

    // Delete one record within a transaction
    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn
            .get_store::<Table<TestRecord>>("delete_test")
            .await
            .expect("Failed to get Table");

        // Delete existing record
        let deleted = table
            .delete(&keys[1])
            .await
            .expect("Failed to delete existing record");
        assert!(deleted, "Should return true when deleting existing record");

        // Verify deletion within same operation
        assert!(
            table.get(&keys[1]).await.is_err(),
            "Deleted record should not be retrievable"
        );

        // Verify other records still exist
        let record1 = table
            .get(&keys[0])
            .await
            .expect("Record 1 should still exist");
        assert_eq!(record1.name, "User 1");

        let record3 = table
            .get(&keys[2])
            .await
            .expect("Record 3 should still exist");
        assert_eq!(record3.name, "User 3");
    }
    txn.commit().await.expect("Failed to commit transaction");

    // Verify deletion persisted using helper
    assert_table_record_deleted(ctx.database(), "delete_test", &keys[1]).await;

    // Verify other records still exist
    assert_table_record(ctx.database(), "delete_test", &keys[0], &initial_records[0]).await;
    assert_table_record(ctx.database(), "delete_test", &keys[2], &initial_records[2]).await;
}

#[tokio::test]
async fn test_table_delete_nonexistent() {
    let ctx = TestContext::new().with_database().await;

    // Create one record
    let record = TestRecord {
        name: "Existing User".to_string(),
        age: 30,
        email: "existing@test.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "delete_nonexistent",
        std::slice::from_ref(&record),
    )
    .await;

    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn
            .get_store::<Table<TestRecord>>("delete_nonexistent")
            .await
            .expect("Failed to get Table");

        // Try to delete non-existent key
        let deleted = table
            .delete("non-existent-uuid")
            .await
            .expect("Delete should not error on non-existent key");
        assert!(
            !deleted,
            "Should return false when deleting non-existent record"
        );

        // Verify existing record is still there
        let existing = table
            .get(&keys[0])
            .await
            .expect("Existing record should remain");
        assert_eq!(existing.name, "Existing User");
    }
    txn.commit().await.expect("Failed to commit transaction");

    // Verify existing record persisted
    assert_table_record(ctx.database(), "delete_nonexistent", &keys[0], &record).await;
}

#[tokio::test]
async fn test_table_delete_and_reinsert() {
    let ctx = TestContext::new().with_database().await;

    // Create initial record
    let initial_record = TestRecord {
        name: "Original User".to_string(),
        age: 25,
        email: "original@test.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "delete_reinsert",
        std::slice::from_ref(&initial_record),
    )
    .await;
    let original_key = &keys[0];

    // Delete the record
    let txn1 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn1
            .get_store::<Table<TestRecord>>("delete_reinsert")
            .await
            .expect("Failed to get Table");

        table
            .delete(original_key)
            .await
            .expect("Failed to delete record");
    }
    txn1.commit().await.expect("Failed to commit deletion");

    // Verify deletion
    assert_table_record_deleted(ctx.database(), "delete_reinsert", original_key).await;

    // Re-insert with the same key
    let txn2 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn2
            .get_store::<Table<TestRecord>>("delete_reinsert")
            .await
            .expect("Failed to get Table");

        let new_record = TestRecord {
            name: "New User".to_string(),
            age: 30,
            email: "new@test.com".to_string(),
        };

        table
            .set(original_key, new_record.clone())
            .await
            .expect("Failed to re-insert record");

        // Verify re-inserted record is retrievable
        let retrieved = table
            .get(original_key)
            .await
            .expect("Re-inserted record should be retrievable");
        assert_eq!(retrieved, new_record);
    }
    txn2.commit().await.expect("Failed to commit re-insertion");

    // Verify new record persisted with same key
    let new_record = TestRecord {
        name: "New User".to_string(),
        age: 30,
        email: "new@test.com".to_string(),
    };
    assert_table_record(ctx.database(), "delete_reinsert", original_key, &new_record).await;
}

#[tokio::test]
async fn test_table_search_after_delete() {
    let ctx = TestContext::new().with_database().await;

    // Create test records using helper
    let records = create_test_records();
    let keys = create_table_operation(ctx.database(), "search_after_delete", &records).await;

    // Verify initial search count
    assert_table_search_count(
        ctx.database(),
        "search_after_delete",
        |record| record.age == 25,
        2,
    )
    .await;

    // Delete one of the age=25 records
    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn
            .get_store::<Table<TestRecord>>("search_after_delete")
            .await
            .expect("Failed to get Table");

        table
            .delete(&keys[0])
            .await
            .expect("Failed to delete record");
    }
    txn.commit().await.expect("Failed to commit deletion");

    // Verify search count decreased
    assert_table_search_count(
        ctx.database(),
        "search_after_delete",
        |record| record.age == 25,
        1,
    )
    .await;

    // Verify the remaining age=25 record is the correct one
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("search_after_delete")
        .await
        .expect("Failed to get Table viewer");

    let age_25_results = viewer
        .search(|record| record.age == 25)
        .await
        .expect("Failed to search after delete");
    assert_eq!(age_25_results.len(), 1);
    assert_eq!(age_25_results[0].1.name, "Charlie Brown");
}

#[tokio::test]
async fn test_table_delete_multiple() {
    let ctx = TestContext::new().with_database().await;

    // Create multiple records
    let values = &[10, 20, 30, 40, 50];
    let keys = create_simple_table_operation(ctx.database(), "delete_multiple", values).await;

    // Delete multiple records in one transaction
    let txn = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    {
        let table = txn
            .get_store::<Table<SimpleRecord>>("delete_multiple")
            .await
            .expect("Failed to get Table");

        // Delete records at indices 1 and 3
        let deleted1 = table
            .delete(&keys[1])
            .await
            .expect("Failed to delete record 1");
        let deleted3 = table
            .delete(&keys[3])
            .await
            .expect("Failed to delete record 3");

        assert!(deleted1);
        assert!(deleted3);

        // Verify deletions
        assert!(table.get(&keys[1]).await.is_err());
        assert!(table.get(&keys[3]).await.is_err());

        // Verify remaining records
        assert_eq!(
            table.get(&keys[0]).await.expect("Record 0 exists").value,
            10
        );
        assert_eq!(
            table.get(&keys[2]).await.expect("Record 2 exists").value,
            30
        );
        assert_eq!(
            table.get(&keys[4]).await.expect("Record 4 exists").value,
            50
        );
    }
    txn.commit().await.expect("Failed to commit deletions");

    // Verify search returns only non-deleted records
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<SimpleRecord>>("delete_multiple")
        .await
        .expect("Failed to get Table viewer");

    let all_records = viewer
        .search(|_| true)
        .await
        .expect("Failed to search all records");
    assert_eq!(all_records.len(), 3);

    // Verify correct records remain
    let values: Vec<i32> = all_records.iter().map(|(_, r)| r.value).collect();
    assert!(values.contains(&10));
    assert!(values.contains(&30));
    assert!(values.contains(&50));
}

// ========== Cache-specific tests ==========

#[tokio::test]
async fn test_table_cache_basic_crud() {
    // Test that cache is built and used correctly for basic CRUD operations
    let ctx = TestContext::new().with_database().await;

    // Insert initial records - this should build cache on first read
    let initial_records = vec![
        TestRecord {
            name: "User 1".to_string(),
            age: 25,
            email: "user1@test.com".to_string(),
        },
        TestRecord {
            name: "User 2".to_string(),
            age: 30,
            email: "user2@test.com".to_string(),
        },
    ];
    let keys = create_table_operation(ctx.database(), "cache_test", &initial_records).await;

    // First read - should build cache
    let viewer1 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_test")
        .await
        .expect("Failed to get Table viewer");
    let record1 = viewer1.get(&keys[0]).await.expect("Failed to get record 1");
    assert_eq!(record1.name, "User 1");

    // Second read - should use cache
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_test")
        .await
        .expect("Failed to get Table viewer");
    let record2 = viewer2.get(&keys[1]).await.expect("Failed to get record 2");
    assert_eq!(record2.name, "User 2");

    // Search should also work with cache
    let all_records = viewer2
        .search(|_| true)
        .await
        .expect("Failed to search records");
    assert_eq!(all_records.len(), 2);
}

#[tokio::test]
async fn test_table_cache_invalidation_on_write() {
    // Test that cache is invalidated and rebuilt when new entries are committed
    let ctx = TestContext::new().with_database().await;

    // Insert initial record
    let initial_record = TestRecord {
        name: "Initial User".to_string(),
        age: 25,
        email: "initial@test.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "cache_invalidation",
        std::slice::from_ref(&initial_record),
    )
    .await;

    // First read - builds cache
    let viewer1 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_invalidation")
        .await
        .expect("Failed to get Table viewer");
    let record1 = viewer1
        .get(&keys[0])
        .await
        .expect("Failed to get initial record");
    assert_eq!(record1.name, "Initial User");

    // Add another record - this changes tips, invalidating cache
    let new_record = TestRecord {
        name: "New User".to_string(),
        age: 30,
        email: "new@test.com".to_string(),
    };
    let new_keys = create_table_operation(
        ctx.database(),
        "cache_invalidation",
        std::slice::from_ref(&new_record),
    )
    .await;

    // Read after write - should detect stale cache and rebuild
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_invalidation")
        .await
        .expect("Failed to get Table viewer");

    // Should see both records
    let all_records = viewer2
        .search(|_| true)
        .await
        .expect("Failed to search records");
    assert_eq!(all_records.len(), 2);

    // Verify both records are accessible
    let record_initial = viewer2
        .get(&keys[0])
        .await
        .expect("Failed to get initial record");
    assert_eq!(record_initial.name, "Initial User");

    let record_new = viewer2
        .get(&new_keys[0])
        .await
        .expect("Failed to get new record");
    assert_eq!(record_new.name, "New User");
}

#[tokio::test]
async fn test_table_cache_after_update() {
    // Test that cache is correctly updated after modifying existing records
    let ctx = TestContext::new().with_database().await;

    // Insert initial record
    let initial_record = TestRecord {
        name: "Original Name".to_string(),
        age: 25,
        email: "original@test.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "cache_update",
        std::slice::from_ref(&initial_record),
    )
    .await;

    // First read to build cache
    let viewer1 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_update")
        .await
        .expect("Failed to get Table viewer");
    let record1 = viewer1.get(&keys[0]).await.expect("Failed to get record");
    assert_eq!(record1.name, "Original Name");

    // Update the record
    let op = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op
            .get_store::<Table<TestRecord>>("cache_update")
            .await
            .expect("Failed to get Table");

        let updated_record = TestRecord {
            name: "Updated Name".to_string(),
            age: 30,
            email: "updated@test.com".to_string(),
        };
        table
            .set(&keys[0], updated_record)
            .await
            .expect("Failed to update record");
    }
    op.commit().await.expect("Failed to commit update");

    // Read after update - cache should be invalidated and rebuilt
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_update")
        .await
        .expect("Failed to get Table viewer");
    let record2 = viewer2
        .get(&keys[0])
        .await
        .expect("Failed to get updated record");
    assert_eq!(record2.name, "Updated Name");
    assert_eq!(record2.age, 30);
}

#[tokio::test]
async fn test_table_cache_after_delete() {
    // Test that cache correctly reflects deletions
    let ctx = TestContext::new().with_database().await;

    // Insert initial records
    let initial_records = vec![
        TestRecord {
            name: "User 1".to_string(),
            age: 25,
            email: "user1@test.com".to_string(),
        },
        TestRecord {
            name: "User 2".to_string(),
            age: 30,
            email: "user2@test.com".to_string(),
        },
    ];
    let keys = create_table_operation(ctx.database(), "cache_delete", &initial_records).await;

    // First read to build cache
    let viewer1 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_delete")
        .await
        .expect("Failed to get Table viewer");
    let all1 = viewer1.search(|_| true).await.expect("Failed to search");
    assert_eq!(all1.len(), 2);

    // Delete one record
    let op = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op
            .get_store::<Table<TestRecord>>("cache_delete")
            .await
            .expect("Failed to get Table");
        table
            .delete(&keys[0])
            .await
            .expect("Failed to delete record");
    }
    op.commit().await.expect("Failed to commit deletion");

    // Read after delete - cache should be invalidated and show only 1 record
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_delete")
        .await
        .expect("Failed to get Table viewer");
    let all2 = viewer2.search(|_| true).await.expect("Failed to search");
    assert_eq!(all2.len(), 1);
    assert_eq!(all2[0].1.name, "User 2");

    // Verify deleted record is not found
    assert!(viewer2.get(&keys[0]).await.is_err());
}

#[tokio::test]
async fn test_table_cache_multiple_stores() {
    // Test that caches are independent per store
    let ctx = TestContext::new().with_database().await;

    // Create records in two different stores
    let records_a = vec![TestRecord {
        name: "Store A User".to_string(),
        age: 25,
        email: "storea@test.com".to_string(),
    }];
    let records_b = vec![TestRecord {
        name: "Store B User".to_string(),
        age: 30,
        email: "storeb@test.com".to_string(),
    }];

    create_table_operation(ctx.database(), "cache_store_a", &records_a).await;
    create_table_operation(ctx.database(), "cache_store_b", &records_b).await;

    // Read from both stores
    let viewer_a = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_store_a")
        .await
        .expect("Failed to get Table viewer A");
    let viewer_b = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_store_b")
        .await
        .expect("Failed to get Table viewer B");

    let results_a = viewer_a.search(|_| true).await.expect("Failed to search A");
    let results_b = viewer_b.search(|_| true).await.expect("Failed to search B");

    assert_eq!(results_a.len(), 1);
    assert_eq!(results_b.len(), 1);
    assert_eq!(results_a[0].1.name, "Store A User");
    assert_eq!(results_b[0].1.name, "Store B User");

    // Update store A
    let op = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op
            .get_store::<Table<TestRecord>>("cache_store_a")
            .await
            .expect("Failed to get Table");
        table
            .insert(TestRecord {
                name: "Store A User 2".to_string(),
                age: 35,
                email: "storea2@test.com".to_string(),
            })
            .await
            .expect("Failed to insert");
    }
    op.commit().await.expect("Failed to commit");

    // Verify store A has 2 records, store B still has 1
    let viewer_a2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_store_a")
        .await
        .expect("Failed to get Table viewer A");
    let viewer_b2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("cache_store_b")
        .await
        .expect("Failed to get Table viewer B");

    let results_a2 = viewer_a2
        .search(|_| true)
        .await
        .expect("Failed to search A");
    let results_b2 = viewer_b2
        .search(|_| true)
        .await
        .expect("Failed to search B");

    assert_eq!(results_a2.len(), 2);
    assert_eq!(results_b2.len(), 1);
}

#[tokio::test]
async fn test_table_cache_historical_tips_skips_cache() {
    // Test that transactions at historical tips don't use/update the cache
    let ctx = TestContext::new().with_database().await;

    // Create initial record
    let initial_record = TestRecord {
        name: "Initial User".to_string(),
        age: 25,
        email: "initial@test.com".to_string(),
    };
    let op1 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    let key1 = {
        let table = op1
            .get_store::<Table<TestRecord>>("historical_tips")
            .await
            .expect("Failed to get Table");
        table
            .insert(initial_record.clone())
            .await
            .expect("Failed to insert")
    };
    let entry1_id = op1.commit().await.expect("Failed to commit");

    // Add second record at current tips
    let second_record = TestRecord {
        name: "Second User".to_string(),
        age: 30,
        email: "second@test.com".to_string(),
    };
    let keys2 = create_table_operation(
        ctx.database(),
        "historical_tips",
        std::slice::from_ref(&second_record),
    )
    .await;

    // Read at current tips - should see both records
    let viewer_current = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("historical_tips")
        .await
        .expect("Failed to get viewer");
    let current_results = viewer_current
        .search(|_| true)
        .await
        .expect("Failed to search");
    assert_eq!(current_results.len(), 2);

    // Create transaction at historical tips (before second record)
    let op_historical = ctx
        .database()
        .new_transaction_with_tips([entry1_id])
        .await
        .expect("Failed to start historical transaction");
    {
        let table = op_historical
            .get_store::<Table<TestRecord>>("historical_tips")
            .await
            .expect("Failed to get Table");

        // Should only see the first record (historical view)
        let historical_results = table.search(|_| true).await.expect("Failed to search");
        assert_eq!(historical_results.len(), 1);
        assert_eq!(historical_results[0].1.name, "Initial User");

        // Verify first record is accessible
        let record = table.get(&key1).await.expect("Failed to get record");
        assert_eq!(record.name, "Initial User");

        // Second record should not be visible
        assert!(table.get(&keys2[0]).await.is_err());
    }

    // Current tips view should still see both records
    let viewer_current2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("historical_tips")
        .await
        .expect("Failed to get viewer");
    let current_results2 = viewer_current2
        .search(|_| true)
        .await
        .expect("Failed to search");
    assert_eq!(current_results2.len(), 2);
}

#[tokio::test]
async fn test_table_cache_tombstone_handling() {
    // Test that tombstones (deleted records) are handled correctly in cache
    let ctx = TestContext::new().with_database().await;

    // Create records
    let records = vec![
        TestRecord {
            name: "User 1".to_string(),
            age: 25,
            email: "user1@test.com".to_string(),
        },
        TestRecord {
            name: "User 2".to_string(),
            age: 30,
            email: "user2@test.com".to_string(),
        },
        TestRecord {
            name: "User 3".to_string(),
            age: 35,
            email: "user3@test.com".to_string(),
        },
    ];
    let keys = create_table_operation(ctx.database(), "tombstone_test", &records).await;

    // First read to build cache
    let viewer1 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("tombstone_test")
        .await
        .expect("Failed to get viewer");
    let all1 = viewer1.search(|_| true).await.expect("Failed to search");
    assert_eq!(all1.len(), 3);

    // Delete middle record
    let op = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op
            .get_store::<Table<TestRecord>>("tombstone_test")
            .await
            .expect("Failed to get Table");
        table
            .delete(&keys[1])
            .await
            .expect("Failed to delete record");
    }
    op.commit().await.expect("Failed to commit deletion");

    // Read after delete - cache should be rebuilt with tombstone
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("tombstone_test")
        .await
        .expect("Failed to get viewer");

    // Should only see 2 records
    let all2 = viewer2.search(|_| true).await.expect("Failed to search");
    assert_eq!(all2.len(), 2);

    // Verify correct records remain
    let record1 = viewer2.get(&keys[0]).await.expect("Record 1 should exist");
    assert_eq!(record1.name, "User 1");

    let record3 = viewer2.get(&keys[2]).await.expect("Record 3 should exist");
    assert_eq!(record3.name, "User 3");

    // Deleted record should not be found
    assert!(viewer2.get(&keys[1]).await.is_err());

    // Re-insert at the same key (resurrect)
    let op2 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op2
            .get_store::<Table<TestRecord>>("tombstone_test")
            .await
            .expect("Failed to get Table");
        let resurrected = TestRecord {
            name: "Resurrected User".to_string(),
            age: 99,
            email: "resurrected@test.com".to_string(),
        };
        table
            .set(&keys[1], resurrected)
            .await
            .expect("Failed to resurrect");
    }
    op2.commit().await.expect("Failed to commit resurrection");

    // Should now see 3 records again
    let viewer3 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("tombstone_test")
        .await
        .expect("Failed to get viewer");
    let all3 = viewer3.search(|_| true).await.expect("Failed to search");
    assert_eq!(all3.len(), 3);

    let resurrected = viewer3
        .get(&keys[1])
        .await
        .expect("Resurrected record should exist");
    assert_eq!(resurrected.name, "Resurrected User");
}

#[tokio::test]
async fn test_table_cache_same_uuid_different_stores() {
    // Test that the same UUID in different stores doesn't cause conflicts
    let ctx = TestContext::new().with_database().await;

    // Create a record in store A
    let op1 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    let shared_uuid = {
        let table = op1
            .get_store::<Table<TestRecord>>("store_a")
            .await
            .expect("Failed to get Table");
        table
            .insert(TestRecord {
                name: "Store A Record".to_string(),
                age: 25,
                email: "storea@test.com".to_string(),
            })
            .await
            .expect("Failed to insert")
    };
    op1.commit().await.expect("Failed to commit");

    // Use the same UUID in store B (via set, not insert)
    let op2 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op2
            .get_store::<Table<TestRecord>>("store_b")
            .await
            .expect("Failed to get Table");
        table
            .set(
                &shared_uuid,
                TestRecord {
                    name: "Store B Record".to_string(),
                    age: 30,
                    email: "storeb@test.com".to_string(),
                },
            )
            .await
            .expect("Failed to set");
    }
    op2.commit().await.expect("Failed to commit");

    // Read from both stores - should get different records
    let viewer_a = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("store_a")
        .await
        .expect("Failed to get viewer A");
    let viewer_b = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("store_b")
        .await
        .expect("Failed to get viewer B");

    let record_a = viewer_a
        .get(&shared_uuid)
        .await
        .expect("Failed to get from store A");
    let record_b = viewer_b
        .get(&shared_uuid)
        .await
        .expect("Failed to get from store B");

    assert_eq!(record_a.name, "Store A Record");
    assert_eq!(record_b.name, "Store B Record");
    assert_ne!(record_a, record_b);

    // Modify store A
    let op3 = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    {
        let table = op3
            .get_store::<Table<TestRecord>>("store_a")
            .await
            .expect("Failed to get Table");
        table
            .set(
                &shared_uuid,
                TestRecord {
                    name: "Store A Modified".to_string(),
                    age: 26,
                    email: "storea_mod@test.com".to_string(),
                },
            )
            .await
            .expect("Failed to modify");
    }
    op3.commit().await.expect("Failed to commit");

    // Store A should show modified, Store B should be unchanged
    let viewer_a2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("store_a")
        .await
        .expect("Failed to get viewer A");
    let viewer_b2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("store_b")
        .await
        .expect("Failed to get viewer B");

    let record_a2 = viewer_a2
        .get(&shared_uuid)
        .await
        .expect("Failed to get from store A");
    let record_b2 = viewer_b2
        .get(&shared_uuid)
        .await
        .expect("Failed to get from store B");

    assert_eq!(record_a2.name, "Store A Modified");
    assert_eq!(record_b2.name, "Store B Record"); // Unchanged
}

#[tokio::test]
async fn test_table_cache_concurrent_reads() {
    // Test that concurrent reads work correctly with caching
    let ctx = TestContext::new().with_database().await;

    // Create test data
    let records: Vec<TestRecord> = (0..10)
        .map(|i| TestRecord {
            name: format!("User {}", i),
            age: 20 + i,
            email: format!("user{}@test.com", i),
        })
        .collect();
    let keys = create_table_operation(ctx.database(), "concurrent_reads", &records).await;

    // Spawn multiple concurrent reads
    let db = ctx.database().clone();
    let keys_clone = keys.clone();

    let handles: Vec<_> = (0..5)
        .map(|task_id| {
            let db = db.clone();
            let keys = keys_clone.clone();
            tokio::spawn(async move {
                let viewer = db
                    .get_store_viewer::<Table<TestRecord>>("concurrent_reads")
                    .await
                    .expect("Failed to get viewer");

                // Read all records
                let all = viewer.search(|_| true).await.expect("Failed to search");
                assert_eq!(all.len(), 10, "Task {} saw wrong count", task_id);

                // Read specific records
                for (i, key) in keys.iter().enumerate() {
                    let record = viewer.get(key).await.expect("Failed to get record");
                    assert_eq!(record.name, format!("User {}", i));
                }

                task_id
            })
        })
        .collect();

    // Wait for all tasks
    for handle in handles {
        handle.await.expect("Task panicked");
    }
}

#[tokio::test]
async fn test_table_cache_after_fork_merge() {
    // Test cache behavior after fork and merge scenarios
    let ctx = TestContext::new().with_database().await;

    // Create base record
    let op_base = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start operation");
    let key1 = {
        let table = op_base
            .get_store::<Table<TestRecord>>("fork_merge")
            .await
            .expect("Failed to get Table");
        table
            .insert(TestRecord {
                name: "Base User".to_string(),
                age: 25,
                email: "base@test.com".to_string(),
            })
            .await
            .expect("Failed to insert")
    };
    let base_id = op_base.commit().await.expect("Failed to commit");

    // Build cache at base
    let viewer_base = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("fork_merge")
        .await
        .expect("Failed to get viewer");
    let base_results = viewer_base
        .search(|_| true)
        .await
        .expect("Failed to search");
    assert_eq!(base_results.len(), 1);

    // Fork: Branch A adds a record
    let op_a = ctx
        .database()
        .new_transaction_with_tips([base_id.clone()])
        .await
        .expect("Failed to start branch A");
    let key_a = {
        let table = op_a
            .get_store::<Table<TestRecord>>("fork_merge")
            .await
            .expect("Failed to get Table");
        table
            .insert(TestRecord {
                name: "Branch A User".to_string(),
                age: 30,
                email: "brancha@test.com".to_string(),
            })
            .await
            .expect("Failed to insert")
    };
    let branch_a_id = op_a.commit().await.expect("Failed to commit branch A");

    // Fork: Branch B adds a different record
    let op_b = ctx
        .database()
        .new_transaction_with_tips([base_id])
        .await
        .expect("Failed to start branch B");
    let key_b = {
        let table = op_b
            .get_store::<Table<TestRecord>>("fork_merge")
            .await
            .expect("Failed to get Table");
        table
            .insert(TestRecord {
                name: "Branch B User".to_string(),
                age: 35,
                email: "branchb@test.com".to_string(),
            })
            .await
            .expect("Failed to insert")
    };
    let _branch_b_id = op_b.commit().await.expect("Failed to commit branch B");

    // Now we have a fork - current tips include both branches
    // Read should merge both branches and see all 3 records
    let viewer_merged = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("fork_merge")
        .await
        .expect("Failed to get viewer");
    let merged_results = viewer_merged
        .search(|_| true)
        .await
        .expect("Failed to search");
    assert_eq!(merged_results.len(), 3);

    // Verify all records are accessible
    let base_record = viewer_merged.get(&key1).await.expect("Base record missing");
    assert_eq!(base_record.name, "Base User");

    let a_record = viewer_merged.get(&key_a).await.expect("Branch A record missing");
    assert_eq!(a_record.name, "Branch A User");

    let b_record = viewer_merged.get(&key_b).await.expect("Branch B record missing");
    assert_eq!(b_record.name, "Branch B User");

    // Merge: Create an entry that has both branches as parents
    let op_merge = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start merge operation");
    {
        let table = op_merge
            .get_store::<Table<TestRecord>>("fork_merge")
            .await
            .expect("Failed to get Table");

        // Just read to ensure we can see all records in the merge transaction
        let all = table.search(|_| true).await.expect("Failed to search");
        assert_eq!(all.len(), 3);
    }
    op_merge.commit().await.expect("Failed to commit merge");

    // After merge, should still see all 3 records
    let viewer_after = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("fork_merge")
        .await
        .expect("Failed to get viewer");
    let after_results = viewer_after
        .search(|_| true)
        .await
        .expect("Failed to search");
    assert_eq!(after_results.len(), 3);
}

#[tokio::test]
async fn test_table_cache_empty_store_first_read() {
    // Test cache behavior when reading from an empty store
    let ctx = TestContext::new().with_database().await;

    // Read from non-existent/empty store
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("empty_cache_test")
        .await
        .expect("Failed to get viewer");

    // Should return empty results
    let results = viewer.search(|_| true).await.expect("Failed to search");
    assert_eq!(results.len(), 0);

    // Get should fail for any key
    assert!(viewer.get("some-uuid").await.is_err());

    // Now add a record
    let record = TestRecord {
        name: "First User".to_string(),
        age: 25,
        email: "first@test.com".to_string(),
    };
    let keys = create_table_operation(
        ctx.database(),
        "empty_cache_test",
        std::slice::from_ref(&record),
    )
    .await;

    // Read again - should see the new record (cache rebuilt)
    let viewer2 = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("empty_cache_test")
        .await
        .expect("Failed to get viewer");
    let results2 = viewer2.search(|_| true).await.expect("Failed to search");
    assert_eq!(results2.len(), 1);

    let fetched = viewer2.get(&keys[0]).await.expect("Failed to get record");
    assert_eq!(fetched.name, "First User");
}

#[tokio::test]
async fn test_table_delete_concurrent_modifications() {
    let ctx = TestContext::new().with_database().await;

    // Create base record
    let txn_base = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to start transaction");
    let key1 = {
        let table = txn_base
            .get_store::<Table<TestRecord>>("concurrent_delete")
            .await
            .expect("Failed to get Table");
        let record = TestRecord {
            name: "Base User".to_string(),
            age: 25,
            email: "base@test.com".to_string(),
        };
        table
            .insert(record)
            .await
            .expect("Failed to insert base record")
    };
    let base_entry_id = txn_base.commit().await.expect("Failed to commit base");

    // Branch A: Delete the record
    let op_branch_a = ctx
        .database()
        .new_transaction_with_tips([base_entry_id.clone()])
        .await
        .expect("Failed to start branch A");
    {
        let table = op_branch_a
            .get_store::<Table<TestRecord>>("concurrent_delete")
            .await
            .expect("Failed to get Table");

        table
            .delete(&key1)
            .await
            .expect("Failed to delete in branch A");
    }
    op_branch_a
        .commit()
        .await
        .expect("Failed to commit branch A deletion");

    // Branch B: Update the same record
    let op_branch_b = ctx
        .database()
        .new_transaction_with_tips([base_entry_id])
        .await
        .expect("Failed to start branch B");
    {
        let table = op_branch_b
            .get_store::<Table<TestRecord>>("concurrent_delete")
            .await
            .expect("Failed to get Table");

        let updated_record = TestRecord {
            name: "Updated User".to_string(),
            age: 30,
            email: "updated@test.com".to_string(),
        };
        table
            .set(&key1, updated_record)
            .await
            .expect("Failed to update in branch B");
    }
    op_branch_b
        .commit()
        .await
        .expect("Failed to commit branch B update");

    // Get merged result - CRDT last-write-wins should apply
    // The result depends on CRDT merge semantics
    let viewer = ctx
        .database()
        .get_store_viewer::<Table<TestRecord>>("concurrent_delete")
        .await
        .expect("Failed to get Table viewer");

    // After CRDT merge, one operation will win
    // We just verify the system doesn't crash and produces a deterministic result
    let result = viewer.get(&key1).await;

    // Either the record exists (update won) or doesn't exist (delete won)
    // Both are valid CRDT outcomes depending on timestamp/ID ordering
    match result {
        Ok(record) => {
            // Update won - verify it's the updated record
            assert_eq!(record.name, "Updated User");
        }
        Err(_) => {
            // Delete won - record doesn't exist
            // This is also valid
        }
    }
}

#[tokio::test]
async fn test_table_entry_format_is_row_ops() {
    use eidetica::store::{TableRowOp, RowOpKind};

    let ctx = TestContext::new().with_database().await;

    // Insert a record
    let op = ctx
        .database()
        .new_transaction()
        .await
        .expect("Failed to create transaction");

    let table = op
        .get_store::<Table<TestRecord>>("format_test")
        .await
        .expect("Failed to get Table");

    let key = table
        .insert(TestRecord {
            name: "Test".to_string(),
            age: 25,
            email: "test@test.com".to_string(),
        })
        .await
        .expect("Failed to insert");

    let entry_id = op.commit().await.expect("Failed to commit");

    // Get the raw entry and check format
    let entry = ctx.database().get_entry(&entry_id).await.expect("Failed to get entry");

    let raw_data = entry.data("format_test").expect("No data for subtree");
    println!("Raw entry data: {}", raw_data);

    // Parse as row ops - this should succeed if using new format
    let ops: Vec<TableRowOp> = serde_json::from_str(raw_data)
        .expect("Failed to parse as row ops - NOT using new format!");

    assert_eq!(ops.len(), 1, "Should have exactly one op");
    assert_eq!(ops[0].uuid, key, "Op UUID should match inserted key");
    match &ops[0].kind {
        RowOpKind::Set { data } => {
            let record: TestRecord = serde_json::from_str(data).expect("Failed to parse record");
            assert_eq!(record.name, "Test");
            assert_eq!(record.age, 25);
        }
        RowOpKind::Delete => panic!("Expected Set, got Delete"),
    }

    println!("✓ Entry format is correctly using row-ops!");
}
