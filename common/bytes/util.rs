/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use std::{
    borrow::Cow,
    fmt::{self, Write},
};

use base64::Engine;

pub const KB: u64 = 1024;
pub const MB: u64 = KB * KB;
pub const GB: u64 = MB * KB;

// TODO: this needs to be optimised using bigger strides than a single byte!
///
/// Performs a big-endian +1 operation that errors on overflow
///
pub fn increment(bytes: &mut [u8]) -> Result<(), BytesError> {
    for byte in bytes.iter_mut().rev() {
        let (val, overflow) = byte.overflowing_add(1);
        *byte = val;
        if !overflow {
            return Ok(());
        }
    }
    Err(BytesError { kind: BytesErrorKind::IncrementOverflow {} })
}

///
/// Performs a 'const' big-endian +1 operation that panics on overflow
///
pub const fn increment_fixed<const SIZE: usize>(mut bytes: [u8; SIZE]) -> [u8; SIZE] {
    let mut index = SIZE;
    while index > 0 {
        let (val, overflow) = bytes[index - 1].overflowing_add(1);
        bytes[index - 1] = val;
        if !overflow {
            return bytes;
        }
        index -= 1;
    }
    panic!("Overflow while incrementing array")
}

#[derive(Debug)]
pub struct BytesError {
    pub kind: BytesErrorKind,
}

#[derive(Debug)]
pub enum BytesErrorKind {
    IncrementOverflow {},
}

impl fmt::Display for BytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            BytesErrorKind::IncrementOverflow {} => {
                write!(f, "BytesError::IncrementOverflow")
            }
        }
    }
}

#[derive(Clone)]
pub struct HexBytesFormatter<'a>(Cow<'a, [u8]>);

impl<'a> HexBytesFormatter<'a> {
    pub fn owned(bytes: Vec<u8>) -> Self {
        Self(Cow::Owned(bytes))
    }

    pub fn borrowed(bytes: &'a [u8]) -> Self {
        Self(Cow::Borrowed(bytes))
    }

    pub fn format_iid(&self) -> String {
        const PREFIX: &'static str = "0x";
        let mut result = String::with_capacity(PREFIX.len() + self.0.len() * 2);
        result.push_str(PREFIX);
        self.0.iter().for_each(|byte| write!(result, "{byte:02x}").expect("Expected IID formatting"));
        result
    }
}

impl fmt::Display for HexBytesFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl fmt::Debug for HexBytesFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const GROUP: usize = 2;
        const BREAK: usize = 16;
        f.write_str("[")?;
        if f.alternate() {
            f.write_str("\n    ")?;
        }
        for (i, byte) in self.0.iter().enumerate() {
            write!(f, "{:02X}", byte)?;
            if i + 1 < self.0.len() {
                if f.alternate() && (i + 1) % BREAK == 0 {
                    f.write_str("\n    ")?;
                } else if (i + 1) % GROUP == 0 {
                    f.write_char(' ')?;
                }
            }
        }
        if f.alternate() {
            f.write_char('\n')?;
        }
        f.write_char(']')?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct Base64Formatter<'a>(Cow<'a, [u8]>);

impl<'a> Base64Formatter<'a> {
    pub fn owned(bytes: Vec<u8>) -> Self {
        Self(Cow::Owned(bytes))
    }

    pub fn borrowed(bytes: &'a [u8]) -> Self {
        Self(Cow::Borrowed(bytes))
    }

    pub fn format(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.0)
    }
}

impl fmt::Display for Base64Formatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl fmt::Debug for Base64Formatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_single_byte_no_carry() {
        let mut bytes = [0u8, 0, 0, 1];
        increment(&mut bytes).unwrap();
        assert_eq!(bytes, [0, 0, 0, 2]);
    }

    #[test]
    fn increment_with_carry() {
        let mut bytes = [0u8, 0, 0, 255];
        increment(&mut bytes).unwrap();
        assert_eq!(bytes, [0, 0, 1, 0]);
    }

    #[test]
    fn increment_with_multi_carry() {
        let mut bytes = [0u8, 0, 255, 255];
        increment(&mut bytes).unwrap();
        assert_eq!(bytes, [0, 1, 0, 0]);
    }

    #[test]
    fn increment_overflow_returns_error() {
        let mut bytes = [255u8, 255, 255, 255];
        let result = increment(&mut bytes);
        assert!(result.is_err());
    }

    #[test]
    fn increment_empty_slice_overflows() {
        let mut bytes: [u8; 0] = [];
        let result = increment(&mut bytes);
        assert!(result.is_err());
    }

    #[test]
    fn increment_single_byte() {
        let mut bytes = [0u8];
        increment(&mut bytes).unwrap();
        assert_eq!(bytes, [1]);
    }

    #[test]
    fn increment_fixed_simple() {
        let result = increment_fixed([0u8, 0, 0, 1]);
        assert_eq!(result, [0, 0, 0, 2]);
    }

    #[test]
    fn increment_fixed_with_carry() {
        let result = increment_fixed([0u8, 0, 0, 255]);
        assert_eq!(result, [0, 0, 1, 0]);
    }

    #[test]
    fn increment_fixed_with_multi_carry() {
        let result = increment_fixed([0u8, 0, 255, 255]);
        assert_eq!(result, [0, 1, 0, 0]);
    }

    #[test]
    #[should_panic(expected = "Overflow")]
    fn increment_fixed_overflow_panics() {
        let _ = increment_fixed([255u8, 255, 255, 255]);
    }

    #[test]
    fn hex_formatter_format_iid_empty() {
        let formatter = HexBytesFormatter::borrowed(&[]);
        assert_eq!(formatter.format_iid(), "0x");
    }

    #[test]
    fn hex_formatter_format_iid_bytes() {
        let formatter = HexBytesFormatter::borrowed(&[0x68, 0x65, 0x6c, 0x6c, 0x6f]);
        assert_eq!(formatter.format_iid(), "0x68656c6c6f");
    }

    #[test]
    fn hex_formatter_format_iid_leading_zeros() {
        let formatter = HexBytesFormatter::borrowed(&[0x00, 0x01, 0x0a]);
        assert_eq!(formatter.format_iid(), "0x00010a");
    }

    #[test]
    fn hex_formatter_owned_and_borrowed_produce_same_output() {
        let bytes = vec![0xab, 0xcd, 0xef];
        let owned = HexBytesFormatter::owned(bytes.clone());
        let borrowed = HexBytesFormatter::borrowed(&bytes);
        assert_eq!(owned.format_iid(), borrowed.format_iid());
    }

    #[test]
    fn hex_formatter_debug_output() {
        let formatter = HexBytesFormatter::borrowed(&[0xaa, 0xbb, 0xcc, 0xdd]);
        let debug = format!("{:?}", formatter);
        assert_eq!(debug, "[AABB CCDD]");
    }

    #[test]
    fn hex_formatter_debug_single_byte() {
        let formatter = HexBytesFormatter::borrowed(&[0x0f]);
        let debug = format!("{:?}", formatter);
        assert_eq!(debug, "[0F]");
    }

    #[test]
    fn base64_formatter_encode() {
        let formatter = Base64Formatter::borrowed(b"hello");
        assert_eq!(formatter.format(), "aGVsbG8=");
    }

    #[test]
    fn base64_formatter_empty() {
        let formatter = Base64Formatter::borrowed(&[]);
        assert_eq!(formatter.format(), "");
    }

    #[test]
    fn base64_formatter_owned_and_borrowed_match() {
        let data = vec![1, 2, 3, 4, 5];
        let owned = Base64Formatter::owned(data.clone());
        let borrowed = Base64Formatter::borrowed(&data);
        assert_eq!(owned.format(), borrowed.format());
    }

    #[test]
    fn base64_formatter_debug_matches_format() {
        let formatter = Base64Formatter::borrowed(b"test");
        let debug_output = format!("{:?}", formatter);
        assert_eq!(debug_output, formatter.format());
    }

    #[test]
    fn constants_are_correct() {
        assert_eq!(KB, 1024);
        assert_eq!(MB, 1024 * 1024);
        assert_eq!(GB, 1024 * 1024 * 1024);
    }

    #[test]
    fn bytes_error_display() {
        let err = BytesError { kind: BytesErrorKind::IncrementOverflow {} };
        let msg = format!("{}", err);
        assert!(msg.contains("IncrementOverflow"));
    }
}
