/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::sync::Arc;

use durability::wal::WAL;
use encoding::EncodingKeyspace;
use storage::{durability_client::WALClient, MVCCStorage};
use test_utils::{create_tmp_dir, init_logging, TempDir};

fn test_backend() -> kv::KVBackend {
    #[cfg(feature = "test-redb")]
    { kv::KVBackend::Redb }
    #[cfg(not(feature = "test-redb"))]
    { kv::KVBackend::RocksDB }
}

pub fn create_core_storage() -> (TempDir, Arc<MVCCStorage<WALClient>>) {
    init_logging();
    let storage_path = create_tmp_dir();
    let wal = WAL::create(&storage_path).unwrap();
    let storage = Arc::new(
        MVCCStorage::create_with_backend::<EncodingKeyspace>(
            "db_storage", &storage_path, WALClient::new(wal), test_backend()
        ).unwrap()
    );
    (storage_path, storage)
}
