use crate::{
    build::{
        Build,
        dispatch,
    },
    error::Result,
    lotpack::{
        Cell,
        SquareAt,
    },
};

/// The non-empty squares of a `world_*.lotpack`, stored in flat arrays, for choosing a build at
/// runtime.
///
/// Squares are in the same order as [`Cell::visit`] returns. Flat arrays are easy to hand to NumPy
/// or Java.
///
/// ```no_run
/// use libpzmap::{Build, lotpack::AnyCell};
///
/// let bytes = std::fs::read("world_0_18.lotpack")?;
/// let cell = AnyCell::parse(Build::B42, 0, 18, &bytes, -32, 32)?;
/// for (index, square) in cell.squares().iter().enumerate() {
///     let tiles = cell.tiles_of(index).unwrap_or_default();
///     println!("layer {} has {} tiles here", square.layer, tiles.len());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnyCell {
    x: i32,
    y: i32,
    squares: Vec<SquareAt>,
    tiles: Vec<i32>,
    offsets: Vec<usize>,
}

impl AnyCell {
    /// Parses the `world_*.lotpack` of cell `(x, y)`, written by `build`.
    ///
    /// Pass the `min_layer` and `max_layer` of the cell's `.lotheader`. This reads the file with
    /// [`Cell::visit`], so it uses less memory than [`Cell::parse`].
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Cell::parse`].
    pub fn parse(
        build: Build,
        x: i32,
        y: i32,
        bytes: &[u8],
        min_layer: i32,
        max_layer: i32,
    ) -> Result<AnyCell> {
        let mut cell = AnyCell {
            x,
            y,
            squares: Vec::new(),
            tiles: Vec::new(),
            offsets: vec![0],
        };

        dispatch!(build, |V| Cell::<V>::visit(
            bytes,
            min_layer,
            max_layer,
            |at, tiles| {
                cell.squares.push(at);
                cell.tiles.extend(tiles.iter());
                cell.offsets.push(cell.tiles.len());
            }
        ))?;

        Ok(cell)
    }

    /// The cell's X position in the cell grid.
    pub fn x(&self) -> i32 {
        self.x
    }

    /// The cell's Y position in the cell grid.
    pub fn y(&self) -> i32 {
        self.y
    }

    /// The position of each non-empty square.
    pub fn squares(&self) -> &[SquareAt] {
        &self.squares
    }

    /// The tiles of every square, one square after another.
    ///
    /// The tiles of square `i` are `tiles[offsets[i]..offsets[i + 1]]`. Each tile is an index into
    /// the list of names in the cell's [`LotHeader`](crate::lotheader::LotHeader).
    pub fn tiles(&self) -> &[i32] {
        &self.tiles
    }

    /// The starting position for each square's tiles in [`AnyCell::tiles`] and a final entry for
    /// the end.
    ///
    /// There are `squares().len() + 1` entries, so square `i` ends at the start of square `i + 1`.
    pub fn offsets(&self) -> &[usize] {
        &self.offsets
    }

    /// How many non-empty squares this cell holds.
    pub fn len(&self) -> usize {
        self.squares.len()
    }

    /// Whether this cell holds no non-empty squares.
    pub fn is_empty(&self) -> bool {
        self.squares.is_empty()
    }

    /// The tiles of one square, given its position in [`AnyCell::squares`].
    pub fn tiles_of(&self, square: usize) -> Option<&[i32]> {
        let start = *self.offsets.get(square)?;
        let end = *self.offsets.get(square.checked_add(1)?)?;
        self.tiles.get(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        build::B42,
        fixtures,
        lotheader::LotHeader,
    };

    /// `tiles_of` returns the same tiles that `Cell::visit` gives for each square.
    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn any_cells_slice_back_into_the_same_tiles() {
        let header_bytes = fixtures::read(&fixtures::lotheader("Muldraugh, KY", 49, 6));
        let pack_bytes = fixtures::read(&fixtures::lotpack("Muldraugh, KY", 49, 6));

        let header = LotHeader::<B42>::parse(49, 6, &header_bytes).expect("header should parse");

        let cell = AnyCell::parse(
            Build::B42,
            49,
            6,
            &pack_bytes,
            header.min_layer,
            header.max_layer,
        )
        .expect("cell should parse");

        assert!(!cell.is_empty());
        assert_eq!(cell.offsets().len(), cell.len() + 1);
        assert_eq!(cell.offsets().last().copied(), Some(cell.tiles().len()));

        let mut expected = Vec::new();
        Cell::<B42>::visit(
            &pack_bytes,
            header.min_layer,
            header.max_layer,
            |at, tiles| {
                expected.push((at, tiles.iter().collect::<Vec<_>>()));
            },
        )
        .expect("cell should stream");

        assert_eq!(cell.len(), expected.len());
        for (index, (at, tiles)) in expected.iter().enumerate() {
            assert_eq!(cell.squares().get(index), Some(at));
            assert_eq!(cell.tiles_of(index), Some(tiles.as_slice()));
        }

        assert_eq!(cell.tiles_of(cell.len()), None);
    }
}
