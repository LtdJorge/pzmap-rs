//! Cell headers (`.lotheader` files).
//!
//! A lotheader lists every tile name in the cell, the cell's rooms and buildings, and the
//! zombie population of each block. It does not say where tiles are placed. That is in the
//! cell's `world_*.lotpack`, which refers to tiles by their position in this file's list of
//! names, so the two files are read together.
//!
//! Parse one with [`LotHeader::parse`], or with [`AnyLotHeader::parse`] when the build needs to be
//! chosen at runtime.

mod parse;

use crate::{
    build::{
        B41,
        B42,
        Build,
        VersionMetadata,
    },
    error::Result,
};
use std::marker::PhantomData;
use yoke::{
    Yoke,
    Yokeable,
};

/// A parsed `.lotheader`.
///
/// Tile and room names borrow the input buffer. To keep the header and its buffer together, use
/// [`AnyLotHeader`]. `V` is the game build, [`B41`] or [`B42`].
#[derive(Clone, Debug, Yokeable)]
#[non_exhaustive]
pub struct LotHeader<'inp, V: VersionMetadata> {
    /// The cell's X position in the cell grid, as given in the file name. It can be negative.
    pub x: i32,
    /// The cell's Y position in the cell grid, as given in the file name. It can be negative.
    pub y: i32,
    /// The version number in the file's header, equal to [`VersionMetadata::VERSION_NUMBER`].
    pub version: u32,
    /// Every distinct tile name used by this cell.
    ///
    /// The lotpack stores each tile as an index into this list, so you need it to know which
    /// tiles a square holds.
    pub strings: Box<[&'inp str]>,
    /// The lowest layer this cell stores.
    pub min_layer: i32,
    /// The highest layer this cell stores + 1.
    pub max_layer: i32,
    /// The rooms in this cell, with positions in world coordinates.
    pub rooms: Box<[Room<'inp>]>,
    /// The buildings in this cell. Each one lists the rooms it is made of.
    pub buildings: Box<[Building]>,
    /// The zombie population of each block, one byte per block.
    ///
    /// The value for the block at (`block_x`, `block_y`) is at index
    /// `block_x * V::CELL_SIZE_IN_BLOCKS + block_y`. [`LotHeader::zpop_at`] does this lookup.
    pub zpop: Vec<u8>,
    /// The number of bytes left over after `zpop`, the last field in the file.
    ///
    /// This is normally zero. Any other value suggests that the file has a layout this parser does
    /// not know. The parser does not treat it as an error. Check it to reject such files.
    pub trailing_bytes: usize,
    pub(crate) marker: PhantomData<fn() -> V>,
}

impl<V: VersionMetadata> LotHeader<'_, V> {
    /// The zombie population of one block, or `None` if either coordinate is out of range.
    pub fn zpop_at(&self, block_x: usize, block_y: usize) -> Option<u8> {
        zpop_at(&self.zpop, V::CELL_SIZE_IN_BLOCKS, block_x, block_y)
    }
}

/// Looks up one block in a zombie population grid with `side` blocks per edge.
pub(crate) fn zpop_at(zpop: &[u8], side: usize, block_x: usize, block_y: usize) -> Option<u8> {
    if block_x >= side || block_y >= side {
        return None;
    }
    zpop.get(block_x.checked_mul(side)?.checked_add(block_y)?)
        .copied()
}

/// A parsed `.lotheader` that owns its buffer, for a build chosen at runtime.
///
/// The methods return the same data as the fields of [`LotHeader`].
///
/// ```no_run
/// use libpzmap::{Build, lotheader::AnyLotHeader};
///
/// let bytes = std::fs::read("0_18.lotheader")?;
/// let header = AnyLotHeader::parse(Build::B42, 0, 18, bytes)?;
/// println!("layers {}..{}", header.min_layer(), header.max_layer());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct AnyLotHeader(Inner);

enum Inner {
    B41(Yoke<LotHeader<'static, B41>, Vec<u8>>),
    B42(Yoke<LotHeader<'static, B42>, Vec<u8>>),
}

/// Evaluates `$body` with `$h` bound to the parsed header, whichever build it is.
macro_rules! each {
    ($self:ident, |$h:ident| $body:expr) => {
        match &$self.0 {
            Inner::B41(yoke) => {
                let $h = yoke.get();
                $body
            }
            Inner::B42(yoke) => {
                let $h = yoke.get();
                $body
            }
        }
    };
}

impl AnyLotHeader {
    /// Parses `bytes` as the `.lotheader` of cell `(x, y)` written by `build`, and keeps `bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`LotHeader::parse`].
    pub fn parse(build: Build, x: i32, y: i32, bytes: Vec<u8>) -> Result<Self> {
        let inner = match build {
            Build::B41 => Inner::B41(Yoke::try_attach_to_cart(bytes, |bytes| {
                LotHeader::parse(x, y, bytes)
            })?),
            Build::B42 => Inner::B42(Yoke::try_attach_to_cart(bytes, |bytes| {
                LotHeader::parse(x, y, bytes)
            })?),
        };
        Ok(AnyLotHeader(inner))
    }

    /// The build the header was parsed as.
    pub fn build(&self) -> Build {
        match self.0 {
            Inner::B41(_) => Build::B41,
            Inner::B42(_) => Build::B42,
        }
    }

    /// The buffer the header was parsed from.
    pub fn bytes(&self) -> &[u8] {
        match &self.0 {
            Inner::B41(yoke) => yoke.backing_cart(),
            Inner::B42(yoke) => yoke.backing_cart(),
        }
    }

    /// See [`LotHeader::x`].
    pub fn x(&self) -> i32 {
        each!(self, |h| h.x)
    }

    /// See [`LotHeader::y`].
    pub fn y(&self) -> i32 {
        each!(self, |h| h.y)
    }

    /// See [`LotHeader::version`].
    pub fn version(&self) -> u32 {
        each!(self, |h| h.version)
    }

    /// See [`LotHeader::strings`].
    pub fn strings(&self) -> &[&str] {
        each!(self, |h| &h.strings)
    }

    /// See [`LotHeader::min_layer`].
    pub fn min_layer(&self) -> i32 {
        each!(self, |h| h.min_layer)
    }

    /// See [`LotHeader::max_layer`].
    pub fn max_layer(&self) -> i32 {
        each!(self, |h| h.max_layer)
    }

    /// See [`LotHeader::rooms`].
    pub fn rooms(&self) -> &[Room<'_>] {
        each!(self, |h| &h.rooms)
    }

    /// See [`LotHeader::buildings`].
    pub fn buildings(&self) -> &[Building] {
        each!(self, |h| &h.buildings)
    }

    /// See [`LotHeader::zpop`].
    pub fn zpop(&self) -> &[u8] {
        each!(self, |h| &h.zpop)
    }

    /// Blocks along one edge of the zombie population grid.
    pub fn zpop_side(&self) -> usize {
        self.build().cell_size_in_blocks()
    }

    /// See [`LotHeader::zpop_at`].
    pub fn zpop_at(&self, block_x: usize, block_y: usize) -> Option<u8> {
        each!(self, |h| h.zpop_at(block_x, block_y))
    }

    /// See [`LotHeader::trailing_bytes`].
    pub fn trailing_bytes(&self) -> usize {
        each!(self, |h| h.trailing_bytes)
    }
}

/// One room of a cell, as listed by its `.lotheader`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Room<'inp> {
    /// The room's name, such as `kitchen`, borrowed from the lotheader buffer.
    pub name: &'inp str,
    /// The layer the room is on.
    pub layer: i32,
    /// The rectangles that make up the room's floor plan, in world coordinates.
    pub rectangles: Vec<Rectangle>,
    /// The objects placed in the room, in world coordinates.
    pub objects: Vec<Object>,
}

/// An axis-aligned rectangle of squares, in world coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rectangle {
    /// World X where the rectangle starts.
    pub x: i32,
    /// World Y where the rectangle starts.
    pub y: i32,
    /// Width in squares.
    pub width: i32,
    /// Height in squares.
    pub height: i32,
}

/// An object placed in a room, in world coordinates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Object {
    /// The game's type number for the object. This crate does not interpret it.
    pub kind: i32,
    /// World X of the object.
    pub x: i32,
    /// World Y of the object.
    pub y: i32,
}

/// A building, described by the rooms it is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Building {
    /// The positions in [`LotHeader::rooms`] of every room in this building.
    pub room_indices: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    /// Parsing through `Build` gives the same header as the typed parser.
    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn headers_parsed_through_build_match_the_typed_parser() {
        let bytes = fixtures::read(&fixtures::lotheader("Muldraugh, KY", 49, 6));

        let borrowed = LotHeader::<B42>::parse(49, 6, &bytes).expect("header should parse");
        let any = AnyLotHeader::parse(Build::B42, 49, 6, bytes.clone())
            .expect("runtime dispatch should parse");

        assert_eq!((any.x(), any.y()), (borrowed.x, borrowed.y));
        assert_eq!(any.strings(), &*borrowed.strings);
        assert_eq!(any.rooms().len(), borrowed.rooms.len());
        assert_eq!(any.buildings(), &*borrowed.buildings);

        // The zombie population grid is 32×32 blocks, indexed `block_x * 32 + block_y`.
        assert_eq!(any.zpop_side(), 32);
        assert_eq!(any.zpop().len(), 32 * 32);
        assert_eq!(any.zpop_at(1, 2), any.zpop().get(32 + 2).copied());
        assert_eq!(any.zpop_at(32, 0), None);
    }
}
