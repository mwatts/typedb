/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use crate::{VectorIndex, VectorIndexConfig};

#[test]
fn create_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::create(dir.path().join("test.vdb"), 3).unwrap();

    // Insert enough points for HNSW to build a well-connected graph
    index.insert(b"entity1", &[1.0, 0.0, 0.0]).unwrap();
    index.insert(b"entity2", &[0.0, 1.0, 0.0]).unwrap();
    index.insert(b"entity3", &[0.9, 0.1, 0.0]).unwrap();
    index.insert(b"entity4", &[0.0, 0.0, 1.0]).unwrap();
    index.insert(b"entity5", &[0.5, 0.5, 0.0]).unwrap();
    index.insert(b"entity6", &[0.1, 0.9, 0.0]).unwrap();

    let results = index.search(&[1.0, 0.0, 0.0], 2);
    assert_eq!(results.len(), 2);
    // entity1 is exact match, entity3 is closest neighbor
    assert_eq!(results[0].entity_id, b"entity1");
    // Second result should be entity3 (0.9, 0.1, 0.0) — very close to query
    assert_eq!(results[1].entity_id, b"entity3");
}

#[test]
fn persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.vdb");

    {
        let mut index = VectorIndex::create(&path, 3).unwrap();
        index.insert(b"entity1", &[1.0, 0.0, 0.0]).unwrap();
        index.insert(b"entity2", &[0.0, 1.0, 0.0]).unwrap();
    }

    let index = VectorIndex::open(&path).unwrap();
    assert_eq!(index.len(), 2);
    let results = index.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results[0].entity_id, b"entity1");
}

#[test]
fn dimension_mismatch_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::create(dir.path().join("test.vdb"), 3).unwrap();

    let result = index.insert(b"bad", &[1.0, 0.0]);
    assert!(result.is_err(), "Should reject vector with wrong dimension");
}

#[test]
fn remove_entity() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::create(dir.path().join("test.vdb"), 3).unwrap();

    index.insert(b"entity1", &[1.0, 0.0, 0.0]).unwrap();
    index.insert(b"entity2", &[0.0, 1.0, 0.0]).unwrap();
    assert_eq!(index.len(), 2);

    index.remove(b"entity1").unwrap();
    assert_eq!(index.len(), 1);

    // Removal is persisted in redb; next open() will rebuild HNSW without it.
}

#[test]
fn empty_search() {
    let dir = tempfile::tempdir().unwrap();
    let index = VectorIndex::create(dir.path().join("test.vdb"), 3).unwrap();

    let results = index.search(&[1.0, 0.0, 0.0], 5);
    assert!(results.is_empty(), "Search on empty index should return nothing");
}

#[test]
fn search_returns_correct_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::create(dir.path().join("test.vdb"), 3).unwrap();

    // Insert enough vectors to ensure HNSW graph is well-connected.
    index.insert(b"far", &[0.0, 0.0, 1.0]).unwrap(); // orthogonal
    index.insert(b"close", &[0.95, 0.05, 0.0]).unwrap(); // near
    index.insert(b"exact", &[1.0, 0.0, 0.0]).unwrap(); // identical
    index.insert(b"mid1", &[0.5, 0.5, 0.0]).unwrap(); // mid-range
    index.insert(b"mid2", &[0.0, 1.0, 0.0]).unwrap(); // orthogonal in Y

    // Ask for top-2: should always find exact match first, close second.
    let results = index.search(&[1.0, 0.0, 0.0], 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].entity_id, b"exact");
    assert_eq!(results[1].entity_id, b"close");
}

#[test]
fn accessors() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = VectorIndex::create(dir.path().join("test.vdb"), 5).unwrap();

    assert_eq!(index.dimension(), 5);
    assert!(index.is_empty());

    index.insert(b"a", &[1.0, 0.0, 0.0, 0.0, 0.0]).unwrap();
    assert!(!index.is_empty());
    assert_eq!(index.len(), 1);
}

#[test]
fn default_config_matches_legacy_constants() {
    let cfg = VectorIndexConfig::default();
    assert_eq!(cfg.m, 16);
    assert_eq!(cfg.ef_construction, 200);
    assert_eq!(cfg.initial_capacity, 1000);
}

#[test]
fn create_with_custom_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = VectorIndexConfig {
        m: 32,
        ef_construction: 400,
        initial_capacity: 500,
    };
    let mut index =
        VectorIndex::create_with_config(dir.path().join("test.vdb"), 3, config).unwrap();

    assert_eq!(*index.config(), config);

    // Insert and search still work with custom params.
    index.insert(b"a", &[1.0, 0.0, 0.0]).unwrap();
    index.insert(b"b", &[0.9, 0.1, 0.0]).unwrap();
    index.insert(b"c", &[0.0, 1.0, 0.0]).unwrap();
    index.insert(b"d", &[0.0, 0.0, 1.0]).unwrap();
    index.insert(b"e", &[0.5, 0.5, 0.0]).unwrap();

    let results = index.search(&[1.0, 0.0, 0.0], 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].entity_id, b"a");
}

#[test]
fn config_persisted_across_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.vdb");

    let config = VectorIndexConfig {
        m: 8,
        ef_construction: 100,
        initial_capacity: 2000,
    };

    {
        let mut index = VectorIndex::create_with_config(&path, 3, config).unwrap();
        index.insert(b"entity1", &[1.0, 0.0, 0.0]).unwrap();
        index.insert(b"entity2", &[0.0, 1.0, 0.0]).unwrap();
    }

    let reopened = VectorIndex::open(&path).unwrap();
    assert_eq!(*reopened.config(), config);
    assert_eq!(reopened.len(), 2);

    // Search works after reopen with persisted config.
    let results = reopened.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results[0].entity_id, b"entity1");
}
