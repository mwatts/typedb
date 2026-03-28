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

//! HNSW-based vector search index with redb persistence.
//!
//! Provides approximate nearest-neighbor search over entity vectors,
//! persisted to disk via redb and indexed in-memory via hnsw_rs.

use std::path::Path;

use hnsw_rs::prelude::{DistCosine, Hnsw};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

// ─── redb table definitions ────────────────────────────────────────

/// Maps entity_id bytes -> bincode-encoded Vec<f32>.
const VECTORS_TABLE: TableDefinition<'_, &[u8], &[u8]> = TableDefinition::new("vectors");

/// Maps metadata key string -> bincode-encoded value bytes.
const METADATA_TABLE: TableDefinition<'_, &str, &[u8]> = TableDefinition::new("metadata");

// ─── Public types ──────────────────────────────────────────────────

/// Distance metric for vector search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistanceMetric {
    Cosine,
    L2,
    InnerProduct,
}

/// A single search result with the matched entity ID and its distance.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entity_id: Vec<u8>,
    pub distance: f32,
}

/// Error type for vector index operations.
#[derive(Debug)]
pub enum VectorError {
    /// The provided vector has wrong dimensionality.
    DimensionMismatch { expected: usize, got: usize },
    /// redb storage error.
    Storage(redb::Error),
    /// redb database error.
    Database(redb::DatabaseError),
    /// redb table error.
    Table(redb::TableError),
    /// redb transaction error.
    Transaction(redb::TransactionError),
    /// redb commit error.
    Commit(redb::CommitError),
    /// redb storage error (generic).
    StorageError(redb::StorageError),
    /// Serialization/deserialization error.
    Serde(bincode::Error),
    /// Entity not found.
    NotFound(Vec<u8>),
    /// Generic error.
    Other(String),
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::DimensionMismatch { expected, got } => {
                write!(f, "dimension mismatch: expected {expected}, got {got}")
            }
            VectorError::Storage(e) => write!(f, "redb error: {e}"),
            VectorError::Database(e) => write!(f, "redb database error: {e}"),
            VectorError::Table(e) => write!(f, "redb table error: {e}"),
            VectorError::Transaction(e) => write!(f, "redb transaction error: {e}"),
            VectorError::Commit(e) => write!(f, "redb commit error: {e}"),
            VectorError::StorageError(e) => write!(f, "redb storage error: {e}"),
            VectorError::Serde(e) => write!(f, "serialization error: {e}"),
            VectorError::NotFound(id) => write!(f, "entity not found: {:?}", id),
            VectorError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for VectorError {}

impl From<redb::Error> for VectorError {
    fn from(e: redb::Error) -> Self {
        VectorError::Storage(e)
    }
}
impl From<redb::DatabaseError> for VectorError {
    fn from(e: redb::DatabaseError) -> Self {
        VectorError::Database(e)
    }
}
impl From<redb::TableError> for VectorError {
    fn from(e: redb::TableError) -> Self {
        VectorError::Table(e)
    }
}
impl From<redb::TransactionError> for VectorError {
    fn from(e: redb::TransactionError) -> Self {
        VectorError::Transaction(e)
    }
}
impl From<redb::CommitError> for VectorError {
    fn from(e: redb::CommitError) -> Self {
        VectorError::Commit(e)
    }
}
impl From<redb::StorageError> for VectorError {
    fn from(e: redb::StorageError) -> Self {
        VectorError::StorageError(e)
    }
}
impl From<bincode::Error> for VectorError {
    fn from(e: bincode::Error) -> Self {
        VectorError::Serde(e)
    }
}

pub type Result<T> = std::result::Result<T, VectorError>;

// ─── HNSW parameters ──────────────────────────────────────────────

/// Maximum number of neighbors per node in the HNSW graph.
const HNSW_MAX_NB_CONNECTION: usize = 16;

/// Size of the dynamic candidate list during construction.
const HNSW_EF_CONSTRUCTION: usize = 200;

/// Initial capacity hint for the HNSW index.
const HNSW_INITIAL_CAPACITY: usize = 1000;

// ─── VectorIndex ───────────────────────────────────────────────────

/// A persisted HNSW vector index.
///
/// Vectors are stored durably in a redb database. The HNSW graph is
/// rebuilt in memory on `open()` and kept in sync on `insert()`.
pub struct VectorIndex {
    db: Database,
    dimension: usize,
    hnsw: Hnsw<'static, f32, DistCosine>,
    /// Maps internal HNSW data-id (usize) -> external entity_id bytes.
    id_to_entity: Vec<Vec<u8>>,
}

impl VectorIndex {
    /// Create a new vector index at `path` with the given vector dimension.
    pub fn create(path: impl AsRef<Path>, dimension: usize) -> Result<Self> {
        let db = Database::create(path.as_ref()).map_err(redb_database_err)?;

        // Initialize tables and metadata.
        {
            let write_txn = db.begin_write()?;
            {
                // Create tables by opening them.
                let _vectors = write_txn.open_table(VECTORS_TABLE)?;
                let mut meta = write_txn.open_table(METADATA_TABLE)?;
                meta.insert("dimension", bincode::serialize(&dimension)?.as_slice())?;
                meta.insert("count", bincode::serialize(&0usize)?.as_slice())?;
            }
            write_txn.commit()?;
        }

        let hnsw = Hnsw::<f32, DistCosine>::new(
            HNSW_MAX_NB_CONNECTION,
            HNSW_INITIAL_CAPACITY,
            16, // max_layer (auto-selected by hnsw_rs)
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        );

        Ok(Self {
            db,
            dimension,
            hnsw,
            id_to_entity: Vec::new(),
        })
    }

    /// Open an existing vector index, rebuilding the HNSW graph from persisted vectors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::open(path.as_ref()).map_err(redb_database_err)?;

        let (dimension, vectors) = {
            let read_txn = db.begin_read()?;
            let meta = read_txn.open_table(METADATA_TABLE)?;

            let dim_bytes = meta
                .get("dimension")?
                .ok_or_else(|| VectorError::Other("missing 'dimension' metadata".into()))?;
            let dimension: usize = bincode::deserialize(dim_bytes.value())?;

            let vectors_table = read_txn.open_table(VECTORS_TABLE)?;
            let mut vectors: Vec<(Vec<u8>, Vec<f32>)> = Vec::new();

            let iter = vectors_table.iter()?;
            for entry in iter {
                let entry = entry?;
                let entity_id = entry.0.value().to_vec();
                let vector: Vec<f32> = bincode::deserialize(entry.1.value())?;
                vectors.push((entity_id, vector));
            }

            (dimension, vectors)
        };

        let capacity = vectors.len().max(HNSW_INITIAL_CAPACITY);
        let hnsw = Hnsw::<f32, DistCosine>::new(
            HNSW_MAX_NB_CONNECTION,
            capacity,
            16,
            HNSW_EF_CONSTRUCTION,
            DistCosine {},
        );

        let mut id_to_entity: Vec<Vec<u8>> = Vec::with_capacity(vectors.len());

        // Bulk-insert for efficiency: collect references for parallel_insert.
        let data_for_insert: Vec<(&Vec<f32>, usize)> = vectors
            .iter()
            .enumerate()
            .map(|(idx, (_, vec))| (vec, idx))
            .collect();

        if !data_for_insert.is_empty() {
            hnsw.parallel_insert(&data_for_insert);
        }

        for (entity_id, _) in &vectors {
            id_to_entity.push(entity_id.clone());
        }

        Ok(Self {
            db,
            dimension,
            hnsw,
            id_to_entity,
        })
    }

    /// Insert a vector for the given entity ID.
    ///
    /// Persists to redb and inserts into the in-memory HNSW graph.
    pub fn insert(&mut self, entity_id: &[u8], vector: &[f32]) -> Result<()> {
        if vector.len() != self.dimension {
            return Err(VectorError::DimensionMismatch {
                expected: self.dimension,
                got: vector.len(),
            });
        }

        // Persist to redb.
        {
            let write_txn = self.db.begin_write()?;
            {
                let mut vectors = write_txn.open_table(VECTORS_TABLE)?;
                vectors.insert(entity_id, bincode::serialize(vector)?.as_slice())?;

                let mut meta = write_txn.open_table(METADATA_TABLE)?;
                let new_count = self.id_to_entity.len() + 1;
                meta.insert("count", bincode::serialize(&new_count)?.as_slice())?;
            }
            write_txn.commit()?;
        }

        // Insert into HNSW.
        let internal_id = self.id_to_entity.len();
        self.id_to_entity.push(entity_id.to_vec());
        self.hnsw.insert((vector, internal_id));

        Ok(())
    }

    /// Search for the `k` nearest neighbors of `query`.
    ///
    /// Returns results sorted by ascending distance.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<SearchResult> {
        if query.len() != self.dimension || self.id_to_entity.is_empty() {
            return Vec::new();
        }

        let ef_search = k.max(HNSW_EF_CONSTRUCTION);
        let neighbours = self.hnsw.search(query, k, ef_search);

        neighbours
            .iter()
            .map(|n| {
                let internal_id = n.d_id;
                SearchResult {
                    entity_id: self.id_to_entity[internal_id].clone(),
                    distance: n.distance,
                }
            })
            .collect()
    }

    /// Remove a vector by entity ID.
    ///
    /// Removes from redb. The HNSW entry is marked as deleted by
    /// setting a tombstone (hnsw_rs does not support true deletion,
    /// so we rebuild on next open).
    pub fn remove(&mut self, entity_id: &[u8]) -> Result<()> {
        // Find internal ID.
        let internal_id = self
            .id_to_entity
            .iter()
            .position(|id| id.as_slice() == entity_id)
            .ok_or_else(|| VectorError::NotFound(entity_id.to_vec()))?;

        // Remove from redb.
        {
            let write_txn = self.db.begin_write()?;
            {
                let mut vectors = write_txn.open_table(VECTORS_TABLE)?;
                vectors.remove(entity_id)?;

                let mut meta = write_txn.open_table(METADATA_TABLE)?;
                let count = self.id_to_entity.len().saturating_sub(1);
                meta.insert("count", bincode::serialize(&count)?.as_slice())?;
            }
            write_txn.commit()?;
        }

        // Mark as deleted in the id map (set to empty sentinel).
        // The HNSW graph will be rebuilt correctly on next open().
        self.id_to_entity[internal_id] = Vec::new();

        Ok(())
    }

    /// Number of vectors in the index.
    pub fn len(&self) -> usize {
        self.id_to_entity.iter().filter(|id| !id.is_empty()).count()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The vector dimension.
    pub fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Convert a redb::DatabaseError into our VectorError.
fn redb_database_err(e: redb::DatabaseError) -> VectorError {
    VectorError::Database(e)
}
