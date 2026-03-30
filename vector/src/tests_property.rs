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

use proptest::prelude::*;

use crate::VectorIndex;

/// Strategy for generating a non-zero 3D vector (avoids all-zeros which have
/// undefined cosine distance).
fn vec3_strategy() -> impl Strategy<Value = [f32; 3]> {
    // Generate components in -10..10, then reject the all-zero vector.
    [-10.0f32..10.0, -10.0f32..10.0, -10.0f32..10.0]
        .prop_filter("non-zero vector", |v| v.iter().any(|c| *c != 0.0))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    /// Insert n random 3D vectors and search with limit k.
    /// The number of results must be at most min(k, n).
    #[test]
    fn search_respects_k(
        n in 1usize..50,
        k in 1usize..20,
        seed_vectors in proptest::collection::vec(vec3_strategy(), 1..50),
        query in vec3_strategy(),
    ) {
        // Clamp to the generated collection size.
        let n = n.min(seed_vectors.len());
        let vectors = &seed_vectors[..n];

        let dir = tempfile::tempdir().unwrap();
        let mut index = VectorIndex::create(dir.path().join("prop.vdb"), 3).unwrap();

        for (i, v) in vectors.iter().enumerate() {
            let id = format!("e{i}");
            index.insert(id.as_bytes(), v.as_slice()).unwrap();
        }

        let results = index.search(query.as_slice(), k);
        prop_assert!(
            results.len() <= k.min(n),
            "expected at most {} results, got {}",
            k.min(n),
            results.len()
        );
    }

    /// Search results must be sorted by ascending distance.
    #[test]
    fn results_sorted_by_distance(
        vectors in proptest::collection::vec(vec3_strategy(), 2..50),
        query in vec3_strategy(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut index = VectorIndex::create(dir.path().join("prop.vdb"), 3).unwrap();

        for (i, v) in vectors.iter().enumerate() {
            let id = format!("e{i}");
            index.insert(id.as_bytes(), v.as_slice()).unwrap();
        }

        let k = vectors.len();
        let results = index.search(query.as_slice(), k);

        for window in results.windows(2) {
            prop_assert!(
                window[0].distance <= window[1].distance,
                "results not sorted: distance {} > {}",
                window[0].distance,
                window[1].distance
            );
        }
    }

    /// Inserting a vector and immediately searching for it should return
    /// that vector among the results with distance approximately zero.
    #[test]
    fn insert_then_search_finds_vector(
        padding in proptest::collection::vec(vec3_strategy(), 0..20),
        target in vec3_strategy(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut index = VectorIndex::create(dir.path().join("prop.vdb"), 3).unwrap();

        // Insert some padding vectors first so the graph has multiple nodes.
        for (i, v) in padding.iter().enumerate() {
            let id = format!("pad{i}");
            index.insert(id.as_bytes(), v.as_slice()).unwrap();
        }

        // Insert the target vector.
        index.insert(b"target", target.as_slice()).unwrap();

        let k = padding.len() + 1;
        let results = index.search(target.as_slice(), k);

        let found = results.iter().any(|r| r.entity_id == b"target");
        prop_assert!(found, "target vector not found in search results");

        // The target's distance to itself should be near zero for cosine.
        if let Some(r) = results.iter().find(|r| r.entity_id == b"target") {
            prop_assert!(
                r.distance < 1e-4,
                "self-distance should be ~0, got {}",
                r.distance
            );
        }
    }

    /// After removing a vector and reopening the index (which rebuilds
    /// the HNSW graph), the removed vector must not appear in search results.
    #[test]
    fn remove_then_search_excludes_vector(
        padding in proptest::collection::vec(vec3_strategy(), 1..20),
        target in vec3_strategy(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prop.vdb");

        {
            let mut index = VectorIndex::create(&path, 3).unwrap();

            for (i, v) in padding.iter().enumerate() {
                let id = format!("pad{i}");
                index.insert(id.as_bytes(), v.as_slice()).unwrap();
            }

            index.insert(b"target", target.as_slice()).unwrap();
            index.remove(b"target").unwrap();
        }

        // Reopen — HNSW is rebuilt from persisted vectors (without the removed one).
        let index = VectorIndex::open(&path).unwrap();

        let k = padding.len() + 1;
        let results = index.search(target.as_slice(), k);

        let found = results.iter().any(|r| r.entity_id == b"target");
        prop_assert!(!found, "removed vector should not appear in search results");
    }
}
