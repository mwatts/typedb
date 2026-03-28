/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::cmp::Ordering;

use error::TypeDBError;
use lending_iterator::{LendingIterator, Seekable};

use crate::iterator::ContinueCondition;

pub struct RedbRangeIterator {
    /// Snapshot of the data taken at iterator creation time.
    entries: Vec<(Box<[u8]>, Box<[u8]>)>,
    /// Current position in entries.
    pos: usize,
    continue_condition: ContinueCondition,
    is_finished: bool,
}

impl RedbRangeIterator {
    pub(crate) fn new_from_entries(entries: Vec<(Box<[u8]>, Box<[u8]>)>, continue_condition: ContinueCondition) -> Self {
        Self { entries, pos: 0, continue_condition, is_finished: false }
    }

    pub(crate) fn new_from_entries_at(
        entries: Vec<(Box<[u8]>, Box<[u8]>)>,
        continue_condition: ContinueCondition,
        start_pos: usize,
    ) -> Self {
        Self { entries, pos: start_pos, continue_condition, is_finished: false }
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

impl LendingIterator for RedbRangeIterator {
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

impl Seekable<[u8]> for RedbRangeIterator {
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
