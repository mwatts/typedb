/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::{Deref, Range},
};

use lending_iterator::higher_order::Hkt;
use primitive::prefix::Prefix;

use crate::byte_array::ByteArray;

pub mod byte_array;
pub mod util;

#[derive(Debug)]
pub enum Bytes<'bytes, const ARRAY_INLINE_SIZE: usize> {
    Array(ByteArray<ARRAY_INLINE_SIZE>),
    Reference(&'bytes [u8]),
}

impl<const INLINE_SIZE: usize> Clone for Bytes<'_, INLINE_SIZE> {
    fn clone(&self) -> Bytes<'static, INLINE_SIZE> {
        match self {
            Bytes::Array(array) => Bytes::Array(array.clone()),
            Bytes::Reference(reference) => Bytes::Array(ByteArray::from(*reference)),
        }
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Bytes<'static, ARRAY_INLINE_SIZE> {
    pub fn copy(bytes: &[u8]) -> Self {
        Self::Array(ByteArray::copy(bytes))
    }

    pub fn inline(bytes: [u8; ARRAY_INLINE_SIZE], length: usize) -> Self {
        Self::Array(ByteArray::inline(bytes, length))
    }
}

impl<'bytes, const ARRAY_INLINE_SIZE: usize> Bytes<'bytes, ARRAY_INLINE_SIZE> {
    pub const fn reference(bytes: &'bytes [u8]) -> Self {
        Self::Reference(bytes)
    }

    pub fn length(&self) -> usize {
        match self {
            Bytes::Array(array) => array.len(),
            Bytes::Reference(reference) => reference.len(),
        }
    }

    pub fn truncate(self, length: usize) -> Bytes<'bytes, ARRAY_INLINE_SIZE> {
        assert!(length <= self.length());
        match self {
            Bytes::Array(mut array) => {
                array.truncate(length);
                Bytes::Array(array)
            }
            Bytes::Reference(reference) => Bytes::Reference(&reference[..length]),
        }
    }

    pub fn into_range(self, range: Range<usize>) -> Bytes<'bytes, ARRAY_INLINE_SIZE> {
        assert!(range.start <= self.length() && range.end <= self.length());
        match self {
            Bytes::Array(mut array) => {
                array.truncate_range(range);
                Bytes::Array(array)
            }
            Bytes::Reference(reference) => Bytes::Reference(&reference[range]),
        }
    }

    pub fn to_owned(&self) -> Bytes<'static, ARRAY_INLINE_SIZE> {
        Bytes::Array(self.to_array())
    }

    pub fn into_owned(self) -> Bytes<'static, ARRAY_INLINE_SIZE> {
        Bytes::Array(self.into_array())
    }

    pub fn unwrap_reference(self) -> &'bytes [u8] {
        if let Bytes::Reference(reference) = self {
            reference
        } else {
            panic!("{} cannot be unwrapped as a reference", self)
        }
    }

    pub fn into_array(self) -> ByteArray<ARRAY_INLINE_SIZE> {
        match self {
            Bytes::Array(array) => array,
            Bytes::Reference(byte_reference) => ByteArray::from(byte_reference),
        }
    }

    pub fn to_array(&self) -> ByteArray<ARRAY_INLINE_SIZE> {
        match self {
            Bytes::Array(array) => array.clone(),
            Bytes::Reference(byte_reference) => ByteArray::from(*byte_reference),
        }
    }
}

impl<const ARRAY_INLINE_SIZE: usize> fmt::Display for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl<const ARRAY_INLINE_SIZE: usize> fmt::LowerHex for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("0x")?;
        for byte in &**self {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Deref for Bytes<'_, ARRAY_INLINE_SIZE> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Bytes::Array(array) => array,
            Bytes::Reference(reference) => reference,
        }
    }
}

impl<const ARRAY_INLINE_SIZE: usize> PartialEq for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn eq(&self, other: &Self) -> bool {
        (**self).eq(&**other)
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Eq for Bytes<'_, ARRAY_INLINE_SIZE> {}

impl<const ARRAY_INLINE_SIZE: usize> PartialOrd for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Ord for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn cmp(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Hash for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Borrow<[u8]> for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Prefix for Bytes<'_, ARRAY_INLINE_SIZE> {
    fn starts_with(&self, other: &Self) -> bool {
        (**self).starts_with(other)
    }

    fn into_starts_with(self, other: Self) -> bool {
        (*self).starts_with(&other)
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Hkt for Bytes<'static, ARRAY_INLINE_SIZE> {
    type HktSelf<'a> = Bytes<'a, ARRAY_INLINE_SIZE>;
}

impl<'a, const ARRAY_INLINE_SIZE: usize> From<Bytes<'a, ARRAY_INLINE_SIZE>> for Vec<u8> {
    fn from(value: Bytes<'a, ARRAY_INLINE_SIZE>) -> Self {
        Self::from(&*value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    #[test]
    fn reference_variant_holds_borrowed_data() {
        let data = b"hello";
        let bytes = Bytes::<16>::reference(data);
        assert_eq!(&*bytes, b"hello");
        assert_eq!(bytes.length(), 5);
    }

    #[test]
    fn copy_creates_owned_array() {
        let bytes = Bytes::<16>::copy(b"hello");
        assert!(matches!(bytes, Bytes::Array(_)));
        assert_eq!(&*bytes, b"hello");
    }

    #[test]
    fn inline_creates_array_variant() {
        let mut data = [0u8; 16];
        data[..5].copy_from_slice(b"hello");
        let bytes = Bytes::<16>::inline(data, 5);
        assert!(matches!(bytes, Bytes::Array(_)));
        assert_eq!(&*bytes, b"hello");
    }

    #[test]
    fn length_returns_correct_value_for_both_variants() {
        let reference = Bytes::<16>::reference(b"abc");
        let owned = Bytes::<16>::copy(b"abcdef");
        assert_eq!(reference.length(), 3);
        assert_eq!(owned.length(), 6);
    }

    #[test]
    fn truncate_reference_preserves_variant() {
        let data = b"hello world";
        let bytes = Bytes::<16>::reference(data);
        let truncated = bytes.truncate(5);
        assert!(matches!(truncated, Bytes::Reference(_)));
        assert_eq!(&*truncated, b"hello");
    }

    #[test]
    fn truncate_array_preserves_variant() {
        let bytes = Bytes::<16>::copy(b"hello world");
        let truncated = bytes.truncate(5);
        assert!(matches!(truncated, Bytes::Array(_)));
        assert_eq!(&*truncated, b"hello");
    }

    #[test]
    fn into_range_reference() {
        let data = b"hello world";
        let bytes = Bytes::<16>::reference(data);
        let ranged = bytes.into_range(6..11);
        assert!(matches!(ranged, Bytes::Reference(_)));
        assert_eq!(&*ranged, b"world");
    }

    #[test]
    fn into_range_array() {
        let bytes = Bytes::<16>::copy(b"hello world");
        let ranged = bytes.into_range(6..11);
        assert_eq!(&*ranged, b"world");
    }

    #[test]
    fn to_owned_converts_reference_to_owned() {
        let data = b"hello";
        let reference = Bytes::<16>::reference(data);
        let owned = reference.to_owned();
        assert!(matches!(owned, Bytes::Array(_)));
        assert_eq!(&*owned, b"hello");
    }

    #[test]
    fn into_owned_consumes_and_converts() {
        let data = b"hello";
        let reference = Bytes::<16>::reference(data);
        let owned = reference.into_owned();
        assert!(matches!(owned, Bytes::Array(_)));
        assert_eq!(&*owned, b"hello");
    }

    #[test]
    fn unwrap_reference_returns_slice() {
        let data = b"hello";
        let bytes = Bytes::<16>::reference(data);
        let slice = bytes.unwrap_reference();
        assert_eq!(slice, b"hello");
    }

    #[test]
    #[should_panic]
    fn unwrap_reference_panics_on_array() {
        let bytes = Bytes::<16>::copy(b"hello");
        let _ = bytes.unwrap_reference();
    }

    #[test]
    fn into_array_from_reference() {
        let data = b"hello";
        let bytes = Bytes::<16>::reference(data);
        let array = bytes.into_array();
        assert_eq!(&*array, b"hello");
    }

    #[test]
    fn into_array_from_array() {
        let bytes = Bytes::<16>::copy(b"hello");
        let array = bytes.into_array();
        assert_eq!(&*array, b"hello");
    }

    #[test]
    fn to_array_clones() {
        let bytes = Bytes::<16>::copy(b"hello");
        let array = bytes.to_array();
        assert_eq!(&*array, b"hello");
        // Original still accessible
        assert_eq!(&*bytes, b"hello");
    }

    #[test]
    fn clone_always_produces_owned() {
        let data = b"hello";
        let reference = Bytes::<16>::reference(data);
        let cloned = reference.clone();
        assert!(matches!(cloned, Bytes::Array(_)));
        assert_eq!(&*cloned, b"hello");
    }

    #[test]
    fn equality_across_variants() {
        let reference = Bytes::<16>::reference(b"hello");
        let owned = Bytes::<16>::copy(b"hello");
        assert_eq!(reference, owned);
    }

    #[test]
    fn inequality_different_content() {
        let a = Bytes::<16>::copy(b"hello");
        let b = Bytes::<16>::copy(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn ordering_is_lexicographic() {
        let a = Bytes::<16>::copy(&[0, 1]);
        let b = Bytes::<16>::copy(&[0, 2]);
        assert!(a < b);
    }

    #[test]
    fn hash_equal_for_same_content() {
        let reference = Bytes::<16>::reference(b"hello");
        let owned = Bytes::<16>::copy(b"hello");
        let hash_ref = {
            let mut h = DefaultHasher::new();
            reference.hash(&mut h);
            h.finish()
        };
        let hash_own = {
            let mut h = DefaultHasher::new();
            owned.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_ref, hash_own);
    }

    #[test]
    fn borrow_returns_slice() {
        let bytes = Bytes::<16>::copy(b"hello");
        let slice: &[u8] = bytes.borrow();
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn lower_hex_format() {
        let bytes = Bytes::<16>::copy(&[0xab, 0xcd, 0xef]);
        let hex = format!("{:x}", bytes);
        assert_eq!(hex, "0xabcdef");
    }

    #[test]
    fn into_vec() {
        let bytes = Bytes::<16>::copy(b"hello");
        let vec: Vec<u8> = Vec::from(bytes);
        assert_eq!(vec, b"hello");
    }

    #[test]
    fn prefix_trait_works() {
        let full = Bytes::<16>::copy(b"hello world");
        let prefix = Bytes::<16>::copy(b"hello");
        assert!(Prefix::starts_with(&full, &prefix));
    }

    #[test]
    fn prefix_trait_non_matching() {
        let full = Bytes::<16>::copy(b"hello");
        let not_prefix = Bytes::<16>::copy(b"world");
        assert!(!Prefix::starts_with(&full, &not_prefix));
    }
}
