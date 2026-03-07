/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use rocksdb::{DBRawIterator, DB};

use crate::rocks::{
    pool::{LIFOPool, PoolRecycleGuard},
    RocksKVStore,
};

#[derive(Default)]
pub struct RocksRawIteratorPool {
    unprefixed_iterators: LIFOPool<DBRawIterator<'static>>,
    prefixed_iterator: LIFOPool<DBRawIterator<'static>>,
}

impl RocksRawIteratorPool {
    pub fn new() -> Self {
        Self {
            // force never pooling by capping at size 0
            unprefixed_iterators: LIFOPool::new_capped(0),
            prefixed_iterator: LIFOPool::new_capped(0),
        }
    }

    pub(super) fn get_iterator_unprefixed(
        &self,
        rocks_store: &RocksKVStore,
    ) -> PoolRecycleGuard<DBRawIterator<'static>> {
        let iterator = self.unprefixed_iterators.get_or_create(|| {
            // SAFETY: `DBRawIterator` holds a reference to the `DB` it was created from.
            // The rocksdb crate expresses this as a lifetime on `DBRawIterator<'db>`. We
            // transmute to `'static` so the iterator can be stored in the pool independently
            // of the `RocksKVStore` borrow. This is safe as long as:
            //   (1) The `RocksKVStore` (and its `DB`) outlives every iterator obtained from
            //       this pool. This is guaranteed by the ownership hierarchy: the pool is a
            //       field of `RocksKVStore`, so it is dropped before the `DB`.
            //   (2) The pool size cap is currently 0, meaning iterators are never returned
            //       to the pool and are dropped immediately after use, eliminating any
            //       cross-borrow aliasing window. If the cap is raised in future, invariant
            //       (1) must still be verified — in particular the iterator must be evicted
            //       from the pool before the owning `RocksKVStore` is dropped.
            let kv_storage: &'static DB = unsafe { std::mem::transmute(&rocks_store.rocks) };
            kv_storage.raw_iterator_opt(rocks_store.default_read_options())
        });

        // TODO: our rocks bindings dont have refresh()
        // if let Err(err) = iterator.refresh();

        iterator
    }

    pub(super) fn get_iterator_prefixed(&self, rocks_store: &RocksKVStore) -> PoolRecycleGuard<DBRawIterator<'static>> {
        let iterator = self.prefixed_iterator.get_or_create(|| {
            // SAFETY: same invariants as `get_iterator_unprefixed` above.
            let kv_storage: &'static DB = unsafe { std::mem::transmute(&rocks_store.rocks) };
            kv_storage.raw_iterator_opt(rocks_store.bloom_read_options())
        });
        // TODO: our rocks bindings dont have refresh()
        // if let Err(err) = iterator.refresh();

        iterator
    }
}
