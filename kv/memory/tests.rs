/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use lending_iterator::{LendingIterator, Seekable};
    use primitive::key_range::{KeyRange, RangeStart};
    use resource::profile::StorageCounters;

    use crate::{
        memory::InMemoryKVStore,
        write_batches::{BufferedWriteBatch, KVWriteBatch},
        KVStore,
    };

    fn create_store() -> InMemoryKVStore {
        InMemoryKVStore::new("test", 0)
    }

    #[test]
    fn put_and_get() {
        let store = create_store();
        store.put(b"key1", b"value1").unwrap();
        let result = store.get(b"key1", |v| v.to_vec()).unwrap();
        assert_eq!(result, Some(b"value1".to_vec()));
    }

    #[test]
    fn get_missing_key() {
        let store = create_store();
        let result = store.get(b"key1", |v| v.to_vec()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn put_overwrite() {
        let store = create_store();
        store.put(b"key1", b"value1").unwrap();
        store.put(b"key1", b"value2").unwrap();
        let result = store.get(b"key1", |v| v.to_vec()).unwrap();
        assert_eq!(result, Some(b"value2".to_vec()));
    }

    #[test]
    fn get_prev() {
        let store = create_store();
        store.put(b"aaa", b"v1").unwrap();
        store.put(b"bbb", b"v2").unwrap();
        store.put(b"ddd", b"v3").unwrap();

        // Exact match
        let result = store.get_prev(b"bbb", |k, v| (k.to_vec(), v.to_vec()));
        assert_eq!(result, Some((b"bbb".to_vec(), b"v2".to_vec())));

        // Between keys — should find bbb
        let result = store.get_prev(b"ccc", |k, v| (k.to_vec(), v.to_vec()));
        assert_eq!(result, Some((b"bbb".to_vec(), b"v2".to_vec())));

        // Before all keys
        let result = store.get_prev(b"000", |k, _v| k.to_vec());
        assert_eq!(result, None);
    }

    #[test]
    fn write_batch() {
        let store = create_store();
        let mut batch = BufferedWriteBatch::new();
        batch.put(b"key1", b"val1");
        batch.put(b"key2", b"val2");
        batch.put(b"key3", b"val3");
        store.write(batch).unwrap();

        assert_eq!(store.get(b"key1", |v| v.to_vec()).unwrap(), Some(b"val1".to_vec()));
        assert_eq!(store.get(b"key2", |v| v.to_vec()).unwrap(), Some(b"val2".to_vec()));
        assert_eq!(store.get(b"key3", |v| v.to_vec()).unwrap(), Some(b"val3".to_vec()));
    }

    #[test]
    fn estimate_size_and_count() {
        let store = create_store();
        assert_eq!(store.estimate_key_count().unwrap(), 0);
        assert_eq!(store.estimate_size_in_bytes().unwrap(), 0);

        store.put(b"key1", b"value1").unwrap();
        store.put(b"key2", b"value2").unwrap();

        assert_eq!(store.estimate_key_count().unwrap(), 2);
        // 4 + 6 + 4 + 6 = 20
        assert_eq!(store.estimate_size_in_bytes().unwrap(), 20);
    }

    #[test]
    fn reset_clears_data() {
        let mut store = create_store();
        store.put(b"key1", b"value1").unwrap();
        store.reset().unwrap();
        assert_eq!(store.get(b"key1", |v| v.to_vec()).unwrap(), None);
        assert_eq!(store.estimate_key_count().unwrap(), 0);
    }

    #[test]
    fn iterate_range_prefix() {
        let store = create_store();
        store.put(b"aa1", b"v1").unwrap();
        store.put(b"aa2", b"v2").unwrap();
        store.put(b"ab1", b"v3").unwrap();
        store.put(b"bb1", b"v4").unwrap();

        let prefix: Bytes<'_, 64> = Bytes::copy(b"aa");
        let range = KeyRange::new_within(prefix, false);
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"aa1");
        assert_eq!(item.1, b"v1");

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"aa2");
        assert_eq!(item.1, b"v2");

        assert!(iter.next().is_none());
    }

    #[test]
    fn iterate_range_unbounded() {
        let store = create_store();
        store.put(b"a", b"1").unwrap();
        store.put(b"b", b"2").unwrap();
        store.put(b"c", b"3").unwrap();

        let start: Bytes<'_, 64> = Bytes::copy(b"a");
        let range = KeyRange::new_unbounded(RangeStart::Inclusive(start));
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        let mut count = 0;
        while iter.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn iterate_seek() {
        let store = create_store();
        store.put(b"a", b"1").unwrap();
        store.put(b"b", b"2").unwrap();
        store.put(b"c", b"3").unwrap();
        store.put(b"d", b"4").unwrap();

        let start: Bytes<'_, 64> = Bytes::copy(b"a");
        let range = KeyRange::new_unbounded(RangeStart::Inclusive(start));
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        // Read first item
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"a");

        // Seek forward to "c"
        iter.seek(b"c");
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"c");

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"d");

        assert!(iter.next().is_none());
    }

    #[test]
    fn kvstore_enum_dispatch() {
        let kv = KVStore::InMemory(create_store());
        kv.put(b"hello", b"world").unwrap();
        let result = kv.get(b"hello", |v| v.to_vec()).unwrap();
        assert_eq!(result, Some(b"world".to_vec()));
        assert_eq!(kv.name(), "test");
        assert_eq!(kv.estimate_key_count().unwrap(), 1);
    }

    #[test]
    fn kvstore_write_with_buffered_batch() {
        let kv = KVStore::InMemory(create_store());
        let mut batch = KVWriteBatch::default(); // Creates Buffered
        batch.put(b"k1", b"v1");
        batch.put(b"k2", b"v2");
        kv.write(batch).unwrap();

        assert_eq!(kv.get(b"k1", |v| v.to_vec()).unwrap(), Some(b"v1".to_vec()));
        assert_eq!(kv.get(b"k2", |v| v.to_vec()).unwrap(), Some(b"v2".to_vec()));
    }
}
