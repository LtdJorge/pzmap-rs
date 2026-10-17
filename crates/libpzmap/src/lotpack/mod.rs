/*!
Cell contents (`world_*.lotpack` files).

A lotpack describes which tiles sit on each square of one cell. The cell is divided into blocks of
8×8 squares in Build 42, or 10×10 in Build 41, and the file stores each block separately. Each
tile is stored as an index into the list of names in the cell's `.lotheader`.

Most squares are empty, so the file stores runs of empty squares as a single count. A run can
continue across rows and layers.

There are three ways to read a lotpack:

- [`Cell::visit`] calls a closure for each non-empty square. It borrows the tile data from the
  input and stores nothing per square. Use it when you only need to go through the squares once.
- [`Cell::parse`] collects the squares into [`Block`]s, so you can look them up afterward.
- [`AnyCell::parse`] collects every square into a few flat arrays for choosing a build at runtime.
*/

mod any;
mod parse;

pub use any::AnyCell;
pub use parse::{
    Block,
    SquareAt,
    SquareTiles,
};

use crate::build::VersionMetadata;
use std::marker::PhantomData;

/// A cell parsed from a `world_*.lotpack`.
///
/// `V` is the game build ([`B41`](crate::B41) or [`B42`](crate::B42)). The two builds use
/// different block sizes, so the type keeps a cell from one build from being mixed up with a cell
/// from the other.
#[derive(Debug)]
pub struct Cell<V: VersionMetadata> {
    pub(crate) blocks: Vec<Option<Block<V>>>,
    pub(crate) marker: PhantomData<fn() -> V>,
}

impl<V: VersionMetadata> Cell<V> {
    /// Every block of the cell, in file order.
    ///
    /// The block at (`block_x`, `block_y`) is at index
    /// `block_x * V::CELL_SIZE_IN_BLOCKS + block_y`, so the length is always
    /// [`VersionMetadata::CELL_SIZE_IN_BLOCKS`] squared. A block whose squares are all empty is
    /// `None`.
    pub fn blocks(&self) -> &[Option<Block<V>>] {
        &self.blocks
    }
}

/// The position of a square within its block.
#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct Coords2D {
    pub(crate) x: usize,
    pub(crate) y: usize,
}
