/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    iter,
    ops::{Deref, DerefMut},
};

use crate::keyspaces::KEYSPACE_MAXIMUM_COUNT;

/// Wraps a `rocksdb::WriteBatch` in a `pub(crate)` newtype so that
/// `KVWriteBatch::RocksDB` cannot be constructed from outside the `kv` crate.
/// External callers must use `KVWriteBatch::default()` (which yields `Buffered`);
/// the RocksDB backend converts the buffered batch to a native `WriteBatch` on
/// write. This prevents the runtime `unreachable!` in `KVStore::write` that
/// would fire if a RocksDB batch were routed to an in-memory store.
pub(crate) struct RocksWriteBatch(pub(crate) rocksdb::WriteBatch);

pub enum KVWriteBatch {
    /// Only constructible within the `kv` crate (see [`RocksWriteBatch`]).
    RocksDB(RocksWriteBatch),
    Buffered(BufferedWriteBatch),
}

impl KVWriteBatch {
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        match self {
            Self::RocksDB(b) => rocksdb::WriteBatch::put(&mut b.0, key, value),
            Self::Buffered(b) => b.put(key, value),
        }
    }
}

impl Default for KVWriteBatch {
    fn default() -> Self {
        Self::Buffered(BufferedWriteBatch::new())
    }
}

/// A backend-agnostic write batch that buffers operations in memory.
/// Used as the default write batch type, and consumed directly by the
/// in-memory backend. The RocksDB backend converts this to a
/// `rocksdb::WriteBatch` on write.
pub struct BufferedWriteBatch {
    pub(crate) ops: Vec<BufferedWriteOp>,
}

pub(crate) enum BufferedWriteOp {
    // NOTE: There is intentionally no `Delete` variant here. TypeDB's MVCC layer encodes
    // logical deletes as a `Put` with an empty value (a "tombstone"), so KV-level deletes
    // are never issued through a write batch. If a future backend needs true KV-level
    // deletes (e.g. for compaction or non-MVCC paths), a `Delete(Box<[u8]>)` variant and
    // corresponding dispatch in `KVStore::write` will be required.
    Put(Box<[u8]>, Box<[u8]>),
}

impl BufferedWriteBatch {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        self.ops.push(BufferedWriteOp::Put(key.as_ref().into(), value.as_ref().into()));
    }
}

impl Default for BufferedWriteBatch {
    fn default() -> Self {
        Self::new()
    }
}

pub struct WriteBatches {
    pub batches: [Option<KVWriteBatch>; KEYSPACE_MAXIMUM_COUNT],
}

impl IntoIterator for WriteBatches {
    type Item = (usize, KVWriteBatch);
    type IntoIter = iter::FilterMap<
        iter::Enumerate<<[Option<KVWriteBatch>; KEYSPACE_MAXIMUM_COUNT] as IntoIterator>::IntoIter>,
        fn((usize, Option<KVWriteBatch>)) -> Option<(usize, KVWriteBatch)>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.batches.into_iter().enumerate().filter_map(|(index, batch)| Some((index, batch?)))
    }
}

impl Deref for WriteBatches {
    type Target = [Option<KVWriteBatch>];
    fn deref(&self) -> &Self::Target {
        &self.batches
    }
}

impl DerefMut for WriteBatches {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.batches
    }
}

impl Default for WriteBatches {
    fn default() -> Self {
        Self { batches: std::array::from_fn(|_| None) }
    }
}
