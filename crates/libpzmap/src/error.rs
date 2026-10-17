//! The error returned for malformed or unsupported files.
//!
//! Every parser returns [`ParseError`], except the translation readers, which return
//! [`TranslationError`](crate::sidecar::translation::TranslationError). A `ParseError` records
//! which format was being read and the byte offset where reading failed. It is `'static` and
//! `Send`, so it can be boxed or sent to another thread.

use std::fmt::{
    self,
    Display,
    Formatter,
};

/// The file format that was being read when an error occurred. `non_exhaustive` because future
/// builds could add new formats.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// A `.lotheader` cell header.
    LotHeader,
    /// A `world_*.lotpack` cell body.
    LotPack,
    /// A `.pack` texture atlas.
    TexturePack,
    /// A Lua table literal such as `objects.lua` or `regions.lua`.
    Lua,
    /// A `streets.xml` street index.
    Streets,
    /// A `.tiles` tile definitions file.
    TileDefs,
}

impl Format {
    /// The format's name, as it appears in error messages.
    pub const fn name(self) -> &'static str {
        match self {
            Format::LotHeader => "lotheader",
            Format::LotPack => "lotpack",
            Format::TexturePack => "texture pack",
            Format::Lua => "lua",
            Format::Streets => "streets.xml",
            Format::TileDefs => "tile definitions",
        }
    }
}

impl Display for Format {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An error in a malformed or unsupported file.
///
/// This type only describes problems with the file being read. Bugs in this crate must be caught by
/// assertions instead.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseError {
    /// The format being read.
    pub format: Format,
    /// The byte offset where the error was found.
    pub at: usize,
    /// What went wrong.
    pub kind: ParseErrorKind,
}

/// The specific failure behind a [`ParseError`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    /// A four-byte magic marker did not match what the format requires.
    Magic {
        /// The marker required by the format
        expected: &'static str,
        /// The four bytes actually found
        found: [u8; 4],
    },

    /// A read ran past the end of the buffer.
    UnexpectedEof {
        /// Number of bytes requested
        needed: usize,
        /// Total length of the buffer
        len: usize,
    },

    /// An unsupported game build or file-header version.
    Version {
        /// What the version belongs to, such as `"Build 42"` or `"PZPK envelope"`.
        what: &'static str,
        /// The version actually found
        found: u32,
    },

    /// A string field was not valid UTF-8.
    Utf8 {
        /// The name of the field being read
        field: &'static str,
    },

    /// A numeric field fell outside the range the format allows.
    OutOfRange {
        /// The name of the field being read.
        field: &'static str,
        /// The invalid value. It is an `i64` so that it can hold any signed or unsigned 32-bit
        /// field.
        value: i64,
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },

    /// A count read from the file that needs more bytes than the file has left.
    ///
    /// The parsers check each count this way before allocating memory for it.
    ImplausibleCount {
        /// Name of the count field.
        field: &'static str,
        /// The count found.
        count: u64,
        /// The fewest bytes that many elements could occupy.
        needed: u64,
        /// Bytes left in the buffer.
        available: usize,
    },

    /// An offset stored in the file points past the end of the file.
    OffsetOutOfBounds {
        /// What the offset points to, such as `"block"`.
        what: &'static str,
        /// The offset found.
        offset: u64,
        /// Total length of the buffer.
        len: usize,
    },

    /// The file had bytes left over after its last field was read.
    TrailingBytes {
        /// How many bytes are left over.
        len: usize,
    },

    /// Text that does not follow the format's grammar.
    ///
    /// Only the text formats report this: Lua tables and `streets.xml`.
    Syntax {
        /// What the grammar allows at the error's offset.
        expected: &'static str,
    },
}

impl ParseError {
    /// An error of `kind` while reading `format` at byte `at`.
    pub fn new(format: Format, at: usize, kind: ParseErrorKind) -> Self {
        ParseError { format, at, kind }
    }
}

impl std::error::Error for ParseError {}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let ParseError { format, at, kind } = self;
        match kind {
            ParseErrorKind::Magic { expected, found } => write!(
                f,
                "{format}: expected magic {expected:?} at byte {at}, found {:?}",
                Ascii(found)
            ),
            ParseErrorKind::UnexpectedEof { needed, len } => write!(
                f,
                "{format}: unexpected end of input at byte {at}, needed {needed} more byte(s) of {len}"
            ),
            ParseErrorKind::Version { what, found } => {
                write!(
                    f,
                    "{format}: unsupported {what} version {found} at byte {at}"
                )
            }
            ParseErrorKind::Utf8 { field } => {
                write!(f, "{format}: invalid UTF-8 in {field} at byte {at}")
            }
            ParseErrorKind::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(
                f,
                "{format}: {field} = {value} at byte {at} is outside {min}..={max}"
            ),
            ParseErrorKind::ImplausibleCount {
                field,
                count,
                needed,
                available,
            } => write!(
                f,
                "{format}: {field} = {count} at byte {at} needs {needed} byte(s), but only {available} remain"
            ),
            ParseErrorKind::OffsetOutOfBounds { what, offset, len } => write!(
                f,
                "{format}: {what} offset {offset} at byte {at} is past the end of the file ({len} bytes)"
            ),
            ParseErrorKind::TrailingBytes { len } => {
                write!(f, "{format}: {len} trailing byte(s) after byte {at}")
            }
            ParseErrorKind::Syntax { expected } => {
                write!(f, "{format}: expected {expected} at byte {at}")
            }
        }
    }
}

/// Shorthand for a parse result.
pub type Result<T> = core::result::Result<T, ParseError>;

/// Formats a magic marker as a quoted string, such as `"LOTP"`, escaping non-printable bytes.
struct Ascii<'a>(&'a [u8]);

impl fmt::Debug for Ascii<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for &byte in self.0 {
            match byte {
                0x20..=0x7e => write!(f, "{}", byte as char)?,
                _ => write!(f, "\\x{byte:02x}")?,
            }
        }
        f.write_str("\"")
    }
}
