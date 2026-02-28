/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::fmt;

use structural_equality::{ordered_hash_combine, StructuralEquality};

#[derive(Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct Variable {
    id: VariableId,
    anonymous: bool,
}

impl Variable {
    pub fn new(id: u16) -> Self {
        Self { id: VariableId { id }, anonymous: false }
    }

    pub fn id(&self) -> VariableId {
        self.id
    }

    pub fn new_anonymous(id: u16) -> Self {
        Self { id: VariableId { id }, anonymous: true }
    }

    pub fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    pub fn is_named(&self) -> bool {
        !self.anonymous
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.anonymous {
            write!(f, "$_{}", self.id)
        } else {
            write!(f, "${}", self.id)
        }
    }
}

impl fmt::Debug for Variable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.anonymous {
            write!(f, "$_{}", self.id)
        } else {
            write!(f, "${}", self.id)
        }
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct VariableId {
    // TODO: retain line/character from original query at which point this Variable was declared
    id: u16,
}

impl VariableId {
    pub fn as_u16(&self) -> u16 {
        self.id
    }
}

impl fmt::Display for VariableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)
    }
}

impl StructuralEquality for Variable {
    fn hash(&self) -> u64 {
        ordered_hash_combine(self.anonymous as u64, self.id.id as u64)
    }

    fn equals(&self, other: &Self) -> bool {
        self == other && self.anonymous == other.anonymous
    }
}

#[cfg(test)]
mod tests {
    use structural_equality::is_structurally_equivalent;

    use super::*;

    #[test]
    fn named_variable_properties() {
        let var = Variable::new(5);
        assert_eq!(var.id().as_u16(), 5);
        assert!(var.is_named());
        assert!(!var.is_anonymous());
    }

    #[test]
    fn anonymous_variable_properties() {
        let var = Variable::new_anonymous(3);
        assert_eq!(var.id().as_u16(), 3);
        assert!(var.is_anonymous());
        assert!(!var.is_named());
    }

    #[test]
    fn named_variable_display() {
        let var = Variable::new(42);
        assert_eq!(format!("{}", var), "$42");
    }

    #[test]
    fn anonymous_variable_display() {
        let var = Variable::new_anonymous(7);
        assert_eq!(format!("{}", var), "$_7");
    }

    #[test]
    fn named_variable_debug() {
        let var = Variable::new(10);
        assert_eq!(format!("{:?}", var), "$10");
    }

    #[test]
    fn anonymous_variable_debug() {
        let var = Variable::new_anonymous(0);
        assert_eq!(format!("{:?}", var), "$_0");
    }

    #[test]
    fn equality_same_named_variables() {
        let a = Variable::new(5);
        let b = Variable::new(5);
        assert_eq!(a, b);
    }

    #[test]
    fn inequality_different_ids() {
        let a = Variable::new(1);
        let b = Variable::new(2);
        assert_ne!(a, b);
    }

    #[test]
    fn inequality_named_vs_anonymous_same_id() {
        let named = Variable::new(5);
        let anon = Variable::new_anonymous(5);
        assert_ne!(named, anon);
    }

    #[test]
    fn ordering_by_id() {
        let a = Variable::new(1);
        let b = Variable::new(2);
        assert!(a < b);
    }

    #[test]
    fn variable_id_display() {
        let var = Variable::new(99);
        assert_eq!(format!("{}", var.id()), "99");
    }

    #[test]
    fn variable_id_as_u16() {
        let var = Variable::new(1000);
        assert_eq!(var.id().as_u16(), 1000);
    }

    #[test]
    fn structural_equality_same_variables() {
        let a = Variable::new(5);
        let b = Variable::new(5);
        assert!(is_structurally_equivalent(&a, &b));
    }

    #[test]
    fn structural_equality_different_variables() {
        let a = Variable::new(5);
        let b = Variable::new(6);
        assert!(!is_structurally_equivalent(&a, &b));
    }

    #[test]
    fn structural_equality_named_vs_anonymous() {
        let named = Variable::new(5);
        let anon = Variable::new_anonymous(5);
        assert!(!is_structurally_equivalent(&named, &anon));
    }

    #[test]
    fn structural_hash_consistency() {
        let a = Variable::new(5);
        let b = Variable::new(5);
        assert_eq!(StructuralEquality::hash(&a), StructuralEquality::hash(&b));
    }

    #[test]
    fn clone_preserves_properties() {
        let original = Variable::new_anonymous(3);
        let cloned = original;
        assert_eq!(cloned.id().as_u16(), 3);
        assert!(cloned.is_anonymous());
    }
}
