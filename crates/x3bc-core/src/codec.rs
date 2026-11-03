//! The canonical binary encoding shared by the trading section and the trading receipt.
//!
//! Little-endian fixed-width integers, `u32`-length-prefixed byte strings and sequences, a tag byte
//! for every `Option` and enum. There is exactly one encoding for every value, so hashing the bytes
//! is hashing the value, and every read is bounds-checked: a truncated or oversized input is a
//! [`CodecError`], never a panic or a silently shortened value.

use alloc::string::String;
use alloc::vec::Vec;

/// Largest sequence or byte string a reader accepts. A length prefix is attacker-controlled; this
/// bounds the allocation it can request before the bytes are known to exist.
pub const MAX_LEN: usize = 1 << 20;

/// Why canonical bytes could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// The input ended before the value did.
    UnexpectedEof,
    /// A length prefix above [`MAX_LEN`] or beyond the remaining input.
    LengthOutOfRange,
    /// A tag byte that names no variant.
    InvalidTag(u8),
    /// A string that is not UTF-8.
    InvalidUtf8,
    /// Bytes left over after a complete value.
    TrailingBytes,
}

/// Appends canonical encodings to a buffer.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub fn bool(&mut self, value: bool) {
        self.bytes.push(u8::from(value));
    }

    pub fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn u128(&mut self, value: u128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn i128(&mut self, value: i128) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub fn array32(&mut self, value: &[u8; 32]) {
        self.bytes.extend_from_slice(value);
    }

    pub fn array64(&mut self, value: &[u8; 64]) {
        self.bytes.extend_from_slice(value);
    }

    /// A length prefix. Lengths are bounded by [`MAX_LEN`] on read; a writer asked to emit a longer
    /// sequence emits it anyway and the reader refuses it, so an oversized value cannot round-trip.
    pub fn length(&mut self, len: usize) {
        self.u32(u32::try_from(len).unwrap_or(u32::MAX));
    }

    pub fn bytes(&mut self, value: &[u8]) {
        self.length(value.len());
        self.bytes.extend_from_slice(value);
    }

    pub fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub fn option<T>(&mut self, value: &Option<T>, mut write: impl FnMut(&mut Self, &T)) {
        match value {
            None => self.u8(0),
            Some(inner) => {
                self.u8(1);
                write(self, inner);
            }
        }
    }

    pub fn seq<T>(&mut self, items: &[T], mut write: impl FnMut(&mut Self, &T)) {
        self.length(items.len());
        for item in items {
            write(self, item);
        }
    }
}

/// Reads canonical encodings from a slice, refusing anything malformed.
pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    /// Refuse bytes left over after a complete value.
    pub fn finish(&self) -> Result<(), CodecError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes)
        }
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(CodecError::UnexpectedEof)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(CodecError::UnexpectedEof)?;
        self.offset = end;
        Ok(slice)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.fixed::<1>()?[0])
    }

    pub fn bool(&mut self) -> Result<bool, CodecError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(CodecError::InvalidTag(other)),
        }
    }

    pub fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.fixed()?))
    }

    pub fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    pub fn u64(&mut self) -> Result<u64, CodecError> {
        Ok(u64::from_le_bytes(self.fixed()?))
    }

    pub fn u128(&mut self) -> Result<u128, CodecError> {
        Ok(u128::from_le_bytes(self.fixed()?))
    }

    pub fn i128(&mut self) -> Result<i128, CodecError> {
        Ok(i128::from_le_bytes(self.fixed()?))
    }

    pub fn array32(&mut self) -> Result<[u8; 32], CodecError> {
        self.fixed()
    }

    pub fn array64(&mut self) -> Result<[u8; 64], CodecError> {
        self.fixed()
    }

    pub fn length(&mut self) -> Result<usize, CodecError> {
        let len = self.u32()? as usize;
        if len > MAX_LEN || len > self.remaining() {
            // A sequence of zero-sized elements cannot exist in this encoding (every element is at
            // least one byte), so a length above the remaining input is malformed, not merely large.
            return Err(CodecError::LengthOutOfRange);
        }
        Ok(len)
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>, CodecError> {
        let len = self.length()?;
        Ok(self.take(len)?.to_vec())
    }

    pub fn string(&mut self) -> Result<String, CodecError> {
        String::from_utf8(self.bytes()?).map_err(|_| CodecError::InvalidUtf8)
    }

    pub fn option<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Option<T>, CodecError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(read(self)?)),
            other => Err(CodecError::InvalidTag(other)),
        }
    }

    pub fn seq<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, CodecError>,
    ) -> Result<Vec<T>, CodecError> {
        let len = self.length()?;
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(read(self)?);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn values_round_trip_and_truncation_is_refused() {
        let mut w = Writer::new();
        w.u8(7);
        w.u128(u128::MAX - 1);
        w.str("x3");
        w.option(&Some(5u16), |w, v| w.u16(*v));
        w.seq(&[1u32, 2, 3], |w, v| w.u32(*v));
        let bytes = w.finish();

        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8(), Ok(7));
        assert_eq!(r.u128(), Ok(u128::MAX - 1));
        assert_eq!(r.string().as_deref(), Ok("x3"));
        assert_eq!(r.option(|r| r.u16()), Ok(Some(5)));
        assert_eq!(r.seq(|r| r.u32()), Ok(vec![1, 2, 3]));
        assert_eq!(r.finish(), Ok(()));

        for cut in 0..bytes.len() {
            let mut r = Reader::new(&bytes[..cut]);
            let result = (|| -> Result<(), CodecError> {
                r.u8()?;
                r.u128()?;
                r.string()?;
                r.option(|r| r.u16())?;
                r.seq(|r| r.u32())?;
                Ok(())
            })();
            assert!(result.is_err(), "truncation at {cut} must be refused");
        }
    }

    #[test]
    fn a_hostile_length_prefix_cannot_allocate() {
        let mut bytes = vec![];
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Reader::new(&bytes).bytes(),
            Err(CodecError::LengthOutOfRange)
        );
    }

    #[test]
    fn bad_tags_are_refused() {
        assert_eq!(Reader::new(&[2]).bool(), Err(CodecError::InvalidTag(2)));
        assert_eq!(
            Reader::new(&[9]).option(|r| r.u8()),
            Err(CodecError::InvalidTag(9))
        );
    }
}
