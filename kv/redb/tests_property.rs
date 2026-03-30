/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use bytes::Bytes;
use lending_iterator::LendingIterator;
use primitive::key_range::{KeyRange, RangeStart};
use proptest::collection::vec;
use proptest::prelude::*;
use resource::profile::StorageCounters;

use crate::redb::RedbKVStore;

fn create_store() -> (RedbKVStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.redb");
    let store = RedbKVStore::new("test", 0, path).unwrap();
    (store, dir)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// put/get roundtrip: any key (1..256 bytes) and value (0..4096 bytes),
    /// put then get should return the value.
    #[test]
    fn put_get_roundtrip(
        key in vec(any::<u8>(), 1..256usize),
        value in vec(any::<u8>(), 0..4096usize)
    ) {
        let (store, _dir) = create_store();
        store.put(&key, &value).unwrap();
        let result = store.get(&key, |v| v.to_vec()).unwrap();
        prop_assert_eq!(result, Some(value));
    }

    /// Range iteration returns keys in sorted (ascending) order: insert random
    /// entries, iterate over all of them, verify keys are non-decreasing.
    #[test]
    fn range_iteration_sorted(
        entries in vec((vec(any::<u8>(), 1..32usize), vec(any::<u8>(), 1..32usize)), 1..50usize)
    ) {
        let (store, _dir) = create_store();
        for (k, v) in &entries {
            store.put(k, v).unwrap();
        }

        let start: Bytes<'_, 64> = Bytes::copy(&[0u8]);
        let range = KeyRange::new_unbounded(RangeStart::Inclusive(start));
        let mut iter = store.iterate_range(&range, StorageCounters::DISABLED);

        let mut keys: Vec<Vec<u8>> = Vec::new();
        while let Some(Ok((k, _v))) = iter.next() {
            keys.push(k.to_vec());
        }

        for w in keys.windows(2) {
            prop_assert!(
                w[0] <= w[1],
                "Keys not sorted: {:?} > {:?}",
                w[0],
                w[1]
            );
        }
    }

    /// Overwrite: put same key twice with different values, get returns the
    /// second (latest) value.
    #[test]
    fn overwrite_returns_latest(
        key in vec(any::<u8>(), 1..256usize),
        value1 in vec(any::<u8>(), 0..4096usize),
        value2 in vec(any::<u8>(), 0..4096usize)
    ) {
        let (store, _dir) = create_store();
        store.put(&key, &value1).unwrap();
        store.put(&key, &value2).unwrap();
        let result = store.get(&key, |v| v.to_vec()).unwrap();
        prop_assert_eq!(result, Some(value2));
    }

    /// get_prev returns the largest key <= query. Insert multiple distinct keys,
    /// then for each key verify get_prev returns the correct predecessor.
    #[test]
    fn get_prev_returns_correct_key(
        mut keys in vec(vec(any::<u8>(), 1..64usize), 2..30usize)
    ) {
        // Deduplicate and sort keys so we have a predictable order.
        keys.sort();
        keys.dedup();
        prop_assume!(keys.len() >= 2);

        let (store, _dir) = create_store();
        for (i, k) in keys.iter().enumerate() {
            // Use the index as a simple value so we can verify which entry was returned.
            store.put(k, &[i as u8]).unwrap();
        }

        // For each inserted key, get_prev with that exact key should return it.
        for (i, k) in keys.iter().enumerate() {
            let result = store.get_prev(k, |found_k, v| (found_k.to_vec(), v.to_vec()));
            prop_assert!(result.is_some(), "get_prev({:?}) returned None", k);
            let (found_key, found_val) = result.unwrap();
            prop_assert_eq!(&found_key, k, "get_prev exact match failed for key {:?}", k);
            prop_assert_eq!(found_val, vec![i as u8]);
        }

        // Query between two consecutive keys: should return the earlier key.
        for pair in keys.windows(2) {
            let lo = &pair[0];
            let hi = &pair[1];

            // Build a query key strictly between lo and hi by appending 0x00
            // to lo (since lo < hi and lo is a prefix-incomparable vec,
            // lo + [0] is > lo; it might equal hi, so skip that case).
            let mut between = lo.clone();
            between.push(0x00);
            if between >= *hi {
                continue; // Cannot construct a key strictly between these two.
            }

            let result = store.get_prev(&between, |found_k, _v| found_k.to_vec());
            prop_assert!(result.is_some(), "get_prev({:?}) returned None", between);
            let found_key = result.unwrap();
            prop_assert!(
                found_key >= *lo && found_key <= between,
                "get_prev({:?}) returned {:?}, expected key in [{:?}, {:?}]",
                between,
                found_key,
                lo,
                between
            );
        }

        // Query before the smallest key: should return None.
        if let Some(first) = keys.first() {
            if first.first().map_or(false, |&b| b > 0) {
                let before = vec![first[0] - 1];
                if before < *first {
                    let result = store.get_prev(&before, |k, _v| k.to_vec());
                    prop_assert!(
                        result.is_none(),
                        "get_prev({:?}) should be None but got {:?}",
                        before,
                        result
                    );
                }
            }
        }
    }
}
