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

//! Tantivy-based full-text search index.
//!
//! Provides BM25-scored full-text search over entity documents,
//! persisted to disk via tantivy's index storage.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{self, Schema, STORED, STRING, TEXT};
use tantivy::schema::Value;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

#[cfg(test)]
mod tests;

// ─── Public types ──────────────────────────────────────────────────

/// A single full-text search result with the matched entity ID and its BM25 score.
#[derive(Debug, Clone)]
pub struct FtsResult {
    pub entity_id: String,
    pub score: f32,
}

/// Error type for full-text index operations.
#[derive(Debug)]
pub enum FullTextError {
    /// Tantivy error.
    Tantivy(tantivy::TantivyError),
    /// Query parse error.
    QueryParse(tantivy::query::QueryParserError),
    /// IO error.
    Io(std::io::Error),
    /// Generic error.
    Other(String),
}

impl std::fmt::Display for FullTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FullTextError::Tantivy(e) => write!(f, "tantivy error: {e}"),
            FullTextError::QueryParse(e) => write!(f, "query parse error: {e}"),
            FullTextError::Io(e) => write!(f, "IO error: {e}"),
            FullTextError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for FullTextError {}

impl From<tantivy::TantivyError> for FullTextError {
    fn from(e: tantivy::TantivyError) -> Self {
        FullTextError::Tantivy(e)
    }
}

impl From<tantivy::query::QueryParserError> for FullTextError {
    fn from(e: tantivy::query::QueryParserError) -> Self {
        FullTextError::QueryParse(e)
    }
}

impl From<std::io::Error> for FullTextError {
    fn from(e: std::io::Error) -> Self {
        FullTextError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, FullTextError>;

// ─── Writer heap size ──────────────────────────────────────────────

/// Default heap size for the tantivy IndexWriter (50 MB).
const WRITER_HEAP_SIZE: usize = 50_000_000;

// ─── FullTextIndex ─────────────────────────────────────────────────

/// A persisted full-text search index backed by tantivy.
///
/// Each indexed document has two fields:
/// - `entity_id` (STORED + STRING, not tokenized) -- the external entity identifier
/// - `text` (TEXT + STORED, tokenized for full-text search) -- the indexed content
pub struct FullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: IndexWriter,
    entity_id_field: schema::Field,
    text_field: schema::Field,
}

impl FullTextIndex {
    /// Create a new full-text index at `path`.
    ///
    /// Creates the directory if it does not exist.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;

        let mut schema_builder = Schema::builder();
        let entity_id_field = schema_builder.add_text_field("entity_id", STRING | STORED);
        let text_field = schema_builder.add_text_field("text", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_dir(path, schema)?;
        let writer = index.writer(WRITER_HEAP_SIZE)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            writer,
            entity_id_field,
            text_field,
        })
    }

    /// Open an existing full-text index at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let index = Index::open_in_dir(path)?;
        let schema = index.schema();

        let entity_id_field = schema
            .get_field("entity_id")
            .map_err(|_| FullTextError::Other("missing 'entity_id' field in schema".into()))?;
        let text_field = schema
            .get_field("text")
            .map_err(|_| FullTextError::Other("missing 'text' field in schema".into()))?;

        let writer = index.writer(WRITER_HEAP_SIZE)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            index,
            reader,
            writer,
            entity_id_field,
            text_field,
        })
    }

    /// Add or update a document in the index.
    ///
    /// If a document with the same `entity_id` already exists, it is replaced.
    /// Commits immediately after the operation.
    pub fn index_document(&mut self, entity_id: &str, text: &str) -> Result<()> {
        // Delete any existing document with this entity_id.
        let term = Term::from_field_text(self.entity_id_field, entity_id);
        self.writer.delete_term(term);

        // Add the new document.
        let mut doc = TantivyDocument::new();
        doc.add_text(self.entity_id_field, entity_id);
        doc.add_text(self.text_field, text);
        self.writer.add_document(doc)?;

        self.writer.commit()?;
        self.reader.reload()?;

        Ok(())
    }

    /// Search the index using a query string, returning up to `limit` results
    /// ranked by BM25 score (descending).
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<FtsResult>> {
        let query_parser = QueryParser::for_index(&self.index, vec![self.text_field]);
        let query = query_parser.parse_query(query_str)?;
        let searcher = self.reader.searcher();
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_address) in top_docs {
            let retrieved_doc: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(entity_id) = retrieved_doc
                .get_first(self.entity_id_field)
                .and_then(|v: tantivy::schema::document::CompactDocValue<'_>| v.as_str())
            {
                results.push(FtsResult {
                    entity_id: entity_id.to_string(),
                    score,
                });
            }
        }

        Ok(results)
    }

    /// Remove all documents with the given `entity_id`.
    pub fn remove(&mut self, entity_id: &str) -> Result<()> {
        let term = Term::from_field_text(self.entity_id_field, entity_id);
        self.writer.delete_term(term);
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Explicit commit (useful for batch inserts when not using `index_document`).
    pub fn commit(&mut self) -> Result<()> {
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Number of documents in the index.
    pub fn count(&self) -> Result<usize> {
        let searcher = self.reader.searcher();
        Ok(searcher.num_docs() as usize)
    }
}
