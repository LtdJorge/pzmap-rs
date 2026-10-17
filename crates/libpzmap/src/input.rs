//! A cursor that reads through a byte buffer.
//!
//! Since the cursor tracks its byte offset, every error returned contains the location in the file
//! where it happened. Every read checks the end of the buffer first.

use crate::error::{
    Format,
    ParseError,
    ParseErrorKind,
    Result,
};

/// A read position in a borrowed buffer and the file format to name in errors.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Input<'a> {
    whole: &'a [u8],
    pos: usize,
    format: Format,
}

impl<'a> Input<'a> {
    /// A cursor at the start of `bytes`.
    #[inline]
    pub(crate) fn new(bytes: &'a [u8], format: Format) -> Self {
        Input {
            whole: bytes,
            pos: 0,
            format,
        }
    }

    /// The cursor's byte offset from the start of the buffer.
    #[inline]
    pub(crate) fn offset(&self) -> usize {
        self.pos
    }

    /// The length of the whole buffer, including bytes already read.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.whole.len()
    }

    /// Bytes not yet read.
    #[inline]
    pub(crate) fn remaining(&self) -> &'a [u8] {
        // `pos` never passes the end of the buffer, so this never falls back to empty.
        self.whole.get(self.pos..).unwrap_or(&[])
    }

    /// Number of bytes not yet read.
    #[inline]
    pub(crate) fn remaining_len(&self) -> usize {
        self.whole.len().saturating_sub(self.pos)
    }

    /// Whether every byte has been read.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.remaining_len() == 0
    }

    /// Returns the next `n` bytes and advances the cursor.
    ///
    /// If fewer than `n` bytes remain, it returns `UnexpectedEof` without changing the cursor.
    #[inline]
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let eof = || {
            self.error(
                self.pos,
                ParseErrorKind::UnexpectedEof {
                    needed: n,
                    len: self.whole.len(),
                },
            )
        };
        let end = self.pos.checked_add(n).ok_or_else(eof)?;
        let bytes = self.whole.get(self.pos..end).ok_or_else(eof)?;

        self.pos = end;
        Ok(bytes)
    }

    /// Skips `n` bytes.
    #[inline]
    pub(crate) fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    /// Reads a little-endian `i32`.
    #[inline]
    pub(crate) fn i32_le(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array4()?))
    }

    /// Reads a little-endian `u32`.
    #[inline]
    pub(crate) fn u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array4()?))
    }

    /// Reads four bytes as an array.
    ///
    /// Since `take(4)` returns exactly four bytes or an error, `copy_from_slice` cannot panic.
    #[inline]
    fn array4(&mut self) -> Result<[u8; 4]> {
        let bytes = self.take(4)?;
        let mut out = [0_u8; 4];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    /// Reads a four-byte magic marker and checks that it is `expected`.
    pub(crate) fn magic(&mut self, expected: &'static str) -> Result<()> {
        let at = self.pos;
        let found = self.array4()?;

        if found == expected.as_bytes() {
            return Ok(());
        }

        Err(self.error(at, ParseErrorKind::Magic { expected, found }))
    }

    /// Reads an optional `magic` marker and the `u32` version that follows it.
    ///
    /// If the marker is absent, it returns version 0 without reading anything.
    pub(crate) fn optional_magic_version(&mut self, magic: &'static str) -> Result<u32> {
        if !self.starts_with(magic.as_bytes()) {
            return Ok(0);
        }

        self.magic(magic)?;
        self.u32_le()
    }

    /// Whether the unread bytes start with `prefix`. Reads nothing.
    pub(crate) fn starts_with(&self, prefix: &[u8]) -> bool {
        self.remaining().starts_with(prefix)
    }

    /// Reads a UTF-8 string preceded by its length as a `u32`.
    pub(crate) fn tlv_string(&mut self, field: &'static str) -> Result<&'a str> {
        let length = self.u32_le()?;
        let at = self.pos;
        let bytes = self.take(length as usize)?;

        self.utf8(bytes, at, field)
    }

    /// Reads a UTF-8 string up to the next `\n`, then skips the newline.
    pub(crate) fn line(&mut self, field: &'static str) -> Result<&'a str> {
        let at = self.pos;
        let end = self
            .remaining()
            .iter()
            .position(|&b| b == b'\n')
            .ok_or(self.error(
                at,
                ParseErrorKind::UnexpectedEof {
                    needed: 1,
                    len: self.whole.len(),
                },
            ))?;

        let bytes = self.take(end)?;
        self.skip(1)?;

        self.utf8(bytes, at, field)
    }

    /// Checks that `bytes`, read from offset `at`, are UTF-8.
    fn utf8(&self, bytes: &'a [u8], at: usize, field: &'static str) -> Result<&'a str> {
        std::str::from_utf8(bytes).map_err(|_| self.error(at, ParseErrorKind::Utf8 { field }))
    }

    /// An error of `kind` at byte `at` and carrying the cursor's format.
    #[inline]
    fn error(&self, at: usize, kind: ParseErrorKind) -> ParseError {
        ParseError::new(self.format, at, kind)
    }

    /// Reads everything before the next `needle`, then skips the needle.
    ///
    /// It is an error if `needle` is missing or comes first, so the result is never empty.
    pub(crate) fn take_until_and_skip(&mut self, needle: &[u8]) -> Result<&'a [u8]> {
        let at = self.pos;
        let found = self
            .remaining()
            .windows(needle.len())
            .position(|window| window == needle)
            .filter(|&index| index > 0)
            .ok_or(self.error(
                at,
                ParseErrorKind::UnexpectedEof {
                    needed: needle.len(),
                    len: self.whole.len(),
                },
            ))?;

        let bytes = self.take(found)?;
        self.skip(needle.len())?;
        Ok(bytes)
    }

    /// A new cursor at byte `offset` from the start of the buffer.
    ///
    /// Returns an error if `offset` is past the end of the buffer.
    pub(crate) fn at(&self, offset: u64, what: &'static str) -> Result<Input<'a>> {
        let pos = usize::try_from(offset)
            .ok()
            .filter(|&pos| pos <= self.whole.len())
            .ok_or(self.error(
                self.pos,
                ParseErrorKind::OffsetOutOfBounds {
                    what,
                    offset,
                    len: self.whole.len(),
                },
            ))?;

        Ok(Input {
            whole: self.whole,
            pos,
            format: self.format,
        })
    }

    /// Checks a count read from the file before it is used to allocate memory.
    ///
    /// `min_bytes_each` is the smallest size one element can have in the file. The count is
    /// rejected if it is negative, or if that many elements could not fit in the bytes left.
    /// This way a corrupt file cannot make the parser allocate far more memory than its own size.
    pub(crate) fn checked_count(
        &self,
        field: &'static str,
        count: i64,
        min_bytes_each: usize,
    ) -> Result<usize> {
        let at = self.pos;
        let available = self.remaining_len();

        let count = u64::try_from(count).map_err(|_| {
            self.error(
                at,
                ParseErrorKind::OutOfRange {
                    field,
                    value: count,
                    min: 0,
                    max: i64::MAX,
                },
            )
        })?;

        let needed = count.saturating_mul(min_bytes_each as u64);
        let implausible = || {
            self.error(
                at,
                ParseErrorKind::ImplausibleCount {
                    field,
                    count,
                    needed,
                    available,
                },
            )
        };
        if needed > available as u64 {
            return Err(implausible());
        }

        // On 32-bit targets a plausible count can still be too large for `usize`.
        usize::try_from(count).map_err(|_| implausible())
    }

    /// Checks that a value read from the file is in `min..=max`.
    ///
    /// Takes an `i64` so that it can check any signed or unsigned 32-bit field.
    pub(crate) fn require_range(
        &self,
        field: &'static str,
        value: i64,
        min: i64,
        max: i64,
    ) -> Result<i64> {
        if value >= min && value <= max {
            return Ok(value);
        }

        Err(self.error(
            self.pos,
            ParseErrorKind::OutOfRange {
                field,
                value,
                min,
                max,
            },
        ))
    }

    /// Checks that a value read from the file is not negative, and returns it as a `usize`.
    pub(crate) fn require_count(&self, field: &'static str, value: i64) -> Result<usize> {
        usize::try_from(value).map_err(|_| {
            self.error(
                self.pos,
                ParseErrorKind::OutOfRange {
                    field,
                    value,
                    min: 0,
                    max: i64::MAX,
                },
            )
        })
    }

    /// Returns an error if any bytes are left unread.
    pub(crate) fn expect_eof(&self) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }

        Err(self.error(
            self.pos,
            ParseErrorKind::TrailingBytes {
                len: self.remaining_len(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(bytes: &[u8]) -> Input<'_> {
        Input::new(bytes, Format::LotPack)
    }

    #[test]
    fn take_does_not_overflow_on_absurd_lengths() {
        let mut cursor = input(&[1, 2, 3]);
        assert!(matches!(
            cursor.take(usize::MAX),
            Err(ParseError {
                kind: ParseErrorKind::UnexpectedEof { .. },
                ..
            })
        ));
    }

    #[test]
    fn magic_mismatch_names_both_sides() {
        let mut cursor = input(b"LOTP");
        assert!(cursor.magic("LOTP").is_ok());

        let mut cursor = input(b"XXXX");
        let err = cursor.magic("LOTH").unwrap_err();
        assert!(matches!(
            err,
            ParseError {
                at: 0,
                kind: ParseErrorKind::Magic { .. },
                ..
            }
        ));
        // Magic markers are displayed as ASCII in the error message.
        assert!(err.to_string().contains("\"XXXX\""), "{err}");
    }

    #[test]
    fn implausible_counts_are_rejected_before_allocating() {
        let cursor = input(&[0; 16]);
        assert!(matches!(
            cursor.checked_count("num_strings", 1_000_000, 4),
            Err(ParseError {
                kind: ParseErrorKind::ImplausibleCount { .. },
                ..
            })
        ));
        assert_eq!(cursor.checked_count("num_strings", 4, 4).unwrap(), 4);
    }

    #[test]
    fn negative_counts_are_rejected() {
        let cursor = input(&[0; 16]);
        assert!(matches!(
            cursor.checked_count("count", -1, 4),
            Err(ParseError {
                kind: ParseErrorKind::OutOfRange { .. },
                ..
            })
        ));
    }

    #[test]
    fn absolute_seek_is_bounds_checked() {
        let cursor = input(&[0; 8]);
        assert_eq!(cursor.at(8, "block").unwrap().offset(), 8);
        assert!(matches!(
            cursor.at(9, "block"),
            Err(ParseError {
                kind: ParseErrorKind::OffsetOutOfBounds { .. },
                ..
            })
        ));
        assert!(matches!(
            cursor.at(u64::MAX, "block"),
            Err(ParseError {
                kind: ParseErrorKind::OffsetOutOfBounds { .. },
                ..
            })
        ));
    }

    #[test]
    fn unterminated_lines_are_an_error() {
        let mut cursor = input(b"kitchen");
        assert!(matches!(
            cursor.line("room"),
            Err(ParseError {
                kind: ParseErrorKind::UnexpectedEof { .. },
                ..
            })
        ));
    }

    #[test]
    fn invalid_utf8_names_the_field() {
        let mut cursor = input(&[0xff, 0xfe, b'\n']);
        assert!(matches!(
            cursor.line("room"),
            Err(ParseError {
                kind: ParseErrorKind::Utf8 { field: "room", .. },
                ..
            })
        ));
    }
}
