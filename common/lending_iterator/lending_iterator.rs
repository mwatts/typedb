/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{borrow::Borrow, cmp::Ordering, marker::PhantomData, mem::transmute};

use crate::{
    adaptors::{
        Chain, Filter, FilterMap, FlatMap, Flatten, Inspect, Map, RepeatEach, TakeWhile, TryFilter, TryFlatMap, Zip,
    },
    higher_order::{AdHocHkt, FnHktHelper, FnMutHktHelper, Hkt},
};

pub mod adaptors;
pub mod higher_order;
pub mod kmerge;

pub trait LendingIterator: 'static {
    type Item<'a>;

    fn next(&mut self) -> Option<Self::Item<'_>>;

    fn chain<Other>(self, other: Other) -> Chain<Self, Other>
    where
        Self: Sized,
        Other: for<'a> LendingIterator<Item<'a> = Self::Item<'a>>,
    {
        Chain::new(self, other)
    }

    fn repeat_each(self, n: usize) -> RepeatEach<Self>
    where
        Self: Sized,
    {
        RepeatEach::new(self, n)
    }

    fn zip<Other>(self, other: Other) -> Zip<Self, Other>
    where
        Self: Sized,
        Other: LendingIterator,
    {
        Zip::new(self, other)
    }

    fn filter<P, F>(self, pred: P) -> Filter<Self, P>
    where
        Self: Sized,
        P: Borrow<F>,
        F: for<'a, 'b> FnHktHelper<&'a Self::Item<'b>, bool> + ?Sized,
    {
        Filter::new(self, pred)
    }

    fn try_filter<P, F, T, E>(self, pred: P) -> TryFilter<Self, P, T, E>
    where
        T: Hkt,
        Self: Sized + for<'a> LendingIterator<Item<'a> = Result<T::HktSelf<'a>, E>>,
        P: Borrow<F>,
        F: for<'a, 'b> FnHktHelper<&'a Self::Item<'b>, Result<bool, E>> + ?Sized,
    {
        TryFilter::new(self, pred)
    }

    fn take_while<P>(self, pred: P) -> TakeWhile<Self, P>
    where
        Self: Sized,
        P: FnMut(&Self::Item<'_>) -> bool,
    {
        TakeWhile::new(self, pred)
    }

    fn inspect<F>(self, f: F) -> Inspect<Self, F>
    where
        Self: Sized,
        F: FnMut(&Self::Item<'_>) + 'static,
    {
        Inspect::new(self, f)
    }

    fn map<B, F>(self, mapper: F) -> Map<Self, F, B>
    where
        Self: Sized,
        B: Hkt + 'static,
        F: for<'a> FnMutHktHelper<Self::Item<'a>, B::HktSelf<'a>>,
    {
        Map::new(self, mapper)
    }

    fn map_static<B, F>(self, mapper: F) -> Map<Self, F, AdHocHkt<B>>
    where
        Self: Sized,
        B: 'static,
        F: for<'a> FnMutHktHelper<Self::Item<'a>, B>,
    {
        Map::new(self, mapper)
    }

    fn filter_map<B, F>(self, mapper: F) -> FilterMap<Self, F, B>
    where
        Self: Sized,
        B: Hkt + 'static,
        F: for<'a> FnMutHktHelper<Self::Item<'a>, Option<B::HktSelf<'a>>>,
    {
        FilterMap::new(self, mapper)
    }

    fn flatten(self) -> Flatten<Self>
    where
        Self: Sized,
        Self::Item<'static>: LendingIterator,
    {
        Flatten::new(self)
    }

    fn flat_map<J, F>(self, mapper: F) -> FlatMap<Self, J, F>
    where
        Self: Sized,
        J: LendingIterator,
        F: for<'a> FnMutHktHelper<Self::Item<'a>, J>,
    {
        FlatMap::new(self, mapper)
    }

    fn try_flat_map<J, F, T, E>(self, mapper: F) -> TryFlatMap<Self, J, F, T, E>
    where
        Self: Sized,
        J: for<'a> LendingIterator<Item<'a> = Result<T, E>>,
        F: for<'a> FnMutHktHelper<Self::Item<'a>, Result<J, E>>,
    {
        TryFlatMap::new(self, mapper)
    }

    fn into_iter(self) -> IntoIter<Self>
    where
        Self: Sized,
        for<'a> Self::Item<'a>: 'static,
    {
        IntoIter::new(self)
    }

    fn collect<B>(self) -> B
    where
        Self: Sized,
        for<'a> Self::Item<'a>: 'static,
        B: FromIterator<Self::Item<'static>>,
    {
        self.into_iter().collect()
    }

    fn try_collect<B, E>(self) -> Result<B, E>
    where
        Self: Sized,
        for<'a> Self::Item<'a>: 'static,
        Result<B, E>: FromIterator<Self::Item<'static>>,
    {
        self.collect()
    }

    fn count(mut self) -> usize
    where
        Self: Sized,
    {
        let mut count = 0;
        while self.next().is_some() {
            count += 1;
        }
        count
    }

    fn count_as_ref(&mut self) -> usize
    where
        Self: Sized,
    {
        let mut count = 0;
        while self.next().is_some() {
            count += 1;
        }
        count
    }
}

pub struct IntoIter<I> {
    iter: I,
}

impl<I> IntoIter<I> {
    fn new(iter: I) -> Self {
        Self { iter }
    }
}

impl<I> Iterator for IntoIter<I>
where
    I: LendingIterator,
    for<'a> I::Item<'a>: 'static,
{
    type Item = I::Item<'static>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            // SAFETY: `Self::Item<'a>: 'static` implies that the item is independent from the iterator.
            transmute::<Option<I::Item<'_>>, Option<I::Item<'static>>>(self.iter.next())
        }
    }
}

pub trait Seekable<K: ?Sized>: LendingIterator {
    fn seek(&mut self, key: &K);
    fn compare_key(&self, item: &Self::Item<'_>, key: &K) -> Ordering;
}

pub struct Peekable<LI: LendingIterator> {
    pub iter: LI,
    pub item: Option<LI::Item<'static>>,
}

impl<LI: LendingIterator> Peekable<LI> {
    pub fn new(iter: LI) -> Self {
        Self { iter, item: None }
    }

    pub fn peek(&mut self) -> Option<&LI::Item<'_>> {
        if self.item.is_none() {
            self.item = unsafe {
                // SAFETY: the stored item is only accessible while mutably borrowing this iterator.
                // When the underlying iterator is advanced, the stored item is discarded.
                transmute::<Option<LI::Item<'_>>, Option<LI::Item<'static>>>(self.iter.next())
            };
        }
        self.get_peeked()
    }

    pub(crate) fn get_peeked(&self) -> Option<&LI::Item<'_>> {
        unsafe {
            // SAFETY: the item reference borrows this iterator. This iterator cannot be advanced while it exists.
            transmute::<Option<&LI::Item<'static>>, Option<&LI::Item<'_>>>(self.item.as_ref())
        }
    }
}

impl<LI: LendingIterator> LendingIterator for Peekable<LI> {
    type Item<'a> = LI::Item<'a>;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        match self.item.take() {
            Some(item) => Some(unsafe {
                // SAFETY: the item borrows this iterator mutably. This iterator cannot be advanced while it exists.
                transmute::<LI::Item<'static>, LI::Item<'_>>(item)
            }),
            None => self.iter.next(),
        }
    }
}

impl<K: ?Sized, LI> Seekable<K> for Peekable<LI>
where
    LI: Seekable<K>,
{
    fn seek(&mut self, key: &K) {
        if self.item.is_some() {
            let item = self.item.as_ref().unwrap();
            match self.compare_key(item, key) {
                Ordering::Less => (), // fallthrough
                Ordering::Equal => return,
                Ordering::Greater => {
                    unreachable!("Key behind the stored item in a Peekable iterator")
                }
            }
        }
        self.item = None;
        self.iter.seek(key);
    }

    fn compare_key(&self, item: &Self::Item<'_>, key: &K) -> Ordering {
        self.iter.compare_key(item, key)
    }
}

pub struct AsNarrowingIterator<I, Item: Hkt> {
    iter: I,
    _phantom_item: PhantomData<Item::HktSelf<'static>>,
}

impl<I: Iterator<Item = Item::HktSelf<'static>>, Item: Hkt> AsNarrowingIterator<I, Item> {
    pub fn new(iter: impl IntoIterator<IntoIter = I>) -> Self {
        Self { iter: iter.into_iter(), _phantom_item: PhantomData }
    }
}

impl<I, Item> LendingIterator for AsNarrowingIterator<I, Item>
where
    I: Iterator<Item = Item::HktSelf<'static>> + 'static,
    Item: Hkt,
{
    type Item<'a> = Item::HktSelf<'a>;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        unsafe {
            // SAFETY: this strictly narrows the lifetime
            self.iter.next().map(|item| transmute::<Self::Item<'static>, Self::Item<'_>>(item))
        }
    }
}

pub struct AsLendingIterator<I: Iterator> {
    iter: I,
}

impl<I: Iterator> AsLendingIterator<I> {
    pub fn new(iter: impl IntoIterator<IntoIter = I>) -> Self {
        AsLendingIterator { iter: iter.into_iter() }
    }
}

impl<I: Iterator + 'static> LendingIterator for AsLendingIterator<I> {
    type Item<'a> = I::Item;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        self.iter.next()
    }
}

pub struct Once<T: Hkt> {
    inner: Option<T::HktSelf<'static>>,
}

pub fn once<T: Hkt>(inner: T::HktSelf<'static>) -> Once<T> {
    Once { inner: Some(inner) }
}

impl<T: Hkt> LendingIterator for Once<T> {
    type Item<'a> = T::HktSelf<'a>;

    fn next(&mut self) -> Option<Self::Item<'_>> {
        self.inner.take().map(|item| unsafe {
            // SAFETY: this strictly narrows the lifetime
            transmute::<Self::Item<'static>, Self::Item<'_>>(item)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AsLendingIterator ---

    #[test]
    fn as_lending_iterator_produces_all_items() {
        let mut iter = AsLendingIterator::new(vec![1, 2, 3]);
        assert_eq!(iter.next(), Some(1));
        assert_eq!(iter.next(), Some(2));
        assert_eq!(iter.next(), Some(3));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn as_lending_iterator_empty() {
        let mut iter = AsLendingIterator::new(Vec::<i32>::new());
        assert_eq!(iter.next(), None);
    }

    // --- count ---

    #[test]
    fn count_elements() {
        let iter = AsLendingIterator::new(vec![1, 2, 3]);
        assert_eq!(iter.count(), 3);
    }

    #[test]
    fn count_empty() {
        let iter = AsLendingIterator::new(Vec::<i32>::new());
        assert_eq!(iter.count(), 0);
    }

    #[test]
    fn count_as_ref_elements() {
        let mut iter = AsLendingIterator::new(vec![1, 2, 3]);
        assert_eq!(iter.count_as_ref(), 3);
    }

    // --- collect ---

    #[test]
    fn collect_to_vec() {
        let iter = AsLendingIterator::new(vec![1, 2, 3]);
        let result: Vec<i32> = iter.collect();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn collect_empty() {
        let iter = AsLendingIterator::new(Vec::<i32>::new());
        let result: Vec<i32> = iter.collect();
        assert!(result.is_empty());
    }

    // --- into_iter ---

    #[test]
    fn into_iter_conversion() {
        let iter = AsLendingIterator::new(vec![10, 20, 30]);
        let std_iter = iter.into_iter();
        let result: Vec<i32> = std_iter.collect();
        assert_eq!(result, vec![10, 20, 30]);
    }

    // --- chain ---

    #[test]
    fn chain_two_iterators() {
        let a = AsLendingIterator::new(vec![1, 2]);
        let b = AsLendingIterator::new(vec![3, 4]);
        let result: Vec<i32> = a.chain(b).collect();
        assert_eq!(result, vec![1, 2, 3, 4]);
    }

    #[test]
    fn chain_first_empty() {
        let a = AsLendingIterator::new(Vec::<i32>::new());
        let b = AsLendingIterator::new(vec![1, 2]);
        let result: Vec<i32> = a.chain(b).collect();
        assert_eq!(result, vec![1, 2]);
    }

    #[test]
    fn chain_second_empty() {
        let a = AsLendingIterator::new(vec![1, 2]);
        let b = AsLendingIterator::new(Vec::<i32>::new());
        let result: Vec<i32> = a.chain(b).collect();
        assert_eq!(result, vec![1, 2]);
    }

    #[test]
    fn chain_both_empty() {
        let a = AsLendingIterator::new(Vec::<i32>::new());
        let b = AsLendingIterator::new(Vec::<i32>::new());
        let result: Vec<i32> = a.chain(b).collect();
        assert!(result.is_empty());
    }

    // --- zip ---

    #[test]
    fn zip_same_length() {
        let a = AsLendingIterator::new(vec![1, 2, 3]);
        let b = AsLendingIterator::new(vec![10, 20, 30]);
        let result: Vec<(i32, i32)> = a.zip(b).collect();
        assert_eq!(result, vec![(1, 10), (2, 20), (3, 30)]);
    }

    #[test]
    fn zip_first_shorter() {
        let a = AsLendingIterator::new(vec![1, 2]);
        let b = AsLendingIterator::new(vec![10, 20, 30]);
        let result: Vec<(i32, i32)> = a.zip(b).collect();
        assert_eq!(result, vec![(1, 10), (2, 20)]);
    }

    #[test]
    fn zip_second_shorter() {
        let a = AsLendingIterator::new(vec![1, 2, 3]);
        let b = AsLendingIterator::new(vec![10]);
        let result: Vec<(i32, i32)> = a.zip(b).collect();
        assert_eq!(result, vec![(1, 10)]);
    }

    #[test]
    fn zip_empty() {
        let a = AsLendingIterator::new(Vec::<i32>::new());
        let b = AsLendingIterator::new(vec![1, 2]);
        let result: Vec<(i32, i32)> = a.zip(b).collect();
        assert!(result.is_empty());
    }

    // --- map_static ---

    #[test]
    fn map_static_transforms_items() {
        let iter = AsLendingIterator::new(vec![1, 2, 3]);
        let result: Vec<i32> = iter.map_static(|x: i32| x * 2).collect();
        assert_eq!(result, vec![2, 4, 6]);
    }

    #[test]
    fn map_static_empty() {
        let iter = AsLendingIterator::new(Vec::<i32>::new());
        let result: Vec<i32> = iter.map_static(|x: i32| x * 2).collect();
        assert!(result.is_empty());
    }

    // --- take_while ---

    #[test]
    fn take_while_stops_at_predicate() {
        let iter = AsLendingIterator::new(vec![1, 2, 3, 4, 5]);
        let result: Vec<i32> = iter.take_while(|x: &i32| *x < 4).collect();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn take_while_all_match() {
        let iter = AsLendingIterator::new(vec![1, 2, 3]);
        let result: Vec<i32> = iter.take_while(|x: &i32| *x < 10).collect();
        assert_eq!(result, vec![1, 2, 3]);
    }

    #[test]
    fn take_while_none_match() {
        let iter = AsLendingIterator::new(vec![10, 20, 30]);
        let result: Vec<i32> = iter.take_while(|x: &i32| *x < 5).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn take_while_empty() {
        let iter = AsLendingIterator::new(Vec::<i32>::new());
        let result: Vec<i32> = iter.take_while(|_: &i32| true).collect();
        assert!(result.is_empty());
    }

    // --- try_collect ---

    #[test]
    fn try_collect_all_ok() {
        let iter = AsLendingIterator::new(vec![Ok(1), Ok(2), Ok(3)]);
        let result: Result<Vec<i32>, &str> = iter.try_collect();
        assert_eq!(result, Ok(vec![1, 2, 3]));
    }

    #[test]
    fn try_collect_with_error() {
        let iter = AsLendingIterator::new(vec![Ok(1), Err("fail"), Ok(3)]);
        let result: Result<Vec<i32>, &str> = iter.try_collect();
        assert_eq!(result, Err("fail"));
    }

    // --- Once ---

    #[test]
    fn once_produces_single_item() {
        let mut iter = once::<AdHocHkt<i32>>(42);
        assert_eq!(iter.next(), Some(42));
        assert_eq!(iter.next(), None);
    }

    // --- Peekable ---

    #[test]
    fn peekable_peek_returns_item() {
        let inner = AsLendingIterator::new(vec![1, 2, 3]);
        let mut peekable = Peekable::new(inner);
        assert_eq!(peekable.peek(), Some(&1));
        assert_eq!(peekable.peek(), Some(&1)); // peek again returns same item
    }

    #[test]
    fn peekable_next_after_peek() {
        let inner = AsLendingIterator::new(vec![1, 2, 3]);
        let mut peekable = Peekable::new(inner);
        assert_eq!(peekable.peek(), Some(&1));
        assert_eq!(peekable.next(), Some(1));
        assert_eq!(peekable.next(), Some(2));
        assert_eq!(peekable.next(), Some(3));
        assert_eq!(peekable.next(), None);
    }

    #[test]
    fn peekable_next_without_peek() {
        let inner = AsLendingIterator::new(vec![1, 2]);
        let mut peekable = Peekable::new(inner);
        assert_eq!(peekable.next(), Some(1));
        assert_eq!(peekable.next(), Some(2));
        assert_eq!(peekable.next(), None);
    }

    #[test]
    fn peekable_empty() {
        let inner = AsLendingIterator::new(Vec::<i32>::new());
        let mut peekable = Peekable::new(inner);
        assert_eq!(peekable.peek(), None);
        assert_eq!(peekable.next(), None);
    }

    // --- inspect ---

    #[test]
    fn inspect_sees_all_items() {
        let mut seen = Vec::new();
        let seen_ptr = &mut seen as *mut Vec<i32>;
        let iter = AsLendingIterator::new(vec![1, 2, 3]);
        let inspected = iter.inspect(move |x: &i32| unsafe { (*seen_ptr).push(*x) });
        let result: Vec<i32> = inspected.collect();
        assert_eq!(result, vec![1, 2, 3]);
        assert_eq!(seen, vec![1, 2, 3]);
    }
}
