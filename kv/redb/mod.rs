/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
pub(crate) mod iterator;
#[cfg(test)]
mod tests;

use std::{
    fs,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::{byte_array::ByteArray, Bytes};
use error::{typedb_error, TypeDBError};
use primitive::key_range::{KeyRange, RangeEnd, RangeStart};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use resource::profile::StorageCounters;

use crate::{
    iterator::ContinueCondition,
    keyspaces::{KeyspaceSet, Keyspaces, KeyspacesError},
    redb::iterator::RedbRangeIterator,
    write_batches::{BufferedWriteBatch, BufferedWriteOp},
    KVStore, KVStoreID,
};

const TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("data");

pub struct RedbKVStore {
    name: &'static str,
    id: KVStoreID,
    path: PathBuf,
    db: Database,
}

impl RedbKVStore {
    pub fn new(name: &'static str, id: KVStoreID, path: PathBuf) -> Result<Self, Box<dyn TypeDBError>> {
        let db = Database::create(&path)
            .map_err(|e| RedbKVError::Open { name, detail: e.to_string() })?;
        // Ensure the table exists by opening a write transaction and creating it.
        {
            let txn = db.begin_write()
                .map_err(|e| RedbKVError::Put { name, detail: e.to_string() })?;
            {
                let _table = txn.open_table(TABLE)
                    .map_err(|e| RedbKVError::Put { name, detail: e.to_string() })?;
            }
            txn.commit().map_err(|e| RedbKVError::Put { name, detail: e.to_string() })?;
        }
        Ok(Self { name, id, path, db })
    }

    pub fn open_keyspaces<KS: KeyspaceSet>(storage_dir: &Path) -> Result<Keyspaces, KeyspacesError> {
        let mut keyspaces = Keyspaces::new();
        for keyspace in KS::iter() {
            keyspaces.validate_new_keyspace(keyspace)?;
            let path = storage_dir.join(format!("{}.redb", keyspace.name()));
            let kv = RedbKVStore::new(keyspace.name(), keyspace.id().into(), path)
                .map_err(|e| KeyspacesError::KVStoreError { typedb_source: e.into() })?;
            keyspaces.keyspaces.push(KVStore::Redb(kv));
            keyspaces.vec_pos_by_id[keyspace.id().0 as usize] = Some(keyspaces.keyspaces.len() - 1);
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
        let txn = self.db.begin_write()
            .map_err(|e| RedbKVError::Put { name: self.name, detail: e.to_string() })?;
        {
            let mut table = txn.open_table(TABLE)
                .map_err(|e| RedbKVError::Put { name: self.name, detail: e.to_string() })?;
            table.insert(key, value)
                .map_err(|e| RedbKVError::Put { name: self.name, detail: e.to_string() })?;
        }
        txn.commit().map_err(|e| RedbKVError::Put { name: self.name, detail: e.to_string() })?;
        Ok(())
    }

    pub fn get<M, V>(&self, key: &[u8], mut mapper: M) -> Result<Option<V>, Box<dyn TypeDBError>>
    where
        M: FnMut(&[u8]) -> V,
    {
        let txn = self.db.begin_read()
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        let table = txn.open_table(TABLE)
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        let result = table.get(key)
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        Ok(result.map(|guard| mapper(guard.value())))
    }

    pub fn get_prev<M, T>(&self, key: &[u8], mut mapper: M) -> Option<T>
    where
        M: FnMut(&[u8], &[u8]) -> T,
    {
        let txn = self.db.begin_read().ok()?;
        let table = txn.open_table(TABLE).ok()?;
        let mut range = table.range::<&[u8]>(..=key).ok()?;
        range.next_back()?.ok().map(|(k, v)| mapper(k.value(), v.value()))
    }

    pub fn iterate_range<const PREFIX_INLINE_SIZE: usize>(
        &self,
        range: &KeyRange<Bytes<'_, PREFIX_INLINE_SIZE>>,
        _storage_counters: StorageCounters,
    ) -> RedbRangeIterator {
        // Snapshot: read all matching entries into a Vec eagerly.
        let (entries, continue_condition, start_pos) = match self.snapshot_range(range) {
            Ok(result) => result,
            Err(_) => {
                // On error, return an empty iterator.
                return RedbRangeIterator::new_from_entries(Vec::new(), ContinueCondition::Always);
            }
        };
        RedbRangeIterator::new_from_entries_at(entries, continue_condition, start_pos)
    }

    fn snapshot_range<const INLINE_BYTES: usize>(
        &self,
        range: &KeyRange<Bytes<'_, INLINE_BYTES>>,
    ) -> Result<(Vec<(Box<[u8]>, Box<[u8]>)>, ContinueCondition, usize), Box<dyn TypeDBError>> {
        let txn = self.db.begin_read()
            .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
        let table = txn.open_table(TABLE)
            .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;

        // Determine the start key.
        let start_key: Vec<u8> = match range.start() {
            RangeStart::Inclusive(bytes) => bytes.as_ref().to_vec(),
            RangeStart::ExcludeFirstWithPrefix(bytes) => bytes.as_ref().to_vec(),
            RangeStart::ExcludePrefix(bytes) => {
                let mut cloned = bytes.to_array();
                cloned.increment().unwrap();
                cloned.as_ref().to_vec()
            }
        };

        // Collect entries matching the range end condition.
        let start_slice = start_key.as_slice();
        let entries: Vec<(Box<[u8]>, Box<[u8]>)> = match range.end() {
            RangeEnd::WithinStartAsPrefix => {
                let prefix: Vec<u8> = range.start().get_value().as_ref().to_vec();
                let iter = table.range::<&[u8]>(start_slice..)
                    .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
                iter.take_while(|r| {
                    r.as_ref().map_or(true, |(k, _)| k.value().starts_with(&prefix))
                })
                .filter_map(|r| r.ok())
                .map(|(k, v)| (k.value().into(), v.value().into()))
                .collect()
            }
            RangeEnd::EndPrefixInclusive(end) => {
                let end_bytes: Vec<u8> = end.as_ref().to_vec();
                let iter = table.range::<&[u8]>(start_slice..)
                    .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
                iter.take_while(|r| {
                    r.as_ref()
                        .map_or(true, |(k, _)| k.value() <= end_bytes.as_slice() || k.value().starts_with(&end_bytes))
                })
                .filter_map(|r| r.ok())
                .map(|(k, v)| (k.value().into(), v.value().into()))
                .collect()
            }
            RangeEnd::EndPrefixExclusive(end) => {
                let end_bytes: Vec<u8> = end.as_ref().to_vec();
                let iter = table.range::<&[u8]>(start_slice..)
                    .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
                iter.take_while(|r| r.as_ref().map_or(true, |(k, _)| k.value() < end_bytes.as_slice()))
                    .filter_map(|r| r.ok())
                    .map(|(k, v)| (k.value().into(), v.value().into()))
                    .collect()
            }
            RangeEnd::Unbounded => {
                let iter = table.range::<&[u8]>(start_slice..)
                    .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
                iter.filter_map(|r| r.ok())
                    .map(|(k, v)| (k.value().into(), v.value().into()))
                    .collect()
            }
        };

        let continue_condition = match range.end() {
            RangeEnd::WithinStartAsPrefix => {
                ContinueCondition::ExactPrefix(ByteArray::from(range.start().get_value().as_ref()))
            }
            RangeEnd::EndPrefixInclusive(end) => ContinueCondition::EndPrefixInclusive(ByteArray::from(end.as_ref())),
            RangeEnd::EndPrefixExclusive(end) => ContinueCondition::EndPrefixExclusive(ByteArray::from(end.as_ref())),
            RangeEnd::Unbounded => ContinueCondition::Always,
        };

        // If start is ExcludeFirstWithPrefix, skip the exact start key.
        let mut start_pos = 0;
        if matches!(range.start(), RangeStart::ExcludeFirstWithPrefix(_)) {
            let start_value = range.start().get_value().as_ref();
            if entries.first().is_some_and(|(k, _)| k.as_ref() == start_value) {
                start_pos = 1;
            }
        }

        Ok((entries, continue_condition, start_pos))
    }

    pub fn write(&self, write_batch: BufferedWriteBatch) -> Result<(), Box<dyn TypeDBError>> {
        let txn = self.db.begin_write()
            .map_err(|e| RedbKVError::BatchWrite { name: self.name, detail: e.to_string() })?;
        {
            let mut table = txn.open_table(TABLE)
                .map_err(|e| RedbKVError::BatchWrite { name: self.name, detail: e.to_string() })?;
            for op in write_batch.ops {
                match op {
                    BufferedWriteOp::Put(key, value) => {
                        table.insert(key.as_ref(), value.as_ref())
                            .map_err(|e| RedbKVError::BatchWrite { name: self.name, detail: e.to_string() })?;
                    }
                }
            }
        }
        txn.commit().map_err(|e| RedbKVError::BatchWrite { name: self.name, detail: e.to_string() })?;
        Ok(())
    }

    pub fn checkpoint(&self, checkpoint_dir: &Path) -> Result<(), Box<dyn TypeDBError>> {
        // Copy the .redb file to the checkpoint directory, preserving the keyspace name
        // as a subdirectory (matching RocksDB's convention — restore_storage_from_checkpoint
        // expects checkpoint_dir/<keyspace_name>/ to be a directory it can read_dir).
        let keyspace_checkpoint_dir = checkpoint_dir.join(self.name);
        if keyspace_checkpoint_dir.exists() {
            return Err(RedbKVError::Checkpoint {
                name: self.name,
                detail: format!("checkpoint directory already exists: {}", keyspace_checkpoint_dir.display()),
            }.into());
        }
        fs::create_dir_all(&keyspace_checkpoint_dir)
            .map_err(|e| RedbKVError::Checkpoint { name: self.name, detail: e.to_string() })?;

        // redb is ACID — the file is always in a consistent state, so a direct
        // copy is safe without explicit compaction.
        // Copy the .redb file into the checkpoint keyspace directory
        let dest = keyspace_checkpoint_dir.join(self.path.file_name().unwrap());
        fs::copy(&self.path, &dest)
            .map_err(|e| RedbKVError::Checkpoint { name: self.name, detail: e.to_string() })?;
        Ok(())
    }

    pub fn delete(self) -> Result<(), Box<dyn TypeDBError>> {
        let path = self.path.clone();
        let name = self.name;
        drop(self.db);
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| RedbKVError::DeleteErrorFileRemove { name, source: Arc::new(e) })?;
        }
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), Box<dyn TypeDBError>> {
        let txn = self.db.begin_write()
            .map_err(|e| RedbKVError::Reset { name: self.name, detail: e.to_string() })?;
        {
            // Delete and recreate the table to clear all data.
            txn.delete_table(TABLE)
                .map_err(|e| RedbKVError::Reset { name: self.name, detail: e.to_string() })?;
            let _table = txn.open_table(TABLE)
                .map_err(|e| RedbKVError::Reset { name: self.name, detail: e.to_string() })?;
        }
        txn.commit().map_err(|e| RedbKVError::Reset { name: self.name, detail: e.to_string() })?;
        Ok(())
    }

    pub fn estimate_size_in_bytes(&self) -> Result<u64, Box<dyn TypeDBError>> {
        let txn = self.db.begin_read()
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        let table = txn.open_table(TABLE)
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        // Sum up key + value lengths. This is an estimate since redb has internal overhead.
        let mut size: u64 = 0;
        let iter = table.iter()
            .map_err(|e| RedbKVError::Iterate { name: self.name, detail: e.to_string() })?;
        for entry in iter {
            if let Ok((k, v)) = entry {
                size += k.value().len() as u64 + v.value().len() as u64;
            }
        }
        Ok(size)
    }

    pub fn estimate_key_count(&self) -> Result<u64, Box<dyn TypeDBError>> {
        let txn = self.db.begin_read()
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        let table = txn.open_table(TABLE)
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        let count = table.len()
            .map_err(|e| RedbKVError::Get { name: self.name, detail: e.to_string() })?;
        Ok(count)
    }
}

impl std::fmt::Debug for RedbKVStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbKVStore")
            .field("path", &self.path)
            .field("name", &self.name)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

typedb_error! {
    pub RedbKVError(component = "Redb error", prefix = "RDB") {
        Open(1, "Redb error opening kv store {name}: {detail}.", name: &'static str, detail: String),
        Get(2, "Redb error getting key in kv store {name}: {detail}.", name: &'static str, detail: String),
        Put(3, "Redb error putting key in kv store {name}: {detail}.", name: &'static str, detail: String),
        BatchWrite(4, "Redb error writing batch to kv store {name}: {detail}.", name: &'static str, detail: String),
        Iterate(5, "Redb error iterating kv store {name}: {detail}.", name: &'static str, detail: String),
        Reset(6, "Redb error resetting kv store {name}: {detail}.", name: &'static str, detail: String),
        Checkpoint(7, "Redb checkpoint error for kv store {name}: {detail}.", name: &'static str, detail: String),
        DeleteErrorFileRemove(30, "Failed to delete file of kv store {name}.", name: &'static str, source: Arc<io::Error>),
    }
}

impl std::fmt::Display for RedbKVError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use error::TypeDBError;
        write!(f, "{}", self.format_code_and_description())
    }
}

impl std::error::Error for RedbKVError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}
