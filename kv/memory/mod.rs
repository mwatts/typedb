/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
pub(crate) mod iterator;
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, RwLock},
};

use bytes::Bytes;
use error::TypeDBError;
use primitive::key_range::KeyRange;
use resource::profile::StorageCounters;

use crate::{
    keyspaces::{KeyspaceId, KeyspaceSet, Keyspaces, KeyspacesError},
    memory::iterator::InMemoryRangeIterator,
    write_batches::{BufferedWriteBatch, BufferedWriteOp},
    KVStore, KVStoreID,
};

#[derive(Debug)]
pub struct InMemoryKVStore {
    name: &'static str,
    id: KVStoreID,
    pub(crate) data: Arc<RwLock<BTreeMap<Box<[u8]>, Box<[u8]>>>>,
}

impl InMemoryKVStore {
    pub fn new(name: &'static str, id: KVStoreID) -> Self {
        Self { name, id, data: Arc::new(RwLock::new(BTreeMap::new())) }
    }

    pub fn open_keyspaces<KS: KeyspaceSet>(_storage_dir: &Path) -> Result<Keyspaces, KeyspacesError> {
        let mut keyspaces = Keyspaces::new();
        for keyspace in KS::iter() {
            keyspaces.validate_new_keyspace(keyspace)?;
            let kv = InMemoryKVStore::new(keyspace.name(), keyspace.id().into());
            keyspaces.keyspaces.push(KVStore::InMemory(kv));
            keyspaces.index[keyspace.id().0 as usize] = Some(KeyspaceId(keyspaces.keyspaces.len() as u8 - 1));
        }
        Ok(keyspaces)
    }

    pub fn id(&self) -> KVStoreID {
        self.id
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn TypeDBError>> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        data.insert(key.into(), value.into());
        Ok(())
    }

    pub fn get<M, V>(&self, key: &[u8], mut mapper: M) -> Result<Option<V>, Box<dyn TypeDBError>>
    where
        M: FnMut(&[u8]) -> V,
    {
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        Ok(data.get(key).map(|v| mapper(v)))
    }

    pub fn get_prev<M, T>(&self, key: &[u8], mut mapper: M) -> Option<T>
    where
        M: FnMut(&[u8], &[u8]) -> T,
    {
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        use std::ops::Bound;
        let key_boxed: Box<[u8]> = key.into();
        data.range::<Box<[u8]>, _>((Bound::Unbounded, Bound::Included(key_boxed)))
            .rev()
            .next()
            .map(|(k, v)| mapper(k, v))
    }

    pub fn iterate_range<const PREFIX_INLINE_SIZE: usize>(
        &self,
        range: &KeyRange<Bytes<'_, PREFIX_INLINE_SIZE>>,
        _storage_counters: StorageCounters,
    ) -> InMemoryRangeIterator {
        InMemoryRangeIterator::new(&self.data, range)
    }

    pub fn write(&self, write_batch: BufferedWriteBatch) -> Result<(), Box<dyn TypeDBError>> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        for op in write_batch.ops {
            match op {
                BufferedWriteOp::Put(key, value) => {
                    data.insert(key, value);
                }
            }
        }
        Ok(())
    }

    pub fn checkpoint(&self, _checkpoint_dir: &Path) -> Result<(), Box<dyn TypeDBError>> {
        Ok(())
    }

    pub fn delete(self) -> Result<(), Box<dyn TypeDBError>> {
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), Box<dyn TypeDBError>> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        data.clear();
        Ok(())
    }

    pub fn estimate_size_in_bytes(&self) -> Result<u64, Box<dyn TypeDBError>> {
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        let size: usize = data.iter().map(|(k, v)| k.len() + v.len()).sum();
        Ok(size as u64)
    }

    pub fn estimate_key_count(&self) -> Result<u64, Box<dyn TypeDBError>> {
        let data = self.data.read().unwrap_or_else(|e| e.into_inner());
        Ok(data.len() as u64)
    }
}
