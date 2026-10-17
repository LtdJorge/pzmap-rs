use crate::{
    build::{
        VersionMetadata,
        repeat,
        widen,
    },
    error::{
        Format,
        ParseError,
        ParseErrorKind,
        Result,
    },
    input::Input,
    lotpack::{
        Cell,
        Coords2D,
    },
};
use std::{
    collections::HashMap,
    marker::PhantomData,
    ops::Range,
};
use tracing::trace;

/// The non-empty squares on every layer for a block.
///
/// A square can hold several tiles, such as a floor, a wall and a piece of furniture. Each tile is
/// an index into the list of names in the cell's `.lotheader`.
#[derive(Debug)]
pub struct Block<V: VersionMetadata> {
    // Each non-empty square's tiles are in a specific range inside `tile_storage`, keyed by layer
    // and position.
    squares: HashMap<(i32, Coords2D), Range<usize>, ahash::RandomState>,
    // The flattened storage of all tiles of every square in the block.
    tile_storage: Vec<i32>,
    marker: PhantomData<fn() -> V>,
}

impl<V: VersionMetadata> Default for Block<V> {
    fn default() -> Self {
        Self {
            squares: HashMap::default(),
            tile_storage: Vec::new(),
            marker: PhantomData,
        }
    }
}

impl<V: VersionMetadata> Block<V> {
    /// The tiles on one square, in the order they are drawn, bottom first.
    ///
    /// `x` and `y` are the square's position within the block. Returns `None` for an empty
    /// square. Each tile is an index into the list of names in the cell's `.lotheader`.
    pub fn tiles(&self, layer: i32, x: usize, y: usize) -> Option<&[i32]> {
        let range = self.squares.get(&(layer, Coords2D { x, y }))?;
        self.tile_storage.get(range.clone())
    }
}

/// Reads the file header, checks the version, and returns where each block starts.
fn open<V: VersionMetadata>(bytes: &[u8]) -> Result<(Input<'_>, Vec<u64>)> {
    let mut input = Input::new(bytes, Format::LotPack);
    trace!("Input size {}", input.len());

    let at = input.offset();
    let version = input.optional_magic_version("LOTP")?;

    if version != const { <V as VersionMetadata>::VERSION_NUMBER } {
        return Err(ParseError {
            format: Format::LotPack,
            at,
            kind: ParseErrorKind::Version {
                what: V::NAME,
                found: version,
            },
        });
    }

    let block_num = input.u32_le()?;
    trace!("block_num {block_num}");

    // The block count must be exactly the build's. That also keeps the block numbers that
    // `Cell::visit` reports in range.
    let expected = const {
        widen(
            <V as VersionMetadata>::CELL_SIZE_IN_BLOCKS
                * <V as VersionMetadata>::CELL_SIZE_IN_BLOCKS,
        )
    };
    input.require_range("block count", i64::from(block_num), expected, expected)?;

    // Each entry is a `u32` offset followed by four zero bytes.
    let block_num = input.checked_count("block count", block_num as i64, 8)?;

    let offsets = repeat(&mut input, block_num, |inp| {
        let offset = inp.u32_le()?;
        let _null = inp.i32_le()?;
        Ok(u64::from(offset))
    })?;

    Ok((input, offsets))
}

impl<V: VersionMetadata> Cell<V> {
    /// Parses a `world_*.lotpack` written by build `V`, keeping its squares for later lookup.
    ///
    /// The file does not record layers. Pass the `min_layer` and `max_layer` of the cell's
    /// [`LotHeader`](crate::lotheader::LotHeader). If you only need to go through the squares once,
    /// [`Cell::visit`] does less work.
    ///
    /// ```no_run
    /// use libpzmap::{B42, lotheader::LotHeader, lotpack::Cell};
    ///
    /// let header_bytes = std::fs::read("0_18.lotheader")?;
    /// let header = LotHeader::<B42>::parse(0, 18, &header_bytes)?;
    ///
    /// let bytes = std::fs::read("world_0_18.lotpack")?;
    /// let cell = Cell::<B42>::parse(&bytes, header.min_layer, header.max_layer)?;
    ///
    /// // Tiles on square (3, 4) of the first block, at ground level.
    /// if let Some(Some(block)) = cell.blocks().first() {
    ///     for &tile in block.tiles(0, 3, 4).unwrap_or_default() {
    ///         let name = usize::try_from(tile).ok().and_then(|i| header.strings.get(i));
    ///         println!("{name:?}");
    ///     }
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the file is truncated, if the version or block count does not match
    /// `V`, if a block offset is past the end of the file, or if a count is larger than the
    /// remaining data.
    pub fn parse(bytes: &[u8], min_layer: i32, max_layer: i32) -> Result<Self> {
        let (input, offsets) = open::<V>(bytes)?;

        let mut blocks = Vec::with_capacity(offsets.len());
        for (index, offset) in offsets.into_iter().enumerate() {
            let mut block_input = input.at(offset, "block")?;
            let mut block = Block::<V>::default();

            visit_block::<V, _>(
                &mut block_input,
                index,
                min_layer,
                max_layer,
                &mut |at, tiles| {
                    let start = block.tile_storage.len();
                    block.tile_storage.extend(tiles.iter());
                    let end = block.tile_storage.len();
                    block
                        .squares
                        .insert((at.layer, Coords2D { x: at.x, y: at.y }), start..end);
                },
            )?;

            blocks.push((!block.squares.is_empty()).then_some(block));
        }

        Ok(Cell {
            blocks,
            marker: PhantomData,
        })
    }
}

/// Reads the length of a run of empty squares and returns how many more squares to skip.
///
/// Returns an error if the length is negative.
#[inline]
fn read_skip_run(input: &mut Input<'_>) -> Result<usize> {
    let run = input.i32_le()?;
    let run = input.require_count("skip run", i64::from(run))?;

    // The run includes the square that holds the run marker.
    Ok(run.saturating_sub(1))
}

/// The tiles of one square, as raw bytes borrowed from the file.
///
/// [`Cell::visit`] passes one of these per square. [`SquareTiles::iter`] decodes the tile indices
/// without allocating.
#[derive(Copy, Clone, Debug)]
pub struct SquareTiles<'inp>(&'inp [u8]);

impl<'inp> SquareTiles<'inp> {
    /// How many tiles this square holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len() / 4
    }

    /// Whether this square holds no tiles.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The raw bytes: four per tile, as little-endian `i32`s.
    #[inline]
    pub fn as_bytes(&self) -> &'inp [u8] {
        self.0
    }

    /// The tile indices. Each one indexes the list of names in the cell's `.lotheader`.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = i32> + 'inp {
        self.0
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(i32::from_le_bytes)
    }
}

/// The position of a non-empty square within its cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SquareAt {
    /// The block's index in [`Cell::blocks`]: `block_x * CELL_SIZE_IN_BLOCKS + block_y`.
    pub block: usize,
    /// The square's layer.
    pub layer: i32,
    /// The square's X within its block.
    pub x: usize,
    /// The square's Y within its block.
    pub y: usize,
    /// The room id the map editor stored on this square, or a negative value for none.
    ///
    /// This is not a reliable way to find a square's room. In Build 42 nearly every square has
    /// -1, including squares inside rooms, and the other values do not index the `.lotheader`
    /// room list. To find a square's room, check which of the header's room rectangles contain
    /// it.
    pub room: i32,
}

impl<V: VersionMetadata> Cell<V> {
    /// Reads a `world_*.lotpack` written by build `V` and calls `visit` for each non-empty
    /// square.
    ///
    /// The tiles passed to `visit` borrow `bytes`. Pass the `min_layer` and `max_layer` of the
    /// cell's [`LotHeader`](crate::lotheader::LotHeader).
    ///
    /// Squares come in file order: by block, then by layer from lowest to highest, then by x,
    /// then by y.
    ///
    /// ```no_run
    /// use libpzmap::{B42, lotpack::Cell};
    ///
    /// let bytes = std::fs::read("world_0_18.lotpack")?;
    /// let mut tiles = 0;
    /// Cell::<B42>::visit(&bytes, -32, 32, |_at, square| tiles += square.len())?;
    /// println!("{tiles} tiles");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`Cell::parse`]. `visit` has already been
    /// called for the squares before the error.
    pub fn visit<'inp, F>(
        bytes: &'inp [u8],
        min_layer: i32,
        max_layer: i32,
        mut visit: F,
    ) -> Result<()>
    where
        F: FnMut(SquareAt, SquareTiles<'inp>),
    {
        let (input, offsets) = open::<V>(bytes)?;

        for (block, offset) in offsets.into_iter().enumerate() {
            let mut block_input = input.at(offset, "block")?;
            visit_block::<V, _>(&mut block_input, block, min_layer, max_layer, &mut visit)?;
        }

        Ok(())
    }
}

/// Reads one block calling `visit` for each non-empty square.
///
/// A run of empty squares can cover single squares, whole rows, or whole layers, so `skip`
/// carries over from one row and layer to the next.
fn visit_block<'inp, V: VersionMetadata, F>(
    input: &mut Input<'inp>,
    block: usize,
    min_layer: i32,
    max_layer: i32,
    visit: &mut F,
) -> Result<()>
where
    F: FnMut(SquareAt, SquareTiles<'inp>),
{
    let block_size = const { <V as VersionMetadata>::BLOCK_SIZE_IN_SQUARES };
    let square_per_layer = block_size * block_size;

    let mut skip = 0_usize;

    for layer in min_layer..max_layer {
        if skip >= square_per_layer {
            skip -= square_per_layer;
            continue;
        }

        for x in 0..block_size {
            if skip >= block_size {
                skip -= block_size;
                continue;
            }

            for y in 0..block_size {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }

                let count = input.i32_le()?;

                // -1 marks a run of empty squares followed by its length
                if count == -1 {
                    skip = read_skip_run(input)?;
                    continue;
                }

                if count > 1 {
                    // `count` includes this room id, which comes before the tiles.
                    let room = input.i32_le()?;

                    let tiles = input.checked_count("square tile count", count as i64 - 1, 4)?;
                    let tiles = input.take(tiles * 4)?;

                    visit(
                        SquareAt {
                            block,
                            layer,
                            x,
                            y,
                            room,
                        },
                        SquareTiles(tiles),
                    );
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        build::B42,
        error::{
            Format,
            ParseError,
            ParseErrorKind,
        },
        lotpack::Cell,
    };

    /// The number of blocks in a Build 42 cell.
    const BLOCKS: u32 = 32 * 32;
    const PROLOGUE_LEN: u32 = 12 + BLOCKS * 8;

    /// The start of a valid Build 42 lotpack, up to the end of its block table, with every
    /// block starting at `block_offset`.
    ///
    /// Tests append block data to this to reach errors inside a block.
    fn prologue(block_offset: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PROLOGUE_LEN as usize);
        bytes.extend_from_slice(b"LOTP");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&BLOCKS.to_le_bytes());
        for _ in 0..BLOCKS {
            bytes.extend_from_slice(&block_offset.to_le_bytes());
            bytes.extend_from_slice(&0_i32.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_truncated_lotpack_is_an_error_not_a_panic() {
        // "LOTP", version 1, the right block count, then nothing where the table should be.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LOTP");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&BLOCKS.to_le_bytes());

        let err = Cell::<B42>::visit(&bytes, 0, 1, |_, _| {})
            .expect_err("a truncated offset table must be rejected");

        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::ImplausibleCount { .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_wrong_version_is_reported_as_such() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LOTP");
        bytes.extend_from_slice(&7_u32.to_le_bytes());

        let err = Cell::<B42>::visit(&bytes, 0, 1, |_, _| {}).expect_err("version 7 is not B42");

        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::LotPack,
                    kind: ParseErrorKind::Version { found: 7, .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_block_offset_past_eof_is_an_error_not_a_panic() {
        // Every table entry points far past the end of the file.
        let bytes = prologue(0xFFFF_FFFF);

        let err = Cell::<B42>::visit(&bytes, 0, 1, |_, _| {})
            .expect_err("an out-of-range block offset must be rejected");

        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::OffsetOutOfBounds { .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    /// A negative run length is an error, not a huge skip.
    #[test]
    fn a_negative_skip_run_is_rejected() {
        let mut bytes = prologue(PROLOGUE_LEN);
        // Every block's data: -1 to start a run, then a negative run length.
        bytes.extend_from_slice(&(-1_i32).to_le_bytes());
        bytes.extend_from_slice(&(-5_i32).to_le_bytes());

        let err = Cell::<B42>::visit(&bytes, 0, 1, |_, _| {})
            .expect_err("a negative skip run must be rejected");

        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::OutOfRange {
                        field: "skip run",
                        value: -5,
                        ..
                    },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }
}

#[cfg(test)]
mod visitor_tests {
    use crate::{
        build::B42,
        fixtures,
        lotheader::LotHeader,
        lotpack::Cell,
    };
    use std::fs;

    /// `Cell::visit` sees the same squares and tiles that `Cell::parse` collects.
    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn visit_matches_parse() -> anyhow::Result<()> {
        let dir = fixtures::muldraugh();
        let mut cells_checked = 0_usize;
        let mut squares_checked = 0_usize;

        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("lotheader") {
                continue;
            }
            let Some((x, y)) = stem.split_once('_') else {
                continue;
            };
            let (x, y): (i32, i32) = (x.parse()?, y.parse()?);

            let header_bytes = fs::read(&path)?;
            let pack_path = dir.join(format!("world_{x}_{y}.lotpack"));
            let pack_bytes = fs::read(&pack_path)?;

            let header = LotHeader::<B42>::parse(x, y, &header_bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));

            let cell = Cell::<B42>::parse(&pack_bytes, header.min_layer, header.max_layer)
                .unwrap_or_else(|e| panic!("{}: {e}", pack_path.display()));

            let mut visited = Vec::new();
            Cell::<B42>::visit(
                &pack_bytes,
                header.min_layer,
                header.max_layer,
                |at, tiles| {
                    visited.push((at, tiles.iter().collect::<Vec<_>>()));
                },
            )
            .unwrap_or_else(|e| panic!("{}: {e}", pack_path.display()));

            for (at, tiles) in &visited {
                let block = cell.blocks()[at.block]
                    .as_ref()
                    .unwrap_or_else(|| panic!("block {} should exist", at.block));
                let handle = block.tiles(at.layer, at.x, at.y).unwrap_or_else(|| {
                    panic!(
                        "square {},{} of layer {} should exist",
                        at.x, at.y, at.layer
                    )
                });

                assert_eq!(handle, tiles.as_slice());
                squares_checked += 1;
            }

            cells_checked += 1;
            if cells_checked >= 200 {
                break;
            }
        }

        assert!(cells_checked > 0, "no cells found in {}", dir.display());
        println!("checked {cells_checked} cells / {squares_checked} squares");
        Ok(())
    }
}
