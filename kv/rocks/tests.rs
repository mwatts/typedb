/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

//! Unit tests for the RocksDB KV backend.
//!
//! These mirror the in-memory backend tests in `kv/memory/tests.rs` so that
//! both backends can be verified to exhibit the same semantics. Additional
//! tests cover RocksDB-specific paths such as prefix bloom-filter iteration
//! and the `ExcludeFirstWithPrefix` range start.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use bytes::Bytes;
    use lending_iterator::{LendingIterator, Seekable};
    use primitive::key_range::{KeyRange, RangeStart};
    use resource::profile::StorageCounters;

    use crate::{
        keyspaces::{KeyspaceId, KeyspaceSet, Keyspaces},
        write_batches::KVWriteBatch,
        KVBackend,
    };

    // ---------------------------------------------------------------------------
    // Test keyspace definitions
    // ---------------------------------------------------------------------------

    /// Two keyspaces: one without a prefix extractor (general-purpose) and one
    /// with a 2-byte prefix extractor (exercises the bloom-filter code path).
    #[derive(Clone, Copy)]
    enum TestKS {
        NoPrefix,
        WithPrefix,
    }

    impl KeyspaceSet for TestKS {
        fn iter() -> impl Iterator<Item = Self> {
            [Self::NoPrefix, Self::WithPrefix].into_iter()
        }

        fn id(&self) -> KeyspaceId {
            match self {
                Self::NoPrefix => KeyspaceId(0),
                Self::WithPrefix => KeyspaceId(1),
            }
        }

        fn name(&self) -> &'static str {
            match self {
                Self::NoPrefix => "no_prefix",
                Self::WithPrefix => "with_prefix",
            }
        }

        fn prefix_length(&self) -> Option<usize> {
            match self {
                Self::NoPrefix => None,
                Self::WithPrefix => Some(2),
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    fn create_tmp_dir() -> PathBuf {
        use std::time::SystemTime;
        // Use subsec_nanos + thread id for uniqueness across parallel tests.
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().subsec_nanos();
        let thread_id = format!("{:?}", std::thread::current().id());
        let dir = std::env::temp_dir()
            .join(format!("typedb_kv_rocks_{}_{}", nanos, thread_id.replace(['(', ')', ' '], "")));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open_test_keyspaces(dir: &Path) -> Keyspaces {
        KVBackend::RocksDB.open_keyspaces::<TestKS>(dir).expect("open RocksDB keyspaces")
    }

    // ---------------------------------------------------------------------------
    // Tests — basic CRUD (no-prefix keyspace)
    // ---------------------------------------------------------------------------

    #[test]
    fn put_and_get() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        store.put(b"key1", b"value1").unwrap();
        let result = store.get(b"key1", |v| v.to_vec()).unwrap();
        assert_eq!(result, Some(b"value1".to_vec()));
    }

    #[test]
    fn get_missing_key() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        let result = store.get(b"absent", |v| v.to_vec()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn put_overwrite() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        store.put(b"key1", b"v1").unwrap();
        store.put(b"key1", b"v2").unwrap();
        let result = store.get(b"key1", |v| v.to_vec()).unwrap();
        assert_eq!(result, Some(b"v2".to_vec()));
    }

    #[test]
    fn get_prev() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        store.put(b"aaa", b"v1").unwrap();
        store.put(b"bbb", b"v2").unwrap();
        store.put(b"ddd", b"v3").unwrap();

        // Exact match.
        let result = store.get_prev(b"bbb", |k, v| (k.to_vec(), v.to_vec()));
        assert_eq!(result, Some((b"bbb".to_vec(), b"v2".to_vec())));

        // Between keys — should return bbb.
        let result = store.get_prev(b"ccc", |k, v| (k.to_vec(), v.to_vec()));
        assert_eq!(result, Some((b"bbb".to_vec(), b"v2".to_vec())));

        // Before all keys — should return None.
        let result = store.get_prev(b"000", |k, _| k.to_vec());
        assert_eq!(result, None);
    }

    #[test]
    fn write_batch_buffered() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        let mut batch = KVWriteBatch::default(); // Buffered
        batch.put(b"k1", b"v1");
        batch.put(b"k2", b"v2");
        batch.put(b"k3", b"v3");
        store.write(batch).unwrap();

        assert_eq!(store.get(b"k1", |v| v.to_vec()).unwrap(), Some(b"v1".to_vec()));
        assert_eq!(store.get(b"k2", |v| v.to_vec()).unwrap(), Some(b"v2".to_vec()));
        assert_eq!(store.get(b"k3", |v| v.to_vec()).unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn estimate_size_and_count() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        store.put(b"key1", b"value1").unwrap();
        store.put(b"key2", b"value2").unwrap();

        // RocksDB estimates are approximate; just assert non-zero.
        assert!(store.estimate_key_count().unwrap() > 0);
        // estimate_size_in_bytes uses the live-data-size property which may be
        // 0 until a flush; just assert the call succeeds.
        let _ = store.estimate_size_in_bytes().unwrap();
    }

    #[test]
    fn reset_clears_data() {
        let dir = create_tmp_dir();
        let mut ks = KVBackend::RocksDB.open_keyspaces::<TestKS>(&dir).unwrap();

        ks.get(TestKS::NoPrefix.id()).put(b"key1", b"value1").unwrap();
        ks.reset().unwrap();

        assert_eq!(ks.get(TestKS::NoPrefix.id()).get(b"key1", |v| v.to_vec()).unwrap(), None);
    }

    // ---------------------------------------------------------------------------
    // Tests — iteration (no-prefix keyspace)
    // ---------------------------------------------------------------------------

    #[test]
    fn iterate_range_unbounded() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

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
    fn iterate_seek_forward() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        for (k, v) in [(b"a", b"1"), (b"b", b"2"), (b"c", b"3"), (b"d", b"4")] {
            store.put(k, v).unwrap();
        }

        let start: Bytes<'_, 64> = Bytes::copy(b"a");
        let range = KeyRange::new_unbounded(RangeStart::Inclusive(start));
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        // Read first item.
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"a");

        // Seek forward to "c", skipping "b".
        iter.seek(b"c");
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"c");

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"d");

        assert!(iter.next().is_none());
    }

    #[test]
    fn iterate_exclude_first_with_prefix() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::NoPrefix.id());

        store.put(b"aa", b"v1").unwrap();
        store.put(b"ab", b"v2").unwrap();
        store.put(b"ac", b"v3").unwrap();

        // ExcludeFirstWithPrefix skips the exact start key.
        let start: Bytes<'_, 64> = Bytes::copy(b"aa");
        let range = KeyRange::new_unbounded(RangeStart::ExcludeFirstWithPrefix(start));
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        let first = iter.next().unwrap().unwrap();
        assert_eq!(first.0, b"ab", "should skip exact start key 'aa'");
    }

    // ---------------------------------------------------------------------------
    // Tests — prefix bloom-filter keyspace
    // ---------------------------------------------------------------------------

    #[test]
    fn iterate_prefix_bloom_path() {
        let dir = create_tmp_dir();
        let ks = open_test_keyspaces(&dir);
        let store = ks.get(TestKS::WithPrefix.id());

        // 2-byte prefix keyspace: "aa" and "bb" prefixes.
        store.put(b"aa1", b"v1").unwrap();
        store.put(b"aa2", b"v2").unwrap();
        store.put(b"bb1", b"v3").unwrap();
        store.put(b"bb2", b"v4").unwrap();

        // WithinStartAsPrefix on a 2-byte prefix should only return "aa*" keys.
        let prefix: Bytes<'_, 64> = Bytes::copy(b"aa");
        let range = KeyRange::new_within(prefix, false);
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"aa1");
        let item = iter.next().unwrap().unwrap();
        assert_eq!(item.0, b"aa2");
        assert!(iter.next().is_none(), "should not leak into 'bb' prefix");
    }
}
