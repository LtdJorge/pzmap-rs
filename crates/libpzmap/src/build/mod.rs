//! The game builds read by this crate, and the cell, block and layer sizes of each build.
//!
//! The map files do not say how big their cells and blocks are, so the parsers need to know which
//! build wrote them. A Build 41 cell is 30×30 blocks of 10×10 squares. A Build 42 cell is 32×32
//! blocks of 8×8 squares.
//!
//! When the build is known at compile time, pass [`B41`] or [`B42`] as a type parameter. The
//! build then becomes part of the type, so a `Cell<B41>` cannot be used where a `Cell<B42>` is
//! expected. When the build is chosen at runtime, use [`Build`].

use crate::{
    error::Result,
    input::Input,
};
use std::fmt::Debug;

mod versions;

pub use versions::{
    B41,
    B42,
};

pub(crate) mod sealed {
    /// Keeps other crates from implementing [`super::VersionMetadata`].
    ///
    /// It also holds file layout details that are internal to the parsers.
    pub trait Sealed {
        /// Whether a padding null byte follows the header's string table.
        const NULL_AFTER_STRING_TABLE: bool;
        /// How this build stores its layer range.
        const LAYER_ENCODING: LayerEncoding;
    }

    /// How a build encodes the layer range in its `.lotheader`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum LayerEncoding {
        /// One `i32`: one more than the highest layer, so it can be used as `max_layer` as is.
        ///
        /// The minimum is always
        /// [`VersionMetadata::MIN_LAYER`](super::VersionMetadata::MIN_LAYER).
        MaxOnly,
        /// Two `i32`s: the lowest layer, then the highest layer. Both are inclusive.
        MinAndMax,
    }
}

/// The sizes and version numbers differing between game builds.
///
/// Only [`B41`] and [`B42`] implement it. Other crates cannot add implementations.
pub trait VersionMetadata: sealed::Sealed + Debug + 'static {
    /// The value written by this build into the `LOTH` and `LOTP` version fields in the files.
    const VERSION_NUMBER: u32;
    /// Width in blocks of one edge of a cell. The total number of blocks in a cell is this number
    /// squared.
    const CELL_SIZE_IN_BLOCKS: usize;
    /// Width in squares of one edge of a block. The total number of squares in a block is this
    /// number squared.
    const BLOCK_SIZE_IN_SQUARES: usize;
    /// The total number of squares in a cell (not a block). CSIB * BSIS.
    const CELL_SIZE_IN_SQUARES: i32;
    /// The lowest layer (height) this build can store.
    const MIN_LAYER: i32;
    /// The highest layer (height) this build can store + 1.
    const MAX_LAYER: i32;
    /// The build's display name.
    const NAME: &'static str;
}

/// A game build chosen at runtime.
///
/// Use this when the build comes from a command-line argument or a language binding, by passing it
/// to [`AnyLotHeader`](crate::lotheader::AnyLotHeader) or [`AnyCell`](crate::lotpack::AnyCell).
///
/// ```
/// use libpzmap::Build;
///
/// let build = Build::from_version_number(1);
/// assert_eq!(build, Some(Build::B42));
/// assert_eq!(Build::B42.cell_size_in_squares(), 256);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Build {
    /// Project Zomboid Build 41.
    B41,
    /// Project Zomboid Build 42.
    B42,
}

/// Calls `$body` with `$V` bound to the type implementing the selected build.
macro_rules! dispatch {
    ($build:expr, |$V:ident| $body:expr) => {
        match $build {
            $crate::build::Build::B41 => {
                type $V = $crate::build::B41;
                $body
            }
            $crate::build::Build::B42 => {
                type $V = $crate::build::B42;
                $body
            }
        }
    };
}

pub(crate) use dispatch;

impl Build {
    /// Every supported build, in release order.
    pub const ALL: &'static [Build] = &[Build::B41, Build::B42];

    /// Try to match a version number coming from the file header to a build.
    pub fn from_version_number(version: u32) -> Option<Build> {
        Build::ALL
            .iter()
            .copied()
            .find(|build| build.version_number() == version)
    }

    /// The build's display name.
    pub fn name(self) -> &'static str {
        dispatch!(self, |V| V::NAME)
    }

    /// The value written by this build into the `LOTH` and `LOTP` version fields in the files.
    pub fn version_number(self) -> u32 {
        dispatch!(self, |V| V::VERSION_NUMBER)
    }

    /// Width in blocks of one edge of a cell. The total number of blocks in a cell is this number
    /// squared.
    pub fn cell_size_in_blocks(self) -> usize {
        dispatch!(self, |V| V::CELL_SIZE_IN_BLOCKS)
    }

    /// Width in squares of one edge of a block. The total number of squares in a block is this
    /// number squared.
    pub fn block_size_in_squares(self) -> usize {
        dispatch!(self, |V| V::BLOCK_SIZE_IN_SQUARES)
    }

    /// The total number of squares in a cell (not a block). CSIB * BSIS.
    pub fn cell_size_in_squares(self) -> i32 {
        dispatch!(self, |V| V::CELL_SIZE_IN_SQUARES)
    }

    /// The lowest layer (height) this build can store.
    pub fn min_layer(self) -> i32 {
        dispatch!(self, |V| V::MIN_LAYER)
    }

    /// The highest layer (height) this build can store + 1.
    pub fn max_layer(self) -> i32 {
        dispatch!(self, |V| V::MAX_LAYER)
    }
}

impl std::fmt::Display for Build {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Converts a block or cell size to `i64` for range checks.
///
/// Asserts that the value fits, so the cast cannot wrap.
///
/// Put it inside a `const { widen(...) }` block to catch wrong constants at compile time, just
/// calling it will delegate the check to panicking at runtime.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
pub(crate) const fn widen(value: usize) -> i64 {
    assert!(value <= i64::MAX as usize, "geometry constant is absurd");
    value as i64
}

/// Runs `parse` `count` times and collects the results.
///
/// This allocates memory for `count` elements up front. If `count` comes from an integer in a file,
/// run it through [`Input::checked_count`] before passing it here.
#[inline]
pub(crate) fn repeat<'a, T, F>(input: &mut Input<'a>, count: usize, mut parse: F) -> Result<Vec<T>>
where
    F: FnMut(&mut Input<'a>) -> Result<T>,
{
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(parse(input)?);
    }
    Ok(out)
}
