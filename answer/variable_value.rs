/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{cmp::Ordering, fmt, sync::Arc};

use encoding::value::value::Value;
use lending_iterator::higher_order::Hkt;

use crate::{Thing, Type};

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum VariableValue<'a> {
    None,
    Type(Type),
    Thing(Thing),
    Value(Value<'a>),
    ThingList(Arc<[Thing]>),
    ValueList(Arc<[Value<'static>]>),
}

impl<'a> VariableValue<'a> {
    pub fn as_type(&self) -> Type {
        self.get_type().unwrap_or_else(|| panic!("VariableValue is not a Type: {:?}", self))
    }

    pub fn get_type(&self) -> Option<Type> {
        match self {
            &VariableValue::Type(type_) => Some(type_),
            _ => None,
        }
    }

    pub fn as_thing(&self) -> &Thing {
        self.get_thing().unwrap_or_else(|| panic!("VariableValue is not a Thing: {:?}", self))
    }

    pub fn get_thing(&self) -> Option<&Thing> {
        match self {
            VariableValue::Thing(thing) => Some(thing),
            _ => None,
        }
    }

    pub fn as_value(&self) -> &Value<'a> {
        match self {
            VariableValue::Value(value) => value,
            _ => panic!("VariableValue is not a value: {:?}", self),
        }
    }

    pub fn to_owned(&self) -> VariableValue<'static> {
        match self {
            VariableValue::None => VariableValue::None,
            &VariableValue::Type(type_) => VariableValue::Type(type_),
            VariableValue::Thing(thing) => VariableValue::Thing(thing.to_owned()),
            VariableValue::Value(value) => VariableValue::Value(value.clone().into_owned()),
            VariableValue::ThingList(list) => VariableValue::ThingList(list.clone()),
            VariableValue::ValueList(list) => VariableValue::ValueList(list.clone()),
        }
    }

    pub fn as_reference(&self) -> VariableValue<'_> {
        match self {
            VariableValue::None => VariableValue::None,
            &VariableValue::Type(type_) => VariableValue::Type(type_),
            VariableValue::Thing(thing) => VariableValue::Thing(thing.clone()),
            VariableValue::Value(value) => VariableValue::Value(value.as_reference()),
            VariableValue::ThingList(list) => VariableValue::ThingList(list.clone()),
            VariableValue::ValueList(list) => VariableValue::ValueList(list.clone()),
        }
    }

    pub fn into_owned(self) -> VariableValue<'static> {
        match self {
            VariableValue::None => VariableValue::None,
            VariableValue::Type(type_) => VariableValue::Type(type_),
            VariableValue::Thing(thing) => VariableValue::Thing(thing),
            VariableValue::Value(value) => VariableValue::Value(value.into_owned()),
            VariableValue::ThingList(list) => VariableValue::ThingList(list),
            VariableValue::ValueList(list) => VariableValue::ValueList(list),
        }
    }

    pub fn next_possible(&self) -> VariableValue<'static> {
        match self {
            VariableValue::None => unreachable!("No next value for an None value."),
            VariableValue::Type(type_) => VariableValue::Type(type_.next_possible()),
            VariableValue::Thing(thing) => VariableValue::Thing(thing.next_possible()),
            VariableValue::Value(_) => unreachable!("Value instances don't have a well defined order."),
            VariableValue::ThingList(_) | VariableValue::ValueList(_) => {
                unreachable!("Lists have no well defined order.")
            }
        }
    }

    /// Returns `true` if the variable value is [`None`].
    ///
    /// [`None`]: VariableValue::None
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn variant_name(&self) -> &'static str {
        match self {
            VariableValue::None => "none",
            VariableValue::Type(_) => "type",
            VariableValue::Thing(_) => "thing",
            VariableValue::Value(_) => "value",
            VariableValue::ThingList(_) => "thing list",
            VariableValue::ValueList(_) => "value list",
        }
    }
}

impl Hkt for VariableValue<'static> {
    type HktSelf<'a> = VariableValue<'a>;
}

impl PartialOrd for VariableValue<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            // special case: None is less than everything, except also equal to None
            (Self::None, Self::None) => Some(Ordering::Equal),
            (Self::None, _) => Some(Ordering::Less),
            (_, Self::None) => Some(Ordering::Greater),
            (Self::Type(self_type), Self::Type(other_type)) => self_type.partial_cmp(other_type),
            (Self::Thing(self_thing), Self::Thing(other_thing)) => self_thing.partial_cmp(other_thing),
            (Self::Value(self_value), Self::Value(other_value)) => self_value.partial_cmp(other_value),
            _ => None,
        }
    }
}

impl fmt::Display for VariableValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableValue::None => write!(f, "[None]"),
            VariableValue::Type(type_) => write!(f, "{}", type_),
            VariableValue::Thing(thing) => write!(f, "{}", thing),
            VariableValue::Value(value) => write!(f, "{}", value),
            VariableValue::ThingList(thing_list) => {
                write!(f, "[")?;
                for thing in thing_list.as_ref() {
                    write!(f, "{}, ", thing)?;
                }
                write!(f, "]")
            }
            VariableValue::ValueList(value_list) => {
                write!(f, "[")?;
                for value in value_list.as_ref() {
                    write!(f, "{}, ", value)?;
                }
                write!(f, "]")
            }
        }
    }
}

impl VariableValue<'_> {
    pub const NONE: VariableValue<'static> = VariableValue::None;
}

pub enum FunctionValue<'a> {
    Thing(Thing),
    ThingOptional(Option<Thing>),
    Value(Value<'a>),
    ValueOptional(Option<Value<'a>>),
    ThingList(Vec<Thing>),
    ThingListOptional(Option<Vec<Thing>>),
    ValueList(Vec<Value<'a>>),
    ValueListOptional(Option<Vec<Value<'a>>>),
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    #[test]
    fn none_variant_is_none() {
        let val = VariableValue::None;
        assert!(val.is_none());
    }

    #[test]
    fn none_constant_is_none() {
        assert!(VariableValue::NONE.is_none());
    }

    #[test]
    fn none_variant_name() {
        assert_eq!(VariableValue::None.variant_name(), "none");
    }

    #[test]
    fn none_display() {
        let display = format!("{}", VariableValue::None);
        assert_eq!(display, "[None]");
    }

    #[test]
    fn none_equality() {
        assert_eq!(VariableValue::None, VariableValue::None);
    }

    #[test]
    fn none_partial_ord_with_none() {
        assert_eq!(VariableValue::None.partial_cmp(&VariableValue::None), Some(Ordering::Equal));
    }

    #[test]
    fn none_to_owned() {
        let owned = VariableValue::None.to_owned();
        assert!(owned.is_none());
    }

    #[test]
    fn none_into_owned() {
        let owned = VariableValue::None.into_owned();
        assert!(owned.is_none());
    }

    #[test]
    fn none_as_reference() {
        let reference = VariableValue::None.as_reference();
        assert!(reference.is_none());
    }

    #[test]
    fn none_get_type_returns_none() {
        assert!(VariableValue::None.get_type().is_none());
    }

    #[test]
    fn none_get_thing_returns_none() {
        assert!(VariableValue::None.get_thing().is_none());
    }

    #[test]
    #[should_panic(expected = "VariableValue is not a Type")]
    fn none_as_type_panics() {
        let _ = VariableValue::None.as_type();
    }

    #[test]
    #[should_panic(expected = "VariableValue is not a Thing")]
    fn none_as_thing_panics() {
        let _ = VariableValue::None.as_thing();
    }

    #[test]
    #[should_panic(expected = "VariableValue is not a value")]
    fn none_as_value_panics() {
        let _ = VariableValue::None.as_value();
    }

    #[test]
    fn value_variant_with_long() {
        let val = VariableValue::Value(Value::Integer(42));
        assert!(!val.is_none());
        assert_eq!(val.variant_name(), "value");
        assert_eq!(*val.as_value(), Value::Integer(42));
    }

    #[test]
    fn value_variant_to_owned() {
        let val = VariableValue::Value(Value::Integer(42));
        let owned = val.to_owned();
        assert_eq!(owned.variant_name(), "value");
    }

    #[test]
    fn value_variant_into_owned() {
        let val = VariableValue::Value(Value::Boolean(true));
        let owned = val.into_owned();
        assert_eq!(owned.variant_name(), "value");
    }

    #[test]
    fn value_partial_ord_same_type() {
        let a = VariableValue::Value(Value::Integer(1));
        let b = VariableValue::Value(Value::Integer(2));
        assert_eq!(a.partial_cmp(&b), Some(Ordering::Less));
    }

    #[test]
    fn none_less_than_value() {
        let none = VariableValue::None;
        let value = VariableValue::Value(Value::Integer(1));
        assert_eq!(none.partial_cmp(&value), Some(Ordering::Less));
    }

    #[test]
    fn value_greater_than_none() {
        let value = VariableValue::Value(Value::Integer(1));
        let none = VariableValue::None;
        assert_eq!(value.partial_cmp(&none), Some(Ordering::Greater));
    }

    #[test]
    fn value_display() {
        let val = VariableValue::Value(Value::Integer(42));
        let display = format!("{}", val);
        assert!(display.contains("42"));
    }

    #[test]
    fn empty_thing_list() {
        let val = VariableValue::ThingList(Arc::from(Vec::<Thing>::new().into_boxed_slice()));
        assert_eq!(val.variant_name(), "thing list");
        assert!(!val.is_none());
    }

    #[test]
    fn empty_value_list() {
        let val = VariableValue::ValueList(Arc::from(Vec::<Value<'static>>::new().into_boxed_slice()));
        assert_eq!(val.variant_name(), "value list");
        assert!(!val.is_none());
    }
}
