/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
use std::{
    cmp::Ordering,
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use bytes::{byte_array::ByteArray, Bytes};
use error::TypeDBError;
use lending_iterator::{LendingIterator, Seekable};
use primitive::key_range::{KeyRange, RangeEnd, RangeStart};
use crate::iterator::ContinueCondition;

pub struct InMemoryRangeIterator {
    /// Snapshot of the data taken at iterator creation time.
    entries: Vec<(Box<[u8]>, Box<[u8]>)>,
    /// Current position in entries.
    pos: usize,
    continue_condition: ContinueCondition,
    is_finished: bool,
}

impl InMemoryRangeIterator {
    pub(crate) fn new<const INLINE_BYTES: usize>(
        data: &Arc<RwLock<BTreeMap<Box<[u8]>, Box<[u8]>>>>,
        range: &KeyRange<Bytes<'_, INLINE_BYTES>>,
    ) -> Self {
        let data = data.read().unwrap_or_else(|e| e.into_inner());

        // Determine the start key for the BTreeMap range scan.
        let start_key: Vec<u8> = match range.start() {
            RangeStart::Inclusive(bytes) => bytes.as_ref().to_vec(),
            RangeStart::ExcludeFirstWithPrefix(bytes) => bytes.as_ref().to_vec(),
            RangeStart::ExcludePrefix(bytes) => {
                let mut cloned = bytes.to_array();
                cloned.increment().unwrap();
                cloned.as_ref().to_vec()
            }
        };

        // Collect only the entries that fall within the requested range. We apply the
        // `continue_condition` end-bound eagerly here rather than lazily in `next()`,
        // avoiding cloning entries that would immediately be rejected. This mirrors what
        // RocksDB does with its bloom-filter and prefix-seek optimisations.
        let start_boxed: Box<[u8]> = start_key.into_boxed_slice();

        // Determine the upper bound for the BTreeMap range scan from the end condition.
        let entries: Vec<(Box<[u8]>, Box<[u8]>)> = match range.end() {
            RangeEnd::WithinStartAsPrefix => {
                let prefix: Box<[u8]> = range.start().get_value().as_ref().into();
                data.range::<Box<[u8]>, _>(start_boxed..)
                    .take_while(|(k, _)| k.starts_with(&prefix))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            }
            RangeEnd::EndPrefixInclusive(end) => {
                let end_box: Box<[u8]> = end.as_ref().into();
                data.range::<Box<[u8]>, _>(start_boxed..)
                    .take_while(|(k, _)| k.as_ref() <= end_box.as_ref() || k.starts_with(&end_box))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            }
            RangeEnd::EndPrefixExclusive(end) => {
                let end_box: Box<[u8]> = end.as_ref().into();
                data.range::<Box<[u8]>, _>(start_boxed..)
                    .take_while(|(k, _)| k.as_ref() < end_box.as_ref())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            }
            RangeEnd::Unbounded => {
                data.range::<Box<[u8]>, _>(start_boxed..).map(|(k, v)| (k.clone(), v.clone())).collect()
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
        let mut pos = 0;
        if matches!(range.start(), RangeStart::ExcludeFirstWithPrefix(_)) {
            let start_value = range.start().get_value().as_ref();
            if entries.first().is_some_and(|(k, _)| k.as_ref() == start_value) {
                pos = 1;
            }
        }

        Self { entries, pos, continue_condition, is_finished: false }
    }

    fn accept_current(&self) -> bool {
        if self.pos >= self.entries.len() {
            return false;
        }
        let key: &[u8] = &self.entries[self.pos].0;
        match &self.continue_condition {
            ContinueCondition::ExactPrefix(prefix) => key.starts_with(prefix),
            ContinueCondition::EndPrefixInclusive(end_inclusive) => {
                key <= end_inclusive.as_ref() || key.starts_with(end_inclusive)
            }
            ContinueCondition::EndPrefixExclusive(end_exclusive) => key < end_exclusive.as_ref(),
            ContinueCondition::Always => true,
        }
    }
}

impl LendingIterator for InMemoryRangeIterator {
    type Item<'a>
        = Result<(&'a [u8], &'a [u8]), Box<dyn TypeDBError>>
    where
        Self: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        if self.is_finished {
            return None;
        }
        if !self.accept_current() {
            self.is_finished = true;
            return None;
        }
        let (key, value) = &self.entries[self.pos];
        self.pos += 1;
        Some(Ok((key.as_ref(), value.as_ref())))
    }
}

impl Seekable<[u8]> for InMemoryRangeIterator {
    fn seek(&mut self, key: &[u8]) {
        if self.is_finished {
            return;
        }
        // Binary search forward from current position.
        let search_range = &self.entries[self.pos..];
        let offset = search_range.partition_point(|(k, _)| k.as_ref() < key);
        self.pos += offset;
    }

    fn compare_key(&self, item: &Self::Item<'_>, key: &[u8]) -> Ordering {
        if let Ok((peek, _)) = item {
            peek.cmp(&key)
        } else {
            Ordering::Equal
        }
    }
}
