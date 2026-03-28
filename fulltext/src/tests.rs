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

use crate::FullTextIndex;

#[test]
fn index_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index
        .index_document("doc1", "the quick brown fox jumps over the lazy dog")
        .unwrap();
    index
        .index_document("doc2", "cats and dogs are popular pets")
        .unwrap();
    index
        .index_document("doc3", "the fox is quick and clever")
        .unwrap();

    let results = index.search("quick fox", 2).unwrap();
    assert_eq!(results.len(), 2);
    // doc1 and doc3 should match "quick fox"
    let ids: Vec<&str> = results.iter().map(|r| r.entity_id.as_str()).collect();
    assert!(ids.contains(&"doc1"));
    assert!(ids.contains(&"doc3"));
}

#[test]
fn persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fts");
    {
        let mut index = FullTextIndex::create(&path).unwrap();
        index.index_document("doc1", "hello world").unwrap();
    }
    let index = FullTextIndex::open(&path).unwrap();
    let results = index.search("hello", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc1");
}

#[test]
fn remove_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();
    index.index_document("doc2", "goodbye world").unwrap();
    assert_eq!(index.count().unwrap(), 2);

    index.remove("doc1").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    let results = index.search("hello", 10).unwrap();
    assert!(results.is_empty(), "Removed document should not appear in search");

    let results = index.search("goodbye", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc2");
}

#[test]
fn update_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "original content").unwrap();

    // Update doc1 with new content
    index.index_document("doc1", "updated content").unwrap();

    // Should still be one document
    assert_eq!(index.count().unwrap(), 1);

    // Should find the updated content
    let results = index.search("updated", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc1");

    // Should NOT find the original content
    let results = index.search("original", 10).unwrap();
    assert!(results.is_empty(), "Old content should be gone after update");
}

#[test]
fn count() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    assert_eq!(index.count().unwrap(), 0);

    index.index_document("doc1", "hello").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    index.index_document("doc2", "world").unwrap();
    assert_eq!(index.count().unwrap(), 2);

    index.remove("doc1").unwrap();
    assert_eq!(index.count().unwrap(), 1);
}

#[test]
fn empty_search() {
    let dir = tempfile::tempdir().unwrap();
    let index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    let results = index.search("anything", 10).unwrap();
    assert!(results.is_empty(), "Search on empty index should return nothing");
}

#[test]
fn search_scores_descending() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index
        .index_document("doc1", "rust rust rust programming language")
        .unwrap();
    index
        .index_document("doc2", "rust is a programming language")
        .unwrap();
    index
        .index_document("doc3", "python is a programming language")
        .unwrap();

    let results = index.search("rust", 3).unwrap();
    // doc1 should score highest (most occurrences of "rust")
    assert!(!results.is_empty());
    assert_eq!(results[0].entity_id, "doc1");

    // Scores should be in descending order
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "Scores should be in descending order"
        );
    }
}
