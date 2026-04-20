/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{error::Error, fmt};

use ::typeql::common::Spannable;
use resource::constants::common::{ERROR_QUERY_POINTER_LINES_AFTER, ERROR_QUERY_POINTER_LINES_BEFORE};

mod typeql;

pub trait TypeDBError {
    fn variant_name(&self) -> &'static str;

    fn component(&self) -> &'static str;

    fn code(&self) -> &'static str;

    fn code_prefix(&self) -> &'static str;

    fn code_number(&self) -> usize;

    fn format_description(&self) -> String;

    fn source_error(&self) -> Option<&(dyn Error + Sync)>;

    fn source_typedb_error(&self) -> Option<&(dyn TypeDBError + Sync)>;

    fn root_source_typedb_error(&self) -> &(dyn TypeDBError + Sync)
    where
        Self: Sized + Sync,
    {
        let mut error: &(dyn TypeDBError + Sync) = self;
        while let Some(source) = error.source_typedb_error() {
            error = source;
        }
        error
    }

    fn source_query(&self) -> Option<&str>;

    fn source_span(&self) -> Option<::typeql::common::Span>;

    fn format_code_and_description(&self) -> String {
        if let Some(query) = self.source_query() {
            if let Some((line_col, _)) = self.bottom_source_span().and_then(|span| query.line_col(span)) {
                if let Some(excerpt) = query.extract_annotated_line_col(
                    // note: span line and col are 1-indexed,must adjust to 0-offset
                    line_col.line as usize - 1,
                    line_col.column as usize - 1,
                    ERROR_QUERY_POINTER_LINES_BEFORE,
                    ERROR_QUERY_POINTER_LINES_AFTER,
                ) {
                    return format!(
                        "[{}] {}\nNear {}:{}\n-----\n{}\n-----",
                        self.code(),
                        self.format_description(),
                        line_col.line,
                        line_col.column,
                        excerpt
                    );
                }
            }
        }
        format!("[{}] {}", self.code(), self.format_description())
    }

    // return most-specific span available
    fn bottom_source_span(&self) -> Option<::typeql::common::Span> {
        self.source_typedb_error().and_then(|err| err.bottom_source_span()).or_else(|| self.source_span())
    }

    fn stack_trace(&self) -> Vec<String>
    where
        Self: Sized + Sync,
    {
        let mut stack_trace = Vec::with_capacity(4); // definitely non-zero!
        let mut error: &(dyn TypeDBError + Sync) = self;
        stack_trace.push(error.format_code_and_description());
        while let Some(source) = error.source_typedb_error() {
            error = source;
            stack_trace.push(error.format_code_and_description());
        }
        if let Some(source) = error.source_error() {
            stack_trace.push(format!("{}", source));
        }
        stack_trace.reverse();
        stack_trace
    }
}

impl PartialEq for dyn TypeDBError {
    fn eq(&self, other: &Self) -> bool {
        self.code() == other.code()
    }
}

impl Eq for dyn TypeDBError {}

impl fmt::Debug for dyn TypeDBError + '_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for dyn TypeDBError + '_ {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = self.source_typedb_error() {
            write!(f, "{}\nCause: \n      {:?}", self.format_code_and_description(), source as &dyn TypeDBError)
        } else if let Some(source) = self.source_error() {
            write!(f, "{}\nCause: \n      {:?}", self.format_code_and_description(), source)
        } else {
            write!(f, "{}", self.format_code_and_description())
        }
    }
}

impl<T: TypeDBError> TypeDBError for Box<T> {
    fn variant_name(&self) -> &'static str {
        (**self).variant_name()
    }

    fn component(&self) -> &'static str {
        (**self).component()
    }

    fn code(&self) -> &'static str {
        (**self).code()
    }

    fn code_prefix(&self) -> &'static str {
        (**self).code_prefix()
    }

    fn code_number(&self) -> usize {
        (**self).code_number()
    }

    fn format_description(&self) -> String {
        (**self).format_description()
    }

    fn source_error(&self) -> Option<&(dyn Error + Sync)> {
        (**self).source_error()
    }

    fn source_typedb_error(&self) -> Option<&(dyn TypeDBError + Sync)> {
        (**self).source_typedb_error()
    }

    fn source_query(&self) -> Option<&str> {
        (**self).source_query()
    }

    fn source_span(&self) -> Option<::typeql::common::Span> {
        (**self).source_span()
    }
}

// ***USAGE WARNING***: We should not set both Source and TypeDBSource, TypeDBSource has precedence!
#[macro_export]
macro_rules! typedb_error {
    ($vis:vis $name:ident(component = $component:literal, prefix = $prefix:literal) { $(
        $variant:ident($number:literal, $description:literal $(, $($arg:tt)*)?),
    )*}) => {
        #[derive(Clone)]
        $vis enum $name {
            $($variant { $($($arg)*)? }),*
        }

        const _: () = {
            // fail to compile if any Numbers are the same
            trait Assert {}
            $(impl Assert for [(); $number ] {})*
        };

        impl $crate::TypeDBError for $name {
            fn variant_name(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => stringify!($variant),)*
                }
            }

            fn component(&self) -> &'static str {
                &$component
            }

            fn code(&self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => concat!($prefix, stringify!($number)),)*
                }
            }

            fn code_prefix(&self) -> &'static str {
                $prefix
            }

            fn code_number(&self) -> usize {
                match self {
                    $(Self::$variant { .. } => $number,)*
                }
            }

            fn format_description(&self) -> String {
                match self {
                    $(typedb_error!(@args $variant { $($($arg)*)? }) => format!($description),)*
                }
            }

            fn source_error(&self) -> Option<&(dyn ::std::error::Error + Sync + 'static)> {
                match self {
                    $(typedb_error!(@source source from $variant { $($($arg)*)? })=> {
                        typedb_error!(@source source { $($($arg)*)? })
                    })*
                }
            }

            fn source_typedb_error(&self) -> Option<&(dyn $crate::TypeDBError + Sync + 'static)> {
                match self {
                    $(typedb_error!(@typedb_source typedb_source from $variant { $($($arg)*)? })=> {
                        typedb_error!(@typedb_source typedb_source { $($($arg)*)? })
                    })*
                }
            }

            fn source_query(&self) -> Option<&str> {
                match self {
                    $(typedb_error!(@source_query source_query from $variant { $($($arg)*)? })=> {
                        typedb_error!(@source_query source_query { $($($arg)*)? })
                    })*
                }
            }

            fn source_span(&self) -> Option<::typeql::common::Span> {
                match self {
                    $(typedb_error!(@source_span source_span from $variant { $($($arg)*)? })=> {
                        typedb_error!(@source_span source_span { $($($arg)*)? })
                    })*
                }
            }
        }

        impl ::std::fmt::Debug for $name {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::fmt::Debug::fmt(self as &dyn $crate::TypeDBError, f)
            }
        }
    };

    (@args $variant:ident { $($arg:ident : $ty:ty),* $(,)? }) => {
        Self::$variant { $($arg),* }
    };

    (@source $ts:ident from $variant:ident { source : $argty:ty $(, $($rest:tt)*)? }) => {
        Self::$variant { source: $ts, .. }
    };
    (@source $ts:ident from $variant:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source $ts from $variant { $($($rest)*)? })
    };
    (@source $ts:ident from $variant:ident { $(,)? }) => {
        Self::$variant { .. }
    };

    (@source $ts:ident { source: $_:ty $(, $($rest:tt)*)? }) => {
        Some($ts as &(dyn ::std::error::Error + Sync + 'static))
    };
    (@source $ts:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source $ts { $($($rest)*)? })
    };
    (@source $ts:ident { $(,)? }) => {
        None
    };

    (@typedb_source $ts:ident from $variant:ident { typedb_source : $argty:ty $(, $($rest:tt)*)? }) => {
        Self::$variant { typedb_source: $ts, .. }
    };
    (@typedb_source $ts:ident from $variant:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@typedb_source $ts from $variant { $($($rest)*)? })
    };
    (@typedb_source $ts:ident from $variant:ident { $(,)? }) => {
        Self::$variant { .. }
    };

    (@typedb_source $ts:ident { typedb_source: $_:ty $(, $($rest:tt)*)? }) => {
        Some($ts as &(dyn $crate::TypeDBError + Sync + 'static))
    };
    (@typedb_source $ts:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@typedb_source $ts { $($($rest)*)? })
    };
    (@typedb_source $ts:ident { $(,)? }) => {
        None
    };

    (@source_query $ts:ident from $variant:ident { source_query : $argty:ty $(, $($rest:tt)*)? }) => {
        Self::$variant { source_query: $ts, .. }
    };
    (@source_query $ts:ident from $variant:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source_query $ts from $variant { $($($rest)*)? })
    };
    (@source_query $ts:ident from $variant:ident { $(,)? }) => {
        Self::$variant { .. }
    };

    (@source_query $ts:ident { source_query: $_:ty $(, $($rest:tt)*)? }) => {
        Some($ts)
    };
    (@source_query $ts:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source_query $ts { $($($rest)*)? })
    };
    (@source_query $ts:ident { $(,)? }) => {
        None
    };

    (@source_span $ts:ident from $variant:ident { source_span : $argty:ty $(, $($rest:tt)*)? }) => {
        Self::$variant { source_span: $ts, .. }
    };
    (@source_span $ts:ident from $variant:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source_span $ts from $variant { $($($rest)*)? })
    };
    (@source_span $ts:ident from $variant:ident { $(,)? }) => {
        Self::$variant { .. }
    };

    (@source_span $ts:ident { source_span: $_:ty $(, $($rest:tt)*)? }) => {
        *$ts
    };
    (@source_span $ts:ident { $arg:ident : $argty:ty $(, $($rest:tt)*)? }) => {
        typedb_error!(@source_span $ts { $($($rest)*)? })
    };
    (@source_span $ts:ident { $(,)? }) => {
        None
    };
}

// Check for usages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnimplementedFeature {
    Lists,
    Structs,

    BuiltinFunction(String),
    LetInBuiltinCall,
    Subkey,
    OptionalFunctions,

    UnsortedJoin,

    PipelineStageInFunction(&'static str),

    IrrelevantUnboundInvertedMode(&'static str),
    QueryingAnnotations,

    NestedOptionalWrites,
}
impl std::fmt::Display for UnimplementedFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[macro_export]
macro_rules! unimplemented_feature {
    ($feature:ident) => {
        unreachable!(
            "FATAL: entered unreachable code that relies on feature: {}. This is a bug!",
            error::UnimplementedFeature::$feature
        )
    };
    ($feature:ident, $msg:literal) => {
        unreachable!(
            "FATAL: entered unreachable code that relies on feature: {}. This is a bug! Details: {}",
            error::UnimplementedFeature::$feature,
            $msg
        )
    };
}

#[cfg(debug_assertions)]
#[macro_export]
macro_rules! todo_must_implement {
    ($msg:literal) => {
        todo!(concat!("TODO: Must implement: ", $msg)) // Ensure this is enabled when checking in.
    };
}

#[cfg(not(debug_assertions))]
#[macro_export]
macro_rules! todo_must_implement {
    ($msg:literal) => {
        compile_error!(concat!("TODO: Must implement: ", $msg)) // Ensure this is enabled when checking in.
    };
}

#[macro_export]
macro_rules! todo_display_for_error {
    ($f:ident, $self:ident) => {
        write!(
            $f,
            "(Proper formatting has not yet been implemented for {})\nThe error is: {:?}\n",
            std::any::type_name::<Self>(),
            $self
        )
    };
}

#[macro_export]
macro_rules! ensure_unimplemented_unused {
    () => {
        compile_error!("Implement this path before making the function usable")
    };
}

#[macro_export]
macro_rules! needs_update_when_feature_is_implemented {
    // Nothing, we just need the compile error when the feature is deleted
    ($feature:ident) => {};
    ($feature:ident, $msg:literal) => {};
}

#[cfg(test)]
mod tests {
    use super::*;

    typedb_error! {
        TestError(component = "Test", prefix = "TST") {
            SimpleError(1, "A simple error occurred."),
            ErrorWithValue(2, "Error for value '{value}'.", value: String),
            ErrorWithSource(3, "Error with source.", typedb_source: Box<TestError>),
        }
    }

    #[test]
    fn simple_error_code() {
        let err = TestError::SimpleError {};
        assert_eq!(err.code(), "TST1");
    }

    #[test]
    fn simple_error_component() {
        let err = TestError::SimpleError {};
        assert_eq!(err.component(), "Test");
    }

    #[test]
    fn simple_error_code_prefix() {
        let err = TestError::SimpleError {};
        assert_eq!(err.code_prefix(), "TST");
    }

    #[test]
    fn simple_error_code_number() {
        let err = TestError::SimpleError {};
        assert_eq!(err.code_number(), 1);
    }

    #[test]
    fn simple_error_variant_name() {
        let err = TestError::SimpleError {};
        assert_eq!(err.variant_name(), "SimpleError");
    }

    #[test]
    fn simple_error_format_description() {
        let err = TestError::SimpleError {};
        assert_eq!(err.format_description(), "A simple error occurred.");
    }

    #[test]
    fn error_with_value_format_description() {
        let err = TestError::ErrorWithValue { value: "test_val".to_string() };
        assert_eq!(err.format_description(), "Error for value 'test_val'.");
    }

    #[test]
    fn error_with_value_code() {
        let err = TestError::ErrorWithValue { value: "x".to_string() };
        assert_eq!(err.code(), "TST2");
        assert_eq!(err.code_number(), 2);
    }

    #[test]
    fn simple_error_has_no_source() {
        let err = TestError::SimpleError {};
        assert!(err.source_error().is_none());
        assert!(err.source_typedb_error().is_none());
    }

    #[test]
    fn error_with_typedb_source() {
        let inner = TestError::SimpleError {};
        let outer = TestError::ErrorWithSource { typedb_source: Box::new(inner) };
        assert!(outer.source_typedb_error().is_some());
        let source = outer.source_typedb_error().unwrap();
        assert_eq!(source.code(), "TST1");
    }

    #[test]
    fn format_code_and_description_no_query() {
        let err = TestError::SimpleError {};
        let formatted = err.format_code_and_description();
        assert_eq!(formatted, "[TST1] A simple error occurred.");
    }

    #[test]
    fn source_query_returns_none_for_simple_error() {
        let err = TestError::SimpleError {};
        assert!(err.source_query().is_none());
    }

    #[test]
    fn source_span_returns_none_for_simple_error() {
        let err = TestError::SimpleError {};
        assert!(err.source_span().is_none());
    }

    #[test]
    fn stack_trace_single_error() {
        let err = TestError::SimpleError {};
        let trace = err.stack_trace();
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0], "[TST1] A simple error occurred.");
    }

    #[test]
    fn stack_trace_with_source() {
        let inner = TestError::SimpleError {};
        let outer = TestError::ErrorWithSource { typedb_source: Box::new(inner) };
        let trace = outer.stack_trace();
        assert_eq!(trace.len(), 2);
        // Stack trace is reversed: root cause first
        assert_eq!(trace[0], "[TST1] A simple error occurred.");
        assert_eq!(trace[1], "[TST3] Error with source.");
    }

    #[test]
    fn root_source_typedb_error_single() {
        let err = TestError::SimpleError {};
        let root = err.root_source_typedb_error();
        assert_eq!(root.code(), "TST1");
    }

    #[test]
    fn root_source_typedb_error_nested() {
        let inner = TestError::SimpleError {};
        let outer = TestError::ErrorWithSource { typedb_source: Box::new(inner) };
        let root = outer.root_source_typedb_error();
        assert_eq!(root.code(), "TST1");
    }

    #[test]
    fn box_delegates_all_methods() {
        let err = Box::new(TestError::ErrorWithValue { value: "boxed".to_string() });
        assert_eq!(err.code(), "TST2");
        assert_eq!(err.component(), "Test");
        assert_eq!(err.code_prefix(), "TST");
        assert_eq!(err.code_number(), 2);
        assert_eq!(err.variant_name(), "ErrorWithValue");
        assert_eq!(err.format_description(), "Error for value 'boxed'.");
    }

    #[test]
    fn typedb_error_equality_by_code() {
        let a = TestError::SimpleError {};
        let b = TestError::SimpleError {};
        let a_ref: &dyn TypeDBError = &a;
        let b_ref: &dyn TypeDBError = &b;
        assert!(a_ref == b_ref);
    }

    #[test]
    fn typedb_error_inequality_different_codes() {
        let a = TestError::SimpleError {};
        let b = TestError::ErrorWithValue { value: "x".to_string() };
        let a_ref: &dyn TypeDBError = &a;
        let b_ref: &dyn TypeDBError = &b;
        assert!(a_ref != b_ref);
    }

    #[test]
    fn display_without_source() {
        let err = TestError::SimpleError {};
        let display = format!("{}", &err as &dyn TypeDBError);
        assert!(display.contains("[TST1]"));
        assert!(display.contains("A simple error occurred."));
        assert!(!display.contains("Cause"));
    }

    #[test]
    fn display_with_source() {
        let inner = TestError::SimpleError {};
        let outer = TestError::ErrorWithSource { typedb_source: Box::new(inner) };
        let display = format!("{}", &outer as &dyn TypeDBError);
        assert!(display.contains("[TST3]"));
        assert!(display.contains("Cause"));
        assert!(display.contains("[TST1]"));
    }

    #[test]
    fn debug_matches_display() {
        let err = TestError::SimpleError {};
        let debug = format!("{:?}", &err as &dyn TypeDBError);
        let display = format!("{}", &err as &dyn TypeDBError);
        assert_eq!(debug, display);
    }

    #[test]
    fn clone_preserves_error() {
        let err = TestError::ErrorWithValue { value: "cloned".to_string() };
        let cloned = err.clone();
        assert_eq!(cloned.code(), "TST2");
        assert_eq!(cloned.format_description(), "Error for value 'cloned'.");
    }
}
