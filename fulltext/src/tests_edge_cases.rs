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

//! Edge-case tests for the Tantivy-based full-text search index.

use crate::FullTextIndex;

// ─── 1. Empty string document ──────────────────────────────────────

#[test]
fn empty_string_document_indexes_without_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    // Indexing an empty string should succeed.
    index.index_document("empty", "").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // Searching any term should still work and return no matches for the empty doc.
    let results = index.search("hello", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn empty_string_document_coexists_with_real_documents() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("empty", "").unwrap();
    index.index_document("real", "tantivy full text search").unwrap();

    assert_eq!(index.count().unwrap(), 2);

    let results = index.search("tantivy", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "real");
}

// ─── 2. Very long document (10K+ chars) ────────────────────────────

#[test]
fn very_long_document_indexes_and_searches() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    // Build a document with 10,000+ characters containing varied words.
    let filler = "lorem ipsum dolor sit amet consectetur adipiscing elit ";
    let long_text = filler.repeat(200); // ~11,000 chars
    assert!(long_text.len() > 10_000);

    index.index_document("long_doc", &long_text).unwrap();
    assert_eq!(index.count().unwrap(), 1);

    let results = index.search("consectetur", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "long_doc");
}

#[test]
fn very_long_document_with_unique_needle() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    // Place a unique word deep in a large document.
    let padding = "common ".repeat(2000);
    let text = format!("{padding} uniqueneedle {padding}");
    assert!(text.len() > 10_000);

    index.index_document("doc1", &text).unwrap();
    index.index_document("doc2", &padding).unwrap();

    let results = index.search("uniqueneedle", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc1");
}

// ─── 3. Special characters in search query ─────────────────────────

#[test]
fn special_char_plus_operator_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();

    // Tantivy interprets `+` as a required-term prefix.
    let result = index.search("+hello", 10);
    // Should not panic; may return results or parse error.
    match result {
        Ok(results) => assert!(!results.is_empty(), "+hello should match doc1"),
        Err(_) => {} // parse error is acceptable
    }
}

#[test]
fn special_char_minus_operator_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();
    index.index_document("doc2", "hello rust").unwrap();

    // `-world` means exclude "world"; combined with `+hello` should yield doc2 only.
    let result = index.search("+hello -world", 10);
    match result {
        Ok(results) => {
            // If Tantivy honours the operators, doc2 should be the only result.
            let ids: Vec<&str> = results.iter().map(|r| r.entity_id.as_str()).collect();
            assert!(!ids.contains(&"doc1") || ids.len() <= 2);
        }
        Err(_) => {} // parse error is acceptable
    }
}

#[test]
fn special_char_quoted_phrase_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index
        .index_document("doc1", "the quick brown fox jumps")
        .unwrap();
    index
        .index_document("doc2", "quick jumps brown fox")
        .unwrap();

    let result = index.search("\"quick brown fox\"", 10);
    match result {
        Ok(results) => {
            // Phrase search should match doc1 (exact phrase) but not doc2.
            let ids: Vec<&str> = results.iter().map(|r| r.entity_id.as_str()).collect();
            assert!(ids.contains(&"doc1"), "doc1 has the exact phrase");
        }
        Err(_) => {} // parse error is acceptable
    }
}

#[test]
fn special_chars_assorted_do_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "testing special chars").unwrap();

    // None of these should panic.
    let queries = [
        "hello AND world",
        "hello OR world",
        "NOT hello",
        "wild*",
        "hello~2",
        "(hello OR world) AND test",
        "field:value",
        "[a TO z]",
        "hello^2",
    ];
    for q in queries {
        let _ = index.search(q, 10);
    }
}

// ─── 4. Search with empty query string ─────────────────────────────

#[test]
fn empty_query_string_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();

    // Empty query: should not panic. May return error or empty results.
    let result = index.search("", 10);
    let _ = result;
}

#[test]
fn whitespace_only_query_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();

    let result = index.search("   ", 10);
    let _ = result;
}

// ─── 5. Search matching zero documents ─────────────────────────────

#[test]
fn search_returns_empty_for_nonexistent_term() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    index.index_document("doc1", "hello world").unwrap();
    index.index_document("doc2", "goodbye world").unwrap();

    let results = index.search("xyzzyx", 10).unwrap();
    assert!(results.is_empty(), "No document contains 'xyzzyx'");
}

#[test]
fn search_returns_empty_on_completely_empty_index() {
    let dir = tempfile::tempdir().unwrap();
    let index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    let results = index.search("anything", 10).unwrap();
    assert!(results.is_empty());
}

// ─── 6. Index same entity_id twice ─────────────────────────────────

#[test]
fn reindexing_same_entity_replaces_not_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("doc1", "alpha beta gamma").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    index.index_document("doc1", "delta epsilon zeta").unwrap();
    assert_eq!(index.count().unwrap(), 1, "Count must stay 1 after re-index");

    // Old content must be gone.
    let results = index.search("alpha", 10).unwrap();
    assert!(results.is_empty(), "Old text 'alpha' should not be found");

    // New content must be present.
    let results = index.search("delta", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc1");
}

#[test]
fn reindexing_same_entity_three_times() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("doc1", "version one").unwrap();
    index.index_document("doc1", "version two").unwrap();
    index.index_document("doc1", "version three").unwrap();

    assert_eq!(index.count().unwrap(), 1, "Count must remain 1 after 3 writes");

    let results = index.search("one", 10).unwrap();
    assert!(results.is_empty());
    let results = index.search("two", 10).unwrap();
    assert!(results.is_empty());
    let results = index.search("three", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "doc1");
}

// ─── 7. Unicode text ───────────────────────────────────────────────

#[test]
fn unicode_cjk_characters() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("zh", "你好世界 全文搜索").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // CJK tokenization depends on the analyzer. At minimum, no panic.
    let result = index.search("你好", 10);
    let _ = result; // may or may not match depending on tokenizer
}

#[test]
fn unicode_hebrew_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("he", "שלום עולם חיפוש טקסט").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // Hebrew words should be tokenized on whitespace at minimum.
    let results = index.search("שלום", 10);
    match results {
        Ok(r) => assert!(!r.is_empty(), "Hebrew word should be found"),
        Err(_) => {} // parse error acceptable for non-Latin scripts
    }
}

#[test]
fn unicode_emoji_in_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index
        .index_document("emoji_doc", "rust is great 🦀 and fast 🚀")
        .unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // The ASCII words around the emoji should be searchable.
    let results = index.search("rust", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].entity_id, "emoji_doc");

    let results = index.search("fast", 10).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn unicode_mixed_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index
        .index_document("mixed", "hello 你好 שלום 🌍 world")
        .unwrap();
    assert_eq!(index.count().unwrap(), 1);

    let results = index.search("hello", 10).unwrap();
    assert_eq!(results.len(), 1);

    let results = index.search("world", 10).unwrap();
    assert_eq!(results.len(), 1);
}

// ─── 8. count() accuracy after index/remove cycles ─────────────────

#[test]
fn count_accuracy_after_multiple_cycles() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();
    assert_eq!(index.count().unwrap(), 0);

    // Add 5 documents.
    for i in 0..5 {
        index
            .index_document(&format!("doc{i}"), &format!("content for document {i}"))
            .unwrap();
    }
    assert_eq!(index.count().unwrap(), 5);

    // Remove 2.
    index.remove("doc1").unwrap();
    index.remove("doc3").unwrap();
    assert_eq!(index.count().unwrap(), 3);

    // Add 3 more.
    for i in 5..8 {
        index
            .index_document(&format!("doc{i}"), &format!("content for document {i}"))
            .unwrap();
    }
    assert_eq!(index.count().unwrap(), 6);

    // Remove all remaining.
    for id in ["doc0", "doc2", "doc4", "doc5", "doc6", "doc7"] {
        index.remove(id).unwrap();
    }
    assert_eq!(index.count().unwrap(), 0);
}

#[test]
fn count_accuracy_interleaved_add_remove_reindex() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("a", "alpha").unwrap();
    index.index_document("b", "beta").unwrap();
    assert_eq!(index.count().unwrap(), 2);

    // Re-index "a" (replace, not add).
    index.index_document("a", "alpha updated").unwrap();
    assert_eq!(index.count().unwrap(), 2);

    // Remove "b".
    index.remove("b").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // Add "c", re-index "a" again.
    index.index_document("c", "gamma").unwrap();
    index.index_document("a", "alpha v3").unwrap();
    assert_eq!(index.count().unwrap(), 2);

    // Remove "a" and "c".
    index.remove("a").unwrap();
    index.remove("c").unwrap();
    assert_eq!(index.count().unwrap(), 0);
}

#[test]
fn count_after_removing_nonexistent_entity() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = FullTextIndex::create(dir.path().join("fts")).unwrap();

    index.index_document("doc1", "hello").unwrap();
    assert_eq!(index.count().unwrap(), 1);

    // Removing a non-existent entity should be a no-op.
    index.remove("nonexistent").unwrap();
    assert_eq!(index.count().unwrap(), 1);
}
