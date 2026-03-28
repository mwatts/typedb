/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{collections::HashMap, error::Error, fmt, path::PathBuf};

use resource::internal_database_prefix;
use serde::{de::DeserializeOwned, Serialize};
use tracing::{event, Level};
use uuid::Uuid;

pub const CACHE_DB_NAME_PREFIX: &str = concat!(internal_database_prefix!(), "cache-");

/// Disk storage backend for SpilloverCache.
#[derive(Debug)]
enum DiskStorage {
    #[cfg(feature = "rocks")]
    RocksDB(rocksdb::DB),
    #[cfg(feature = "redb")]
    Redb(redb::Database),
    /// No disk backend — spillover goes to memory (unbounded).
    None,
}

#[cfg(feature = "redb")]
const REDB_CACHE_TABLE: redb::TableDefinition<'_, &[u8], &[u8]> = redb::TableDefinition::new("cache");

// A single-threaded configurable cache which prioritizes using a simple in-memory storage, but
// spills the excessive data not fitting into the memory requirements over to disk.
#[derive(Debug)]
pub struct SpilloverCache<T: Serialize + DeserializeOwned + Clone> {
    memory_storage: HashMap<String, T>,
    disk_storage_path: PathBuf,
    disk_storage: DiskStorage,
    memory_size_limit: usize,
}

impl<T: Serialize + DeserializeOwned + Clone> SpilloverCache<T> {
    pub fn new(disk_storage_dir: &PathBuf, name_prefix: Option<&str>, memory_size_limit: usize) -> Self {
        let unique_db_name = Uuid::new_v4().to_string();
        let disk_storage_path =
            disk_storage_dir.join(format!("{}{}{}", CACHE_DB_NAME_PREFIX, name_prefix.unwrap_or(""), unique_db_name));

        SpilloverCache {
            memory_storage: HashMap::new(),
            disk_storage_path,
            disk_storage: DiskStorage::None,
            memory_size_limit,
        }
    }

    pub fn insert(&mut self, key: String, value: T) -> Result<(), CacheError> {
        self.remove(&key)?;
        match self.memory_storage.len() < self.memory_size_limit {
            true => {
                self.memory_storage.insert(key, value);
                Ok(())
            }
            false => self.disk_storage_insert(key, value),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<T>, CacheError> {
        match self.memory_storage.get(key).cloned() {
            Some(value) => Ok(Some(value)),
            None => self.disk_storage_get(key),
        }
    }

    pub fn remove(&mut self, key: &str) -> Result<(), CacheError> {
        match self.memory_storage.remove(key) {
            Some(_) => Ok(()),
            None => self.disk_storage_remove(key),
        }
    }

    fn disk_storage_insert(&mut self, key: String, value: T) -> Result<(), CacheError> {
        self.ensure_disk_storage()?;
        let serialized = bincode::serialize(&value).map_err(|_| CacheError::DiskStorageSerialization {})?;

        match &self.disk_storage {
            #[cfg(feature = "rocks")]
            DiskStorage::RocksDB(db) => {
                db.put(&key, &serialized)
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })
            }
            #[cfg(feature = "redb")]
            DiskStorage::Redb(db) => {
                let txn = db.begin_write()
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                {
                    let mut table = txn.open_table(REDB_CACHE_TABLE)
                        .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                    table.insert(key.as_bytes(), serialized.as_slice())
                        .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                }
                txn.commit()
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })
            }
            DiskStorage::None => {
                // No disk backend — store in memory regardless of limit
                self.memory_storage.insert(key, value);
                Ok(())
            }
        }
    }

    fn disk_storage_get(&self, key: &str) -> Result<Option<T>, CacheError> {
        match &self.disk_storage {
            #[cfg(feature = "rocks")]
            DiskStorage::RocksDB(db) => {
                if let Some(bytes) = db.get(key).map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })? {
                    return bincode::deserialize(&bytes)
                        .map(Some)
                        .map_err(|_| CacheError::DiskStorageDeserialization {});
                }
                Ok(None)
            }
            #[cfg(feature = "redb")]
            DiskStorage::Redb(db) => {
                let txn = db.begin_read()
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                let table = match txn.open_table(REDB_CACHE_TABLE) {
                    Ok(table) => table,
                    Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
                    Err(e) => return Err(CacheError::DiskStorageAccess { source: e.to_string() }),
                };
                match table.get(key.as_bytes()) {
                    Ok(Some(value)) => {
                        bincode::deserialize(value.value())
                            .map(Some)
                            .map_err(|_| CacheError::DiskStorageDeserialization {})
                    }
                    Ok(None) => Ok(None),
                    Err(e) => Err(CacheError::DiskStorageAccess { source: e.to_string() }),
                }
            }
            DiskStorage::None => Ok(None),
        }
    }

    fn disk_storage_remove(&mut self, key: &str) -> Result<(), CacheError> {
        match &self.disk_storage {
            #[cfg(feature = "rocks")]
            DiskStorage::RocksDB(db) => {
                db.delete(key).map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })
            }
            #[cfg(feature = "redb")]
            DiskStorage::Redb(db) => {
                let txn = db.begin_write()
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                {
                    let mut table = txn.open_table(REDB_CACHE_TABLE)
                        .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                    table.remove(key.as_bytes())
                        .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
                }
                txn.commit()
                    .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })
            }
            DiskStorage::None => Ok(()),
        }
    }

    /// Lazily initialize disk storage on first spillover.
    fn ensure_disk_storage(&mut self) -> Result<(), CacheError> {
        if !matches!(self.disk_storage, DiskStorage::None) {
            return Ok(());
        }

        // Prefer rocks if available, then redb, then fall back to memory-only
        #[cfg(feature = "rocks")]
        {
            let mut options = rocksdb::Options::default();
            options.create_if_missing(true);
            let db = rocksdb::DB::open(&options, &self.disk_storage_path)
                .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
            self.disk_storage = DiskStorage::RocksDB(db);
            return Ok(());
        }

        #[cfg(feature = "redb")]
        {
            let db = redb::Database::create(&self.disk_storage_path)
                .map_err(|e| CacheError::DiskStorageAccess { source: e.to_string() })?;
            self.disk_storage = DiskStorage::Redb(db);
            return Ok(());
        }

        // No disk backend available — spillover stays in memory
        #[allow(unreachable_code)]
        Ok(())
    }
}

impl<T: Serialize + DeserializeOwned + Clone> Drop for SpilloverCache<T> {
    fn drop(&mut self) {
        let has_disk = !matches!(self.disk_storage, DiskStorage::None);
        self.disk_storage = DiskStorage::None; // drop the DB handle first
        if has_disk {
            if let Err(e) = std::fs::remove_dir_all(&self.disk_storage_path) {
                event!(Level::TRACE, "Failed to delete a temporary DB directory {:?}: {e}", self.disk_storage_path);
            }
            // redb uses a single file, not a directory
            if self.disk_storage_path.is_file() {
                let _ = std::fs::remove_file(&self.disk_storage_path);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum CacheError {
    DiskStorageAccess { source: String },
    DiskStorageSerialization,
    DiskStorageDeserialization,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::DiskStorageAccess { source } => write!(f, "Cannot access disk storage, {source}"),
            CacheError::DiskStorageSerialization => write!(f, "Internal error: cannot write data to the disk storage"),
            CacheError::DiskStorageDeserialization => {
                write!(f, "Internal error: disk storage is corrupted and data cannot be read")
            }
        }
    }
}

impl Error for CacheError {}

#[cfg(test)]
pub mod tests {
    use test_utils::{create_tmp_dir, TempDir};

    use crate::SpilloverCache;
    macro_rules! put {
        ($cache:ident, $key:literal, $value:literal) => {
            $cache.insert($key.to_owned(), $value.to_owned()).unwrap()
        };
    }
    macro_rules! get {
        ($cache:ident, $key:literal) => {
            $cache.get($key).unwrap().as_ref().map(String::as_str)
        };
    }

    fn create_cache_in_tmpdir() -> (TempDir, SpilloverCache<String>) {
        let tmp_dir = create_tmp_dir();
        let cache: SpilloverCache<String> = SpilloverCache::new(&tmp_dir.as_ref().to_path_buf(), Some("unit_test"), 1);
        (tmp_dir, cache)
    }

    #[test]
    fn test_insert_spillover_duplicates() {
        let (_tmp_dir, mut cache) = create_cache_in_tmpdir();
        put!(cache, "key1", "value1");
        assert_eq!(get!(cache, "key1"), Some("value1"));
        put!(cache, "key1", "value2");
        assert_eq!(get!(cache, "key1"), Some("value2"));
    }

    #[test]
    fn test_delete_insert_duplicates() {
        let (_tmp_dir, mut cache) = create_cache_in_tmpdir();
        put!(cache, "key1", "value1");
        assert_eq!(get!(cache, "key1"), Some("value1"));

        put!(cache, "key2", "value2_1");
        assert_eq!(cache.get("key2").unwrap().unwrap(), "value2_1");

        cache.remove("key1").unwrap();
        assert_eq!(get!(cache, "key1"), None);

        put!(cache, "key2", "value2_2");
        assert_eq!(get!(cache, "key2"), Some("value2_2"));

        cache.remove("key2").unwrap();
        assert_eq!(get!(cache, "key2"), None);
    }
}
