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
    ops::{Deref, DerefMut, Range},
};

use primitive::prefix::Prefix;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::util::{increment, BytesError, HexBytesFormatter};

#[derive(Clone)]
pub enum ByteArray<const INLINE_BYTES: usize> {
    Inline { bytes: [u8; INLINE_BYTES], len: u8 },
    Boxed(Box<[u8]>),
}

impl<const INLINE_BYTES: usize> ByteArray<INLINE_BYTES> {
    pub const fn empty() -> ByteArray<INLINE_BYTES> {
        ByteArray::Inline { bytes: [0; INLINE_BYTES], len: 0 }
    }

    pub fn zeros(length: usize) -> ByteArray<INLINE_BYTES> {
        if length <= INLINE_BYTES {
            Self::Inline { bytes: [0; INLINE_BYTES], len: length as u8 }
        } else {
            ByteArray::Boxed(vec![0u8; length].into())
        }
    }

    pub fn copy(bytes: &[u8]) -> ByteArray<INLINE_BYTES> {
        if bytes.len() <= INLINE_BYTES {
            let mut inline = [0; INLINE_BYTES];
            inline[..bytes.len()].copy_from_slice(bytes);
            ByteArray::Inline { bytes: inline, len: bytes.len() as u8 }
        } else {
            ByteArray::Boxed(bytes.into())
        }
    }

    pub const fn copy_inline(bytes: &[u8]) -> ByteArray<INLINE_BYTES> {
        assert!(bytes.len() <= INLINE_BYTES);
        let mut inline = [0; INLINE_BYTES];
        let mut i = 0;
        while i < bytes.len() {
            inline[i] = bytes[i];
            i += 1;
        }
        ByteArray::Inline { bytes: inline, len: bytes.len() as u8 }
    }

    pub fn copy_concat<const N: usize>(slices: [&[u8]; N]) -> ByteArray<INLINE_BYTES> {
        let length: usize = slices.iter().map(|slice| slice.len()).sum();
        if length <= INLINE_BYTES {
            let mut data = [0; INLINE_BYTES];
            let mut end = 0;
            for slice in slices {
                data[end..][..slice.len()].copy_from_slice(slice);
                end += slice.len();
            }
            ByteArray::Inline { len: length as u8, bytes: data }
        } else {
            ByteArray::Boxed(slices.concat().into_boxed_slice())
        }
    }

    pub fn inline(bytes: [u8; INLINE_BYTES], len: usize) -> ByteArray<INLINE_BYTES> {
        ByteArray::Inline { bytes, len: len as u8 }
    }

    pub fn boxed(bytes: Box<[u8]>) -> ByteArray<INLINE_BYTES> {
        ByteArray::Boxed(bytes)
    }

    pub fn truncate(&mut self, length: usize) {
        assert!(length <= self.len());
        match self {
            ByteArray::Inline { len, .. } => *len = length as u8,
            ByteArray::Boxed(boxed) => *boxed = boxed[..length].into(),
        }
    }

    pub fn truncate_range(&mut self, range: Range<usize>) {
        assert!(range.start <= self.len() && range.end <= self.len());
        match self {
            ByteArray::Inline { bytes, len } => {
                *len = range.len() as u8;
                bytes.copy_within(range, 0);
            }
            ByteArray::Boxed(boxed) => *boxed = boxed[range].into(),
        }
    }

    pub fn starts_with(&self, bytes: &[u8]) -> bool {
        self.len() >= bytes.len() && &self[0..bytes.len()] == bytes
    }

    pub fn increment(&mut self) -> Result<(), BytesError> {
        increment(self)
    }
}

impl<const INLINE_BYTES: usize> fmt::Debug for ByteArray<INLINE_BYTES> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", HexBytesFormatter::borrowed(self))
    }
}

impl<const BYTES: usize> From<&[u8]> for ByteArray<BYTES> {
    fn from(byte_reference: &[u8]) -> Self {
        ByteArray::copy(byte_reference)
    }
}

impl<const INLINE_BYTES: usize> Hash for ByteArray<INLINE_BYTES> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl<const INLINE_SIZE: usize> Serialize for ByteArray<INLINE_SIZE> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ByteArray::Inline { bytes, len } => Box::<[u8]>::from(&bytes[..*len as usize]).serialize(serializer),
            ByteArray::Boxed(bytes) => bytes.serialize(serializer),
        }
    }
}

impl<'de, const INLINE_SIZE: usize> Deserialize<'de> for ByteArray<INLINE_SIZE> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::Boxed(Box::deserialize(deserializer)?))
    }
}

impl<const INLINE_SIZE: usize> AsRef<[u8]> for ByteArray<INLINE_SIZE> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<const INLINE_SIZE: usize> Borrow<[u8]> for ByteArray<INLINE_SIZE> {
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl<const INLINE_SIZE: usize> Deref for ByteArray<INLINE_SIZE> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Boxed(bytes) => bytes,
            Self::Inline { bytes, len } => &bytes[..*len as usize],
        }
    }
}

impl<const INLINE_SIZE: usize> DerefMut for ByteArray<INLINE_SIZE> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Boxed(bytes) => bytes,
            Self::Inline { bytes, len } => &mut bytes[..*len as usize],
        }
    }
}

impl<const INLINE_BYTES: usize> PartialOrd for ByteArray<INLINE_BYTES> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<const INLINE_BYTES: usize> Ord for ByteArray<INLINE_BYTES> {
    fn cmp(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}

impl<const INLINE_BYTES: usize> PartialEq for ByteArray<INLINE_BYTES> {
    fn eq(&self, other: &Self) -> bool {
        (**self).eq(&**other)
    }
}

impl<const INLINE_BYTES: usize> Eq for ByteArray<INLINE_BYTES> {}

impl<const INLINE_BYTES: usize> PartialEq<[u8]> for ByteArray<INLINE_BYTES> {
    fn eq(&self, other: &[u8]) -> bool {
        &**self == other
    }
}

impl<const ARRAY_INLINE_SIZE: usize> Prefix for ByteArray<ARRAY_INLINE_SIZE> {
    fn starts_with(&self, other: &Self) -> bool {
        self.starts_with(other)
    }

    fn into_starts_with(self, other: Self) -> bool {
        self.starts_with(&other)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    #[test]
    fn empty_has_zero_length() {
        let arr = ByteArray::<16>::empty();
        assert_eq!(arr.len(), 0);
        assert_eq!(&*arr, &[] as &[u8]);
    }

    #[test]
    fn zeros_creates_inline_when_small() {
        let arr = ByteArray::<16>::zeros(8);
        assert_eq!(arr.len(), 8);
        assert!(matches!(arr, ByteArray::Inline { .. }));
        assert!(arr.iter().all(|&b| b == 0));
    }

    #[test]
    fn zeros_creates_boxed_when_large() {
        let arr = ByteArray::<8>::zeros(100);
        assert_eq!(arr.len(), 100);
        assert!(matches!(arr, ByteArray::Boxed(_)));
        assert!(arr.iter().all(|&b| b == 0));
    }

    #[test]
    fn copy_small_is_inline() {
        let arr = ByteArray::<16>::copy(b"hello");
        assert_eq!(arr.len(), 5);
        assert!(matches!(arr, ByteArray::Inline { .. }));
        assert_eq!(&*arr, b"hello");
    }

    #[test]
    fn copy_large_is_boxed() {
        let data = vec![42u8; 100];
        let arr = ByteArray::<8>::copy(&data);
        assert_eq!(arr.len(), 100);
        assert!(matches!(arr, ByteArray::Boxed(_)));
        assert_eq!(&*arr, &data[..]);
    }

    #[test]
    fn copy_exact_inline_size_is_inline() {
        let data = [1u8; 16];
        let arr = ByteArray::<16>::copy(&data);
        assert!(matches!(arr, ByteArray::Inline { .. }));
        assert_eq!(arr.len(), 16);
    }

    #[test]
    fn copy_inline_basic() {
        let arr = ByteArray::<16>::copy_inline(b"test");
        assert_eq!(&*arr, b"test");
    }

    #[test]
    fn copy_concat_two_slices() {
        let arr = ByteArray::<16>::copy_concat([b"hel", b"lo"]);
        assert_eq!(&*arr, b"hello");
    }

    #[test]
    fn copy_concat_empty_slices() {
        let arr = ByteArray::<16>::copy_concat([b"", b"", b""]);
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn copy_concat_boxed_when_large() {
        let a = vec![1u8; 50];
        let b = vec![2u8; 50];
        let arr = ByteArray::<8>::copy_concat([&a, &b]);
        assert_eq!(arr.len(), 100);
        assert!(matches!(arr, ByteArray::Boxed(_)));
    }

    #[test]
    fn truncate_shortens_inline() {
        let mut arr = ByteArray::<16>::copy(b"hello world");
        arr.truncate(5);
        assert_eq!(&*arr, b"hello");
    }

    #[test]
    fn truncate_shortens_boxed() {
        let data = vec![1u8; 100];
        let mut arr = ByteArray::<8>::copy(&data);
        arr.truncate(10);
        assert_eq!(arr.len(), 10);
    }

    #[test]
    fn truncate_to_zero() {
        let mut arr = ByteArray::<16>::copy(b"hello");
        arr.truncate(0);
        assert_eq!(arr.len(), 0);
    }

    #[test]
    fn truncate_range_middle() {
        let mut arr = ByteArray::<16>::copy(b"hello world");
        arr.truncate_range(6..11);
        assert_eq!(&*arr, b"world");
    }

    #[test]
    fn truncate_range_from_start() {
        let mut arr = ByteArray::<16>::copy(b"hello");
        arr.truncate_range(0..3);
        assert_eq!(&*arr, b"hel");
    }

    #[test]
    fn starts_with_matching_prefix() {
        let arr = ByteArray::<16>::copy(b"hello world");
        assert!(arr.starts_with(b"hello"));
    }

    #[test]
    fn starts_with_non_matching() {
        let arr = ByteArray::<16>::copy(b"hello");
        assert!(!arr.starts_with(b"world"));
    }

    #[test]
    fn starts_with_empty_prefix() {
        let arr = ByteArray::<16>::copy(b"hello");
        assert!(arr.starts_with(b""));
    }

    #[test]
    fn starts_with_longer_prefix_returns_false() {
        let arr = ByteArray::<16>::copy(b"hi");
        assert!(!arr.starts_with(b"hello"));
    }

    #[test]
    fn starts_with_exact_match() {
        let arr = ByteArray::<16>::copy(b"hello");
        assert!(arr.starts_with(b"hello"));
    }

    #[test]
    fn increment_no_carry() {
        let mut arr = ByteArray::<4>::copy(&[0, 0, 0, 1]);
        arr.increment().unwrap();
        assert_eq!(&*arr, &[0, 0, 0, 2]);
    }

    #[test]
    fn increment_with_carry() {
        let mut arr = ByteArray::<4>::copy(&[0, 0, 0, 255]);
        arr.increment().unwrap();
        assert_eq!(&*arr, &[0, 0, 1, 0]);
    }

    #[test]
    fn increment_overflow() {
        let mut arr = ByteArray::<4>::copy(&[255, 255, 255, 255]);
        assert!(arr.increment().is_err());
    }

    #[test]
    fn deref_provides_slice_access() {
        let arr = ByteArray::<16>::copy(b"hello");
        let slice: &[u8] = &arr;
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn deref_mut_allows_modification() {
        let mut arr = ByteArray::<16>::copy(b"hello");
        arr[0] = b'H';
        assert_eq!(&*arr, b"Hello");
    }

    #[test]
    fn equality_between_inline_and_boxed() {
        let inline = ByteArray::<16>::copy(b"hello");
        let boxed = ByteArray::<4>::copy(b"hello");
        // Different types so can't directly compare, but content is same
        assert_eq!(&*inline, &*boxed);
    }

    #[test]
    fn equality_same_content() {
        let a = ByteArray::<16>::copy(b"test");
        let b = ByteArray::<16>::copy(b"test");
        assert_eq!(a, b);
    }

    #[test]
    fn inequality_different_content() {
        let a = ByteArray::<16>::copy(b"hello");
        let b = ByteArray::<16>::copy(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn equality_with_raw_slice() {
        let arr = ByteArray::<16>::copy(b"hello");
        assert!(arr == b"hello"[..]);
    }

    #[test]
    fn ordering_lexicographic() {
        let a = ByteArray::<16>::copy(&[0, 1]);
        let b = ByteArray::<16>::copy(&[0, 2]);
        assert!(a < b);
    }

    #[test]
    fn ordering_prefix_shorter_is_less() {
        let a = ByteArray::<16>::copy(&[1, 2]);
        let b = ByteArray::<16>::copy(&[1, 2, 3]);
        assert!(a < b);
    }

    #[test]
    fn hash_same_for_equal_arrays() {
        let a = ByteArray::<16>::copy(b"test");
        let b = ByteArray::<16>::copy(b"test");
        let hash_a = {
            let mut hasher = DefaultHasher::new();
            a.hash(&mut hasher);
            hasher.finish()
        };
        let hash_b = {
            let mut hasher = DefaultHasher::new();
            b.hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn clone_produces_independent_copy() {
        let original = ByteArray::<16>::copy(b"hello");
        let mut cloned = original.clone();
        cloned[0] = b'H';
        assert_eq!(&*original, b"hello");
        assert_eq!(&*cloned, b"Hello");
    }

    #[test]
    fn from_slice_conversion() {
        let arr: ByteArray<16> = ByteArray::from(&b"hello"[..]);
        assert_eq!(&*arr, b"hello");
    }

    #[test]
    fn as_ref_returns_slice() {
        let arr = ByteArray::<16>::copy(b"hello");
        let slice: &[u8] = arr.as_ref();
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn borrow_returns_slice() {
        let arr = ByteArray::<16>::copy(b"hello");
        let slice: &[u8] = arr.borrow();
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn debug_format_shows_hex() {
        let arr = ByteArray::<16>::copy(&[0xab, 0xcd]);
        let debug = format!("{:?}", arr);
        assert!(debug.contains("AB"));
        assert!(debug.contains("CD"));
    }

    #[test]
    fn prefix_trait_works() {
        let full = ByteArray::<16>::copy(b"hello world");
        let prefix = ByteArray::<16>::copy(b"hello");
        assert!(Prefix::starts_with(&full, &prefix));
    }

    #[test]
    fn prefix_trait_into_starts_with() {
        let full = ByteArray::<16>::copy(b"hello world");
        let prefix = ByteArray::<16>::copy(b"hello");
        assert!(Prefix::into_starts_with(full, prefix));
    }
}
