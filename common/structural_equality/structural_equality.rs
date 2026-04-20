/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    mem::Discriminant,
};

mod typeql_structural_equality;

pub fn is_structurally_equivalent<T: StructuralEquality>(first: &T, second: &T) -> bool {
    let first_hash = first.hash();
    let second_hash = second.hash();
    first_hash == second_hash && first.equals(second)
}

pub fn ordered_hash_combine(a: u64, b: u64) -> u64 {
    a ^ (b.wrapping_add(0x9e3779b9).wrapping_add(a << 6).wrapping_add(a >> 2))
}

pub trait StructuralEquality {
    // following the java-style hashing
    fn hash(&self) -> u64;

    fn hash_into(&self, hasher: &mut impl Hasher) {
        hasher.write_u64(self.hash())
    }

    fn equals(&self, other: &Self) -> bool;
}

impl StructuralEquality for bool {
    fn hash(&self) -> u64 {
        *self as u64
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl StructuralEquality for u64 {
    fn hash(&self) -> u64 {
        *self
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl StructuralEquality for usize {
    fn hash(&self) -> u64 {
        *self as u64
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: StructuralEquality> StructuralEquality for Vec<T> {
    fn hash(&self) -> u64 {
        (self as &[T]).hash()
    }

    fn equals(&self, other: &Self) -> bool {
        (self as &[T]).equals(other)
    }
}

impl<T: StructuralEquality> StructuralEquality for [T] {
    fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.iter().for_each(|element| element.hash_into(&mut hasher));
        hasher.finish()
    }

    fn equals(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a.equals(b))
    }
}

impl<T: StructuralEquality + Hash> StructuralEquality for HashSet<T> {
    fn hash(&self) -> u64 {
        self.iter().fold(0, |acc, element| {
            // values may generally be in a small rang, so we run them through a hasher first to make the XOR more effective
            let mut hasher = DefaultHasher::new();
            element.hash_into(&mut hasher);
            // WARNING: must use XOR or other commutative operator!
            acc ^ hasher.finish()
        })
    }

    /// Note: this is a quadratic operation! Best to precede with a Hash check elsewhere.
    fn equals(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        self.iter().all(|element| other.iter().any(|other_element| element.equals(other_element)))
    }
}

impl<K: StructuralEquality + Ord, V: StructuralEquality> StructuralEquality for BTreeMap<K, V> {
    fn hash(&self) -> u64 {
        self.iter().fold(0, |acc, (key, value)| {
            // WARNING: must use XOR or other commutative operator!
            acc ^ (key, value).hash()
        })
    }

    fn equals(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        self.iter().all(|(key, value)| {
            // TODO: bit strange that we don't use structural equality here, but we do in the hash?
            other.get(key).is_some_and(|other_value| value.equals(other_value))
        })
    }
}

impl<K: StructuralEquality + Hash, V: StructuralEquality> StructuralEquality for HashMap<K, V> {
    fn hash(&self) -> u64 {
        self.iter().fold(0, |acc, (key, value)| {
            // WARNING: must use XOR or other commutative operator!
            acc ^ (key, value).hash()
        })
    }

    fn equals(&self, other: &Self) -> bool {
        // Note: this is a quadratic operation! Best to precede with a Hash check elsewhere.
        if self.len() != other.len() {
            return false;
        }

        self.iter().all(|(key, value)| {
            other.iter().any(|(other_key, other_value)| key.equals(other_key) && value.equals(other_value))
        })
    }
}

impl<T: StructuralEquality> StructuralEquality for Option<T> {
    fn hash(&self) -> u64 {
        match self {
            None => 0,
            Some(inner) => inner.hash(),
        }
    }

    fn equals(&self, other: &Self) -> bool {
        match (self, other) {
            (None, None) => true,
            (Some(inner), Some(other_inner)) => inner.equals(other_inner),
            _ => false,
        }
    }
}

// NOTE: specifically not `AsRef<str>` since this may admit too many equalities by accident - we must get &str explicitly first
impl StructuralEquality for str {
    fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        Hash::hash(self, &mut hasher);
        hasher.finish()
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T> StructuralEquality for Discriminant<T> {
    fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        Hash::hash(self, &mut hasher);
        hasher.finish()
    }

    fn equals(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: StructuralEquality> StructuralEquality for &T {
    fn hash(&self) -> u64 {
        T::hash(self)
    }

    fn equals(&self, other: &Self) -> bool {
        T::equals(self, other)
    }
}

macro_rules! tuple_structural_equality {
    ($($t:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($t),+> StructuralEquality for ($($t),+)
        where $($t: StructuralEquality,)+
        {
            fn hash(&self) -> u64 {
                #[allow(non_snake_case)]
                let ($($t),+) = self;

                let mut hasher = DefaultHasher::new();
                $($t.hash_into(&mut hasher);)+
                hasher.finish()
            }

            fn equals(&self, other: &Self) -> bool {
                paste::paste! {
                    #[allow(non_snake_case)]
                    let ($($t),+) = self;

                    #[allow(non_snake_case)]
                    let ($([< other_ $t >]),+) = other;

                    $($t.equals([<other_ $t>])) && +
                }
            }
        }
    };
}

tuple_structural_equality! { T, U }
tuple_structural_equality! { T, U, V }

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use super::*;

    // --- Primitive types ---

    #[test]
    fn bool_hash_and_equality() {
        assert_eq!(StructuralEquality::hash(&true), 1);
        assert_eq!(StructuralEquality::hash(&false), 0);
        assert!(StructuralEquality::equals(&true, &true));
        assert!(StructuralEquality::equals(&false, &false));
        assert!(!StructuralEquality::equals(&true, &false));
    }

    #[test]
    fn u64_hash_is_identity() {
        assert_eq!(StructuralEquality::hash(&42u64), 42);
        assert!(StructuralEquality::equals(&42u64, &42u64));
        assert!(!StructuralEquality::equals(&42u64, &43u64));
    }

    #[test]
    fn usize_hash_and_equality() {
        assert!(StructuralEquality::equals(&10usize, &10usize));
        assert!(!StructuralEquality::equals(&10usize, &20usize));
    }

    // --- is_structurally_equivalent ---

    #[test]
    fn is_structurally_equivalent_matching() {
        assert!(is_structurally_equivalent(&42u64, &42u64));
    }

    #[test]
    fn is_structurally_equivalent_different() {
        assert!(!is_structurally_equivalent(&42u64, &43u64));
    }

    // --- ordered_hash_combine ---

    #[test]
    fn ordered_hash_combine_is_order_dependent() {
        let ab = ordered_hash_combine(1, 2);
        let ba = ordered_hash_combine(2, 1);
        assert_ne!(ab, ba);
    }

    #[test]
    fn ordered_hash_combine_different_inputs_different_results() {
        let a = ordered_hash_combine(100, 200);
        let b = ordered_hash_combine(100, 201);
        assert_ne!(a, b);
    }

    // --- Vec / Slice ---

    #[test]
    fn vec_equality() {
        let a: Vec<u64> = vec![1, 2, 3];
        let b: Vec<u64> = vec![1, 2, 3];
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn vec_inequality_different_content() {
        let a: Vec<u64> = vec![1, 2, 3];
        let b: Vec<u64> = vec![1, 2, 4];
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn vec_inequality_different_length() {
        let a: Vec<u64> = vec![1, 2];
        let b: Vec<u64> = vec![1, 2, 3];
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn empty_vecs_equal() {
        let a: Vec<u64> = vec![];
        let b: Vec<u64> = vec![];
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    // --- HashSet ---

    #[test]
    fn hashset_equality_order_independent() {
        let a: HashSet<u64> = [1, 2, 3].into();
        let b: HashSet<u64> = [3, 1, 2].into();
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn hashset_inequality_different_content() {
        let a: HashSet<u64> = [1, 2, 3].into();
        let b: HashSet<u64> = [1, 2, 4].into();
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn hashset_inequality_different_size() {
        let a: HashSet<u64> = [1, 2].into();
        let b: HashSet<u64> = [1, 2, 3].into();
        assert!(!StructuralEquality::equals(&a, &b));
    }

    // --- BTreeMap ---

    #[test]
    fn btreemap_equality() {
        let a: BTreeMap<u64, u64> = [(1, 10), (2, 20)].into();
        let b: BTreeMap<u64, u64> = [(2, 20), (1, 10)].into();
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn btreemap_inequality_different_values() {
        let a: BTreeMap<u64, u64> = [(1, 10)].into();
        let b: BTreeMap<u64, u64> = [(1, 11)].into();
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn btreemap_inequality_different_size() {
        let a: BTreeMap<u64, u64> = [(1, 10)].into();
        let b: BTreeMap<u64, u64> = [(1, 10), (2, 20)].into();
        assert!(!StructuralEquality::equals(&a, &b));
    }

    // --- HashMap ---

    #[test]
    fn hashmap_equality() {
        let a: HashMap<u64, u64> = [(1, 10), (2, 20)].into_iter().collect();
        let b: HashMap<u64, u64> = [(2, 20), (1, 10)].into_iter().collect();
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn hashmap_inequality_different_values() {
        let a: HashMap<u64, u64> = [(1, 10)].into_iter().collect();
        let b: HashMap<u64, u64> = [(1, 11)].into_iter().collect();
        assert!(!StructuralEquality::equals(&a, &b));
    }

    // --- Option ---

    #[test]
    fn option_none_equals_none() {
        let a: Option<u64> = None;
        let b: Option<u64> = None;
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn option_some_equals_some() {
        let a = Some(42u64);
        let b = Some(42u64);
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn option_some_not_equals_none() {
        let a = Some(42u64);
        let b: Option<u64> = None;
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn option_none_not_equals_some() {
        let a: Option<u64> = None;
        let b = Some(42u64);
        assert!(!StructuralEquality::equals(&a, &b));
    }

    // --- str ---

    #[test]
    fn str_equality() {
        assert!(StructuralEquality::equals("hello", "hello"));
        assert!(!StructuralEquality::equals("hello", "world"));
    }

    #[test]
    fn str_hash_consistency() {
        let h1 = StructuralEquality::hash("hello");
        let h2 = StructuralEquality::hash("hello");
        assert_eq!(h1, h2);
    }

    // --- Reference ---

    #[test]
    fn reference_delegates_to_inner() {
        let a = 42u64;
        let b = 42u64;
        assert!(StructuralEquality::equals(&&a, &&b));
        assert_eq!(StructuralEquality::hash(&&a), StructuralEquality::hash(&&b));
    }

    // --- Tuples ---

    #[test]
    fn tuple2_equality() {
        let a = (1u64, 2u64);
        let b = (1u64, 2u64);
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn tuple2_inequality() {
        let a = (1u64, 2u64);
        let b = (1u64, 3u64);
        assert!(!StructuralEquality::equals(&a, &b));
    }

    #[test]
    fn tuple3_equality() {
        let a = (1u64, 2u64, 3u64);
        let b = (1u64, 2u64, 3u64);
        assert!(StructuralEquality::equals(&a, &b));
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn tuple3_inequality() {
        let a = (1u64, 2u64, 3u64);
        let b = (1u64, 2u64, 4u64);
        assert!(!StructuralEquality::equals(&a, &b));
    }

    // --- hash_into ---

    #[test]
    fn hash_into_writes_hash_value() {
        let val = 42u64;
        let mut hasher = DefaultHasher::new();
        StructuralEquality::hash_into(&val, &mut hasher);
        let result = hasher.finish();
        // hash_into should write the structural hash (42) into the hasher
        let mut expected_hasher = DefaultHasher::new();
        expected_hasher.write_u64(42);
        assert_eq!(result, expected_hasher.finish());
    }
}
