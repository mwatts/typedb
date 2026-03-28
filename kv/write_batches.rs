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

#[cfg(feature = "rocks")]
pub(crate) struct RocksWriteBatch(pub(crate) rocksdb::WriteBatch);

pub enum KVWriteBatch {
    #[cfg(feature = "rocks")]
    RocksDB(RocksWriteBatch),
    Buffered(BufferedWriteBatch),
}

impl KVWriteBatch {
    pub fn put(&mut self, key: impl AsRef<[u8]>, value: impl AsRef<[u8]>) {
        match self {
            #[cfg(feature = "rocks")]
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

pub struct BufferedWriteBatch {
    pub(crate) ops: Vec<BufferedWriteOp>,
}

pub(crate) enum BufferedWriteOp {
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
