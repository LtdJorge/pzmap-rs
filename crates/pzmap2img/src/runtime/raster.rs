//! The tiles and sprites of a built map, for bitmap rendering.
//!
//! [`Map`] holds every cell of a map, looked up by cell coordinates. Each [`CellData`] stores,
//! for each layer, which squares have tiles and the tile ids on each of those squares.
//!
//! Most squares are empty, so a cell doesn't store a slot for every square. Instead, each layer
//! is split into groups of 8x8 squares. Each group has a `u64` with one bit per square, set
//! when the square has tiles. The tile ids of all non-empty squares sit in one flat array, and
//! a second array holds where each square's tiles start in it.
//!
//! The 8x8 groups don't depend on the game's own block size. Build 42's blocks are 8x8 squares
//! and match the groups exactly. Build 41's blocks are 10x10, too many for one `u64`, so a
//! Build 41 cell is covered by `ceil(300 / 8)` groups along each side instead.

use crate::{
    error::{
        BuildError,
        Result,
    },
    runtime::size::HeapSize,
    string_table::{
        StringTable,
        TileId,
        TileNames,
    },
    textures::{
        Sprite,
        TextureLibrary,
    },
};
use libpzmap::VersionMetadata;

/// Squares along one edge of a group.
pub(crate) const GROUP_SQUARES_U32: u32 = 8;
pub(crate) const GROUP_SQUARES: usize = GROUP_SQUARES_U32 as usize;
/// Squares in one group, one per bit of a `u64`.
pub(crate) const SQUARES_PER_GROUP: usize = GROUP_SQUARES * GROUP_SQUARES;

const _: () = assert!(SQUARES_PER_GROUP == u64::BITS as usize);

/// The size of a cell in one game build, and how it divides into groups of 8x8 squares.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellGeometry {
    cell_squares: u32,
    groups_per_row: u32,
}

impl CellGeometry {
    /// Creates the geometry for cells that are `cell_squares` squares on each side.
    pub const fn new(cell_squares: u32) -> Self {
        CellGeometry {
            cell_squares,
            groups_per_row: cell_squares.div_ceil(GROUP_SQUARES_U32),
        }
    }

    /// Returns the geometry for game build `V`.
    pub const fn of<V: VersionMetadata>() -> Self {
        Self::new(<V as VersionMetadata>::CELL_SIZE_IN_SQUARES.cast_unsigned())
    }

    /// Returns the number of squares along one side of a cell: 256 in Build 42, 300 in
    /// Build 41.
    #[inline]
    pub const fn cell_squares(&self) -> u32 {
        self.cell_squares
    }

    /// Returns the number of groups along one side of a cell.
    #[inline]
    pub const fn groups_per_row(&self) -> u32 {
        self.groups_per_row
    }

    /// Returns the number of groups in one layer of one cell.
    #[inline]
    pub const fn groups(&self) -> usize {
        (self.groups_per_row * self.groups_per_row) as usize
    }
}

/// Which squares of one group have tiles, and where the group's entries start.
///
/// Both fields share one 16-byte struct, so a lookup reads one cache line instead of two.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub(crate) struct BlockIdx {
    /// One bit per square in the group. A set bit means the square has at least one tile.
    pub mask: u64,
    /// The index of this group's first non-empty square in the layer's `starts` array.
    pub base: u32,
    __pad: u32,
}

const _: () = assert!(
    size_of::<BlockIdx>() == 16,
    "The size of BlockIdx has changed"
);

/// One layer of one cell that has at least one non-empty square.
pub(crate) struct LayerIndex {
    layer: i32,
    groups_per_row: u32,
    groups: Box<[BlockIdx]>,
    /// Where each non-empty square's tiles start in the cell's tile array, ordered by group,
    /// then by square within the group.
    ///
    /// It has one extra entry at the end, so square `i`'s tiles are `starts[i]..starts[i + 1]`.
    starts: Box<[u32]>,
}

impl LayerIndex {
    /// Returns the layer number.
    pub fn layer(&self) -> i32 {
        self.layer
    }

    /// Returns where a square's tiles are in the cell's tile array.
    ///
    /// `sub_x` and `sub_y` are square coordinates within the cell. Returns `None` if the square
    /// has no tiles or is outside the cell.
    #[inline]
    pub fn square(&self, sub_x: usize, sub_y: usize) -> Option<std::ops::Range<usize>> {
        let per_row = self.groups_per_row as usize;

        let group = (sub_x / GROUP_SQUARES) * per_row + (sub_y / GROUP_SQUARES);
        let square = (sub_x % GROUP_SQUARES) * GROUP_SQUARES + (sub_y % GROUP_SQUARES);
        let bit = 1_u64 << square;

        let entry = self.groups.get(group)?;
        if entry.mask & bit == 0 {
            return None;
        }

        // `rank` is the number of non-empty squares in this group before the target square.
        let rank = (entry.mask & (bit - 1)).count_ones() as usize;
        let i = entry.base as usize + rank;

        let start = *self.starts.get(i)? as usize;
        let end = *self.starts.get(i + 1)? as usize;
        Some(start..end)
    }

    /// Returns the number of non-empty squares on this layer.
    pub fn occupied_squares(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }
}

impl HeapSize for LayerIndex {
    /// Doesn't count the cell's tile array, which all layers share.
    fn heap_size(&self) -> usize {
        self.groups.len() * size_of::<BlockIdx>() + self.starts.len() * size_of::<u32>()
    }
}

/// One cell's tiles: which squares have tiles on each layer, and their tile ids.
pub struct CellData {
    /// The cell's x coordinate, in cells.
    pub cell_x: i32,
    /// The cell's y coordinate, in cells.
    pub cell_y: i32,
    geometry: CellGeometry,
    layers: Box<[LayerIndex]>,
    tiles: Box<[TileId]>,
}

impl CellData {
    pub(crate) fn layers(&self) -> &[LayerIndex] {
        &self.layers
    }

    /// Returns the number of squares, over all layers, that have at least one tile.
    pub fn occupied_squares(&self) -> usize {
        self.layers.iter().map(LayerIndex::occupied_squares).sum()
    }

    /// Returns the number of squares on `layer` that have at least one tile.
    pub fn occupied_squares_on(&self, layer: i32) -> usize {
        self.layers
            .iter()
            .filter(|l| l.layer() == layer)
            .map(LayerIndex::occupied_squares)
            .sum()
    }

    /// Returns the number of layers that have at least one non-empty square.
    pub fn populated_layers(&self) -> usize {
        self.layers.len()
    }

    /// Returns the cell's geometry.
    pub fn geometry(&self) -> CellGeometry {
        self.geometry
    }

    /// Returns true if this cell has any tiles on `layer`.
    pub fn has_layer(&self, layer: i32) -> bool {
        self.layer_index(layer).is_some()
    }

    fn layer_index(&self, layer: i32) -> Option<&LayerIndex> {
        // A cell has only a few non-empty layers, so a linear scan is faster than a map.
        self.layers.iter().find(|l| l.layer == layer)
    }

    /// Returns the tiles on a square, in the order they are drawn, bottom first.
    ///
    /// `sub_x` and `sub_y` are square coordinates within the cell. The order is the one in the
    /// lotpack. An empty square, or one outside the cell, gives an empty slice.
    #[inline]
    pub fn tiles(&self, layer: i32, sub_x: usize, sub_y: usize) -> &[TileId] {
        match self.layer_index(layer).and_then(|l| l.square(sub_x, sub_y)) {
            Some(range) => self.tiles.get(range).unwrap_or_default(),
            None => &[],
        }
    }

    /// Returns the number of tiles in the cell, over all squares and layers.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
}

impl HeapSize for CellData {
    fn heap_size(&self) -> usize {
        self.layers.iter().map(|l| l.heap_size()).sum::<usize>()
            + self.tiles.len() * size_of::<TileId>()
            + self.layers.len() * size_of::<LayerIndex>()
    }
}

/// One layer of a [`CellBuilder`], while it is still being filled.
struct LayerBucket {
    layer: i32,
    groups: Box<[BlockIdx]>,
    /// Where each square's tiles start in this bucket's own `tiles`. [`CellBuilder::build`]
    /// offsets them when it joins the buckets into one array.
    starts: Vec<u32>,
    tiles: Vec<TileId>,
    occupied: u32,
}

/// Collects one cell's squares while its lotpack is parsed, then builds a [`CellData`].
///
/// The lotpack lists squares block by block, with all layers of a block together. The builder
/// keeps each layer in its own bucket, and [`Self::build`] joins the buckets in layer order.
pub(crate) struct CellBuilder {
    cell_x: i32,
    cell_y: i32,
    geometry: CellGeometry,
    buckets: Vec<LayerBucket>,
    /// The most recently used bucket. [`Self::bucket_for`] checks it before searching.
    last: usize,
}

impl CellBuilder {
    /// Creates an empty builder for the cell at (`cell_x`, `cell_y`).
    pub fn new(cell_x: i32, cell_y: i32, geometry: CellGeometry) -> Self {
        CellBuilder {
            cell_x,
            cell_y,
            geometry,
            buckets: Vec::new(),
            last: usize::MAX,
        }
    }

    /// Returns the index of the bucket for `layer`, creating it if needed.
    #[inline]
    fn bucket_for(&mut self, layer: i32) -> usize {
        if let Some(bucket) = self.buckets.get(self.last)
            && bucket.layer == layer
        {
            return self.last;
        }

        let found = match self.buckets.iter().position(|b| b.layer == layer) {
            Some(index) => index,
            None => {
                self.buckets.push(LayerBucket {
                    layer,
                    groups: vec![BlockIdx::default(); self.geometry.groups()].into_boxed_slice(),
                    starts: vec![0],
                    tiles: Vec::new(),
                    occupied: 0,
                });
                self.buckets.len() - 1
            }
        };

        self.last = found;
        found
    }

    /// Adds one non-empty square and its tiles. `sub_x` and `sub_y` are square coordinates
    /// within the cell.
    ///
    /// # Panics
    ///
    /// Panics if `sub_x` or `sub_y` is outside the cell.
    // This runs once for every square of the map, so it indexes directly instead of using
    // `get_mut`. `index` comes from `bucket_for`. `group` is in range because of the assertion
    // on `sub_x` and `sub_y`, since `groups_per_row` is `ceil(cell_squares / 8)`.
    #[allow(clippy::indexing_slicing, clippy::cast_possible_truncation)]
    #[inline]
    pub fn push_square(
        &mut self,
        layer: i32,
        sub_x: usize,
        sub_y: usize,
        tiles: impl Iterator<Item = TileId>,
    ) {
        let cell_squares = self.geometry.cell_squares as usize;
        let per_row = self.geometry.groups_per_row as usize;

        assert!(
            sub_x < cell_squares && sub_y < cell_squares,
            "square ({sub_x},{sub_y}) is outside a {cell_squares}-square cell"
        );

        let group = (sub_x / GROUP_SQUARES) * per_row + (sub_y / GROUP_SQUARES);
        let square = (sub_x % GROUP_SQUARES) * GROUP_SQUARES + (sub_y % GROUP_SQUARES);

        let index = self.bucket_for(layer);
        let bucket = &mut self.buckets[index];

        let entry = &mut bucket.groups[group];
        if entry.mask == 0 {
            entry.base = bucket.occupied;
        }
        entry.mask |= 1_u64 << square;

        bucket.tiles.extend(tiles);
        bucket.starts.push(bucket.tiles.len() as u32);
        bucket.occupied += 1;
    }

    /// Joins the layers in layer order and returns the finished cell.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::CellTooLarge`] if the cell has more tiles than a `u32` can index.
    pub fn build(mut self) -> Result<CellData> {
        self.buckets.sort_unstable_by_key(|b| b.layer);

        let total: usize = self.buckets.iter().map(|b| b.tiles.len()).sum();
        if u32::try_from(total).is_err() {
            return Err(BuildError::CellTooLarge {
                cell_x: self.cell_x,
                cell_y: self.cell_y,
                tiles: total,
                max: u32::MAX,
            });
        }

        // Reuses the first bucket's tile array for the result instead of copying it.
        let extra: usize = self.buckets.iter().skip(1).map(|b| b.tiles.len()).sum();
        let mut tiles = match self.buckets.first_mut() {
            Some(first) => {
                let mut tiles = std::mem::take(&mut first.tiles);
                tiles.reserve_exact(extra);
                tiles
            }
            None => Vec::new(),
        };

        let mut layers = Vec::with_capacity(self.buckets.len());
        for (index, bucket) in self.buckets.into_iter().enumerate() {
            // Fits in a `u32`, because the check above rejected a cell with more tiles.
            let base = if index == 0 {
                0
            } else {
                u32::try_from(tiles.len()).unwrap_or(u32::MAX)
            };
            if index != 0 {
                tiles.extend_from_slice(&bucket.tiles);
            }

            let starts = if base == 0 {
                bucket.starts.into_boxed_slice()
            } else {
                bucket
                    .starts
                    .into_iter()
                    .map(|start| start + base)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            };

            layers.push(LayerIndex {
                layer: bucket.layer,
                groups_per_row: self.geometry.groups_per_row,
                groups: bucket.groups,
                starts,
            });
        }

        Ok(CellData {
            cell_x: self.cell_x,
            cell_y: self.cell_y,
            geometry: self.geometry,
            layers: layers.into_boxed_slice(),
            tiles: tiles.into_boxed_slice(),
        })
    }
}

/// A built map for bitmap rendering: its cells, its sprites, and its tile names.
///
/// [`build`](crate::builder::build) produces it when [`BuildTargets::raster`] is set.
///
/// [`BuildTargets::raster`]: crate::runtime::BuildTargets::raster
pub struct Map {
    cells: Box<[CellData]>,
    /// A grid covering every cell from `origin` to `origin + extent`. Each entry is an index
    /// into `cells`, or `NO_CELL` where the map has no cell.
    cell_index: Box<[u32]>,
    origin: (i32, i32),
    extent: (i32, i32),
    geometry: CellGeometry,
    textures: TextureLibrary,
    tile_names: StringTable<TileNames>,
}

const NO_CELL: u32 = u32::MAX;

impl Map {
    /// Creates a map from its cells, the geometry they share, their sprites and the tile name
    /// table their tile ids come from.
    pub fn new(
        cells: Vec<CellData>,
        geometry: CellGeometry,
        textures: TextureLibrary,
        tile_names: StringTable<TileNames>,
    ) -> Self {
        let (min_x, min_y, max_x, max_y) = cells.iter().fold(
            (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
            |(a, b, c, d), cell| {
                (
                    a.min(cell.cell_x),
                    b.min(cell.cell_y),
                    c.max(cell.cell_x),
                    d.max(cell.cell_y),
                )
            },
        );

        let (w, h) = if cells.is_empty() {
            (0, 0)
        } else {
            (
                max_x.saturating_sub(min_x).saturating_add(1),
                max_y.saturating_sub(min_y).saturating_add(1),
            )
        };

        // `w` and `h` are not negative, and every cell is at or after the origin, so the
        // values below are not negative either.
        let (uw, uh) = (
            w.max(0).cast_unsigned() as usize,
            h.max(0).cast_unsigned() as usize,
        );
        let mut cell_index = vec![NO_CELL; uw * uh];

        for (i, cell) in cells.iter().enumerate() {
            let cx = cell.cell_x.saturating_sub(min_x).max(0).cast_unsigned() as usize;
            let cy = cell.cell_y.saturating_sub(min_y).max(0).cast_unsigned() as usize;

            if let Some(slot) = cell_index.get_mut(cy * uw + cx) {
                *slot = u32::try_from(i).unwrap_or(NO_CELL);
            }
        }

        Map {
            cells: cells.into_boxed_slice(),
            cell_index: cell_index.into_boxed_slice(),
            origin: (min_x, min_y),
            extent: (w, h),
            geometry,
            textures,
            tile_names,
        }
    }

    /// Returns the cell at the given cell coordinates, or `None` if the map has no cell there.
    #[inline]
    pub fn cell(&self, cell_x: i32, cell_y: i32) -> Option<&CellData> {
        let cx = cell_x.checked_sub(self.origin.0)?;
        let cy = cell_y.checked_sub(self.origin.1)?;
        if cx < 0 || cy < 0 || cx >= self.extent.0 || cy >= self.extent.1 {
            return None;
        }

        // The check above rejected negative values, so these conversions succeed.
        let (cx, cy) = (usize::try_from(cx).ok()?, usize::try_from(cy).ok()?);
        let stride = usize::try_from(self.extent.0).ok()?;

        let slot = *self.cell_index.get(cy * stride + cx)?;
        if slot == NO_CELL {
            return None;
        }

        self.cells.get(slot as usize)
    }

    /// Returns the cell geometry every cell in the map shares.
    pub fn geometry(&self) -> CellGeometry {
        self.geometry
    }

    /// Returns the tiles on a square, given its world square coordinates and its layer.
    ///
    /// The tiles are in drawing order, bottom first. A square with no tiles, or outside the
    /// map, gives an empty slice.
    #[inline]
    pub fn tiles_at(&self, world_x: i32, world_y: i32, layer: i32) -> &[TileId] {
        let size = self.geometry.cell_squares.cast_signed();
        let (cell_x, sub_x) = (world_x.div_euclid(size), world_x.rem_euclid(size));
        let (cell_y, sub_y) = (world_y.div_euclid(size), world_y.rem_euclid(size));

        let (Ok(sub_x), Ok(sub_y)) = (usize::try_from(sub_x), usize::try_from(sub_y)) else {
            return &[];
        };

        match self.cell(cell_x, cell_y) {
            Some(cell) => cell.tiles(layer, sub_x, sub_y),
            None => &[],
        }
    }

    /// Returns the sprite for a tile, or `None` if no loaded texture pack has it.
    #[inline]
    pub fn sprite(&self, id: TileId) -> Option<Sprite<'_>> {
        self.textures.get(id)
    }

    /// Returns the sprites of every tile the map uses.
    pub fn textures(&self) -> &TextureLibrary {
        &self.textures
    }

    /// Returns the tile name table, indexed by tile id.
    pub fn tile_names(&self) -> &StringTable<TileNames> {
        &self.tile_names
    }

    /// Returns every cell in the map.
    pub fn cells(&self) -> &[CellData] {
        &self.cells
    }

    /// Returns the lowest and highest layer that has tiles in any cell, or `None` if the map
    /// has no tiles. Both ends are inclusive.
    pub fn layer_range(&self) -> Option<(i32, i32)> {
        self.cells
            .iter()
            .flat_map(|cell| cell.layers())
            .map(LayerIndex::layer)
            .fold(None, |range, layer| match range {
                None => Some((layer, layer)),
                Some((min, max)) => Some((min.min(layer), max.max(layer))),
            })
    }
}

impl HeapSize for Map {
    fn heap_size(&self) -> usize {
        self.cells.iter().map(|c| c.heap_size()).sum::<usize>()
            + self.cell_index.len() * size_of::<u32>()
            + self.textures.blob_len()
            + self.textures.len() * size_of::<crate::textures::TexMeta>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libpzmap::{
        B41,
        B42,
    };

    #[test]
    fn b42_geometry_is_unchanged_by_the_regrouping() {
        let geometry = CellGeometry::of::<B42>();
        assert_eq!(geometry.cell_squares(), 256);
        assert_eq!(geometry.groups_per_row(), 32);
        assert_eq!(geometry.groups(), 1024);
        assert_eq!(geometry.groups() * size_of::<BlockIdx>(), 16 * 1024);
    }

    #[test]
    fn b41_geometry_rounds_up_to_whole_groups() {
        let geometry = CellGeometry::of::<B41>();
        assert_eq!(geometry.cell_squares(), 300);
        assert_eq!(geometry.groups_per_row(), 38);
        assert_eq!(geometry.groups(), 1444);
    }

    fn tile(i: usize) -> TileId {
        TileId::from_index(i).expect("valid index")
    }

    #[test]
    fn squares_round_trip_through_the_csr() {
        let geometry = CellGeometry::of::<B42>();
        let mut builder = CellBuilder::new(3, 4, geometry);

        let placed = [(0, 0), (0, 1), (7, 7), (8, 0), (255, 255), (100, 200)];
        for (n, (x, y)) in placed.iter().enumerate() {
            builder.push_square(0, *x, *y, [tile(n + 1), tile(n + 2)].into_iter());
        }
        builder.push_square(3, 10, 10, [tile(99)].into_iter());

        let cell = builder.build().expect("cell should build");

        for (n, (x, y)) in placed.iter().enumerate() {
            assert_eq!(
                cell.tiles(0, *x, *y),
                &[tile(n + 1), tile(n + 2)],
                "square ({x},{y})"
            );
        }

        assert_eq!(cell.tiles(3, 10, 10), &[tile(99)]);
        assert_eq!(cell.tiles(0, 1, 1), &[]);
        assert_eq!(cell.tiles(7, 0, 0), &[]);
    }

    #[test]
    fn b41_edge_groups_resolve() {
        let geometry = CellGeometry::of::<B41>();
        let mut builder = CellBuilder::new(0, 0, geometry);

        builder.push_square(0, 299, 299, [tile(7)].into_iter());
        builder.push_square(0, 296, 0, [tile(8)].into_iter());

        let cell = builder.build().expect("cell should build");
        assert_eq!(cell.tiles(0, 299, 299), &[tile(7)]);
        assert_eq!(cell.tiles(0, 296, 0), &[tile(8)]);
        assert_eq!(cell.tiles(0, 298, 299), &[]);
    }

    #[test]
    fn layers_are_independent() {
        let mut builder = CellBuilder::new(0, 0, CellGeometry::of::<B42>());
        builder.push_square(-5, 4, 4, [tile(1)].into_iter());
        builder.push_square(0, 4, 4, [tile(2)].into_iter());
        builder.push_square(9, 4, 4, [tile(3)].into_iter());

        let cell = builder.build().expect("cell should build");
        assert_eq!(cell.tiles(-5, 4, 4), &[tile(1)]);
        assert_eq!(cell.tiles(0, 4, 4), &[tile(2)]);
        assert_eq!(cell.tiles(9, 4, 4), &[tile(3)]);
        assert_eq!(cell.layers().len(), 3);
    }

    #[test]
    #[should_panic(expected = "outside a 256-square cell")]
    fn a_square_outside_the_cell_is_a_programmer_error() {
        let mut builder = CellBuilder::new(0, 0, CellGeometry::of::<B42>());
        builder.push_square(0, 256, 0, std::iter::empty());
    }
}
