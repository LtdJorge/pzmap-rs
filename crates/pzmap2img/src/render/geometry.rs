//! The isometric projection from map squares to image pixels.
//!
//! The game map is a grid of squares, addressed by world coordinates `(world_x, world_y)`. On
//! screen, the map is turned 45 degrees and squashed, so each square is drawn as a diamond. This
//! module places those diamonds on an intermediate *isometric grid*:
//!
//! ```text
//! grid_col = world_x - world_y
//! grid_row = world_x + world_y
//! ```
//!
//! A square's grid point is the top tip of its diamond. Moving one square along either world axis
//! moves one column sideways and one row down. So `grid_col + grid_row` is always even for a
//! square. Grid points where the sum is odd fall between diamonds and have no square.
//!
//! One grid step is [`IsoScale::step_x`] by [`IsoScale::step_y`] pixels: 64x32 for the game's
//! 2x texture packs, and 32x16 for the 1x packs. A diamond is two steps wide and two steps high.
//! Everything else here, such as cell bounds, tile origins and [`GRID_ROWS_PER_LAYER`], is in
//! grid units, and is the same at both scales. Only [`IsoScale`] converts grid units to pixels.
//!
//! The size of a cell (256 squares across in Build 42, 300 in Build 41) is passed in, to
//! [`cell_grid_bounds`] for example. This module does not assume either.

use crate::math::div_floor_i32;
use std::ops::Add;

/// How many grid rows each height layer moves up the image.
///
/// This is six at both scales: 192 pixels at 2x and 96 pixels at 1x.
pub const GRID_ROWS_PER_LAYER: i32 = 6;

/// The size of one isometric grid step in pixels, which sets the scale the map is drawn at.
///
/// A `.pack` file does not say which scale its sprites are, so the caller must choose the scale
/// that matches the packs. With the wrong scale, every sprite is placed correctly but drawn at the
/// wrong size.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct IsoScale {
    step_x: i32,
    step_y: i32,
}

impl IsoScale {
    /// The scale of the 2x packs, such as `Tiles2x.pack`. 64x32 pixels per step.
    pub const X2: IsoScale = IsoScale::new(64, 32);

    /// The scale of the 1x packs. 32x16 pixels per step.
    pub const X1: IsoScale = IsoScale::new(32, 16);

    /// A scale with the given step sizes in pixels. Each is raised to at least 1, so they are
    /// never zero.
    pub const fn new(step_x: i32, step_y: i32) -> Self {
        IsoScale {
            step_x: if step_x < 1 { 1 } else { step_x },
            step_y: if step_y < 1 { 1 } else { step_y },
        }
    }

    /// The width of one grid step in pixels.
    #[inline]
    pub const fn step_x(&self) -> i32 {
        self.step_x
    }

    /// The height of one grid step in pixels.
    #[inline]
    pub const fn step_y(&self) -> i32 {
        self.step_y
    }

    /// The height of a square's diamond in pixels, which is two grid steps.
    #[inline]
    pub const fn diamond_height(&self) -> i32 {
        self.step_y * 2
    }

    /// How many pixels each height layer moves up the image.
    #[inline]
    pub const fn layer_height_px(&self) -> i32 {
        self.step_y * GRID_ROWS_PER_LAYER
    }

    /// Whether a square tile of `size` pixels is a whole number of grid steps both ways.
    ///
    /// Otherwise tiles would end partway through a grid step, and neighboring tiles would
    /// disagree about which squares they cover.
    pub const fn divides_tile(&self, size: u32) -> bool {
        // `size` is a `u32` and the steps are positive `i32`s, so both fit in a `u64` exactly.
        let size = size as u64;
        size > 0
            && size.is_multiple_of(self.step_x.cast_unsigned() as u64)
            && size.is_multiple_of(self.step_y.cast_unsigned() as u64)
    }
}

impl Default for IsoScale {
    /// [`Self::X2`].
    fn default() -> Self {
        IsoScale::X2
    }
}

impl std::fmt::Display for IsoScale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.step_x, self.step_y)
    }
}

/// A rectangle on the isometric grid. All four edges are inclusive.
///
/// It is also used for margins. Then each field is an amount to move that edge by, so `left` and
/// `top` are usually negative. Adding a margin to a rectangle with `+` grows the rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds2D {
    /// The leftmost grid column.
    pub left: i32,
    /// The top grid row.
    pub top: i32,
    /// The rightmost grid column.
    pub right: i32,
    /// The bottom grid row.
    pub bottom: i32,
}

impl Bounds2D {
    /// All four fields zero. As a margin, it changes nothing.
    pub const ZERO: Bounds2D = Bounds2D {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    /// The smallest rectangle that contains both.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Bounds2D {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

impl Add for Bounds2D {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Bounds2D {
            left: self.left + rhs.left,
            top: self.top + rhs.top,
            right: self.right + rhs.right,
            bottom: self.bottom + rhs.bottom,
        }
    }
}

/// The grid point of a world square.
#[inline]
pub(crate) const fn world_to_grid(world_x: i32, world_y: i32) -> (i32, i32) {
    (world_x - world_y, world_x + world_y)
}

/// The world square at a grid point, or `None` for points between diamonds.
#[inline]
pub(crate) const fn grid_to_world(grid_col: i32, grid_row: i32) -> Option<(i32, i32)> {
    if (grid_col + grid_row) & 1 != 0 {
        return None;
    }
    // Both sums are even here, so shifting right by one divides exactly, including for
    // negative values.
    Some(((grid_col + grid_row) >> 1, (grid_row - grid_col) >> 1))
}

/// The grid rectangle around the block of world squares from `(x1, y1)` to `(x2, y2)`.
///
/// On the grid the block is a diamond. Each edge of the rectangle comes from one corner of the
/// block. For example, the leftmost column is at the corner with the smallest `x` and largest
/// `y`.
pub(crate) const fn square_grid_bounds(x1: i32, y1: i32, x2: i32, y2: i32) -> Bounds2D {
    Bounds2D {
        left: x1 - y2,
        top: x1 + y1,
        right: x2 - y1,
        bottom: x2 + y2,
    }
}

/// The grid rectangle around cell `(cell_x, cell_y)`, for cells `cell_squares` squares across.
pub const fn cell_grid_bounds(cell_x: i32, cell_y: i32, cell_squares: i32) -> Bounds2D {
    let x1 = cell_x * cell_squares;
    let y1 = cell_y * cell_squares;
    square_grid_bounds(x1, y1, x1 + cell_squares - 1, y1 + cell_squares - 1)
}

/// The grid rectangle around every cell in `cells`, or `None` if there are no cells.
pub fn map_grid_bounds<I>(cells: I, cell_squares: i32) -> Option<Bounds2D>
where
    I: IntoIterator<Item = (i32, i32)>,
{
    cells
        .into_iter()
        .map(|(cell_x, cell_y)| cell_grid_bounds(cell_x, cell_y, cell_squares))
        .reduce(Bounds2D::union)
}

/// Rounds `origin` down to a multiple of `align`. An `align` of 0 leaves it unchanged.
///
/// This rounds down even for negative values. The origin is the image's top-left corner, and
/// rounding toward zero would move it into the map and cut off its left or top edge.
#[inline]
pub(crate) const fn align_origin(origin: i32, align: i32) -> i32 {
    if align == 0 {
        origin
    } else {
        align * div_floor_i32(origin, align)
    }
}

/// Where the image sits on the isometric grid, and how it is cut into tiles.
///
/// The image's top-left corner, its origin, is a grid point. It is placed on a multiple of
/// `cols_per_tile * 2^(align_levels - 1)` columns, and likewise for rows. This keeps tile edges
/// in the same places on the native level and the next `align_levels - 1` levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsoLayout {
    scale: IsoScale,
    origin_col: i32,
    origin_row: i32,
    cols_per_tile: i32,
    rows_per_tile: i32,
    grid_cols: i32,
    grid_rows: i32,
    margin: Bounds2D,
}

impl IsoLayout {
    /// Lays out an image covering `bounds`, cut into square tiles of `tile_size` pixels.
    ///
    /// `bounds` must already include the output margin from
    /// [`margins::output_margin`](super::margins::output_margin). `margin` is the render margin
    /// from [`margins::render_margin`](super::margins::render_margin).
    ///
    /// Returns `None` if `tile_size` is not a whole number of grid steps both ways.
    pub fn new(
        bounds: Bounds2D,
        scale: IsoScale,
        tile_size: u32,
        align_levels: u32,
        margin: Bounds2D,
    ) -> Option<Self> {
        if !scale.divides_tile(tile_size) {
            return None;
        }
        let tile_px = i32::try_from(tile_size).ok()?;

        let cols_per_tile = tile_px / scale.step_x();
        let rows_per_tile = tile_px / scale.step_y();

        // With 3 aligned levels, every 4th tile edge on this level is also a tile edge two
        // levels up.
        let align_tiles = 1_i32 << align_levels.saturating_sub(1).min(20);
        let origin_col = align_origin(bounds.left, cols_per_tile * align_tiles);
        let origin_row = align_origin(bounds.top, rows_per_tile * align_tiles);

        Some(IsoLayout {
            scale,
            origin_col,
            origin_row,
            cols_per_tile,
            rows_per_tile,
            grid_cols: bounds.right - origin_col + 1,
            grid_rows: bounds.bottom - origin_row + 1,
            margin,
        })
    }

    /// The scale this layout was built for.
    pub const fn scale(&self) -> IsoScale {
        self.scale
    }

    /// The grid column at the image's left edge.
    pub const fn origin_col(&self) -> i32 {
        self.origin_col
    }

    /// The grid row at the image's top edge.
    pub const fn origin_row(&self) -> i32 {
        self.origin_row
    }

    /// How many grid columns one tile spans.
    pub const fn cols_per_tile(&self) -> i32 {
        self.cols_per_tile
    }

    /// How many grid rows one tile spans.
    pub const fn rows_per_tile(&self) -> i32 {
        self.rows_per_tile
    }

    /// The width and height of a tile in pixels.
    pub const fn tile_size(&self) -> u32 {
        // Never negative, because `new` rejects a tile size of 0.
        (self.cols_per_tile * self.scale.step_x()).cast_unsigned()
    }

    /// How many extra grid points past each edge of a tile are checked for sprites that reach
    /// into it. See [`margins::render_margin`](super::margins::render_margin).
    pub const fn margin(&self) -> Bounds2D {
        self.margin
    }

    /// The full-resolution image size in pixels, as `(width, height)`.
    pub fn canonical_pixels(&self) -> (u32, u32) {
        let width = i64::from(self.grid_cols).saturating_mul(i64::from(self.scale.step_x()));
        let height = i64::from(self.grid_rows).saturating_mul(i64::from(self.scale.step_y()));
        (clamp_to_u32(width), clamp_to_u32(height))
    }

    /// The grid point at the top-left corner of tile `(tile_col, tile_row)` when drawing
    /// `layer`.
    ///
    /// Higher layers are drawn higher up the image. Moving the tile down the grid by the same
    /// amount brings a higher layer's squares into it.
    #[inline]
    pub const fn tile_origin(&self, tile_col: i32, tile_row: i32, layer: i32) -> (i32, i32) {
        (
            self.origin_col + self.cols_per_tile * tile_col,
            self.origin_row + self.rows_per_tile * tile_row + GRID_ROWS_PER_LAYER * layer,
        )
    }

    /// Every world square whose sprites might reach into tile `(tile_col, tile_row)` at `layer`.
    ///
    /// Squares come one grid row at a time, top to bottom. That is also the order to draw them
    /// in: a square lower on the grid is nearer the viewer and covers the squares behind it.
    pub(crate) fn squares_for_tile(
        &self,
        tile_col: i32,
        tile_row: i32,
        layer: i32,
    ) -> impl Iterator<Item = SquareSample> {
        let (origin_col, origin_row) = self.tile_origin(tile_col, tile_row, layer);

        // Copied, so the returned iterator does not borrow `self`.
        let (step_x, step_y) = (self.scale.step_x(), self.scale.step_y());
        let half_diamond = self.scale.diamond_height() / 2;

        let margin = self.margin;
        let col_start = origin_col + margin.left;
        let col_end = origin_col + self.cols_per_tile + margin.right;
        let row_start = origin_row + margin.top;
        let row_end = origin_row + self.rows_per_tile + margin.bottom;

        (row_start..=row_end).flat_map(move |grid_row| {
            (col_start..=col_end).filter_map(move |grid_col| {
                let (world_x, world_y) = grid_to_world(grid_col, grid_row)?;

                // The anchor is the middle of the diamond, half a diamond below its top tip.
                // Sprite offsets are measured from this point.
                Some(SquareSample {
                    world_x,
                    world_y,
                    anchor_x: (grid_col - origin_col) * step_x,
                    anchor_y: (grid_row - origin_row) * step_y + half_diamond,
                })
            })
        })
    }

    /// The tile that contains the grid point of world square `(world_x, world_y)` when drawing
    /// `layer`.
    ///
    /// The square's sprites may also reach into neighboring tiles.
    pub fn tile_for_world_square(&self, world_x: i32, world_y: i32, layer: i32) -> (i32, i32) {
        let (grid_col, grid_row) = world_to_grid(world_x, world_y);
        let col = div_floor_i32(grid_col - self.origin_col, self.cols_per_tile);
        let row = div_floor_i32(
            grid_row - self.origin_row - GRID_ROWS_PER_LAYER * layer,
            self.rows_per_tile,
        );
        (col, row)
    }
}

/// Clamps a pixel size to `1..=u32::MAX`.
fn clamp_to_u32(value: i64) -> u32 {
    u32::try_from(value.clamp(1, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// One world square to draw into a tile, and where its anchor is in the tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SquareSample {
    pub world_x: i32,
    pub world_y: i32,
    /// The anchor's x position in pixels, from the tile's left edge.
    pub anchor_x: i32,
    /// The anchor's y position in pixels, from the tile's top edge.
    pub anchor_y: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_projection_round_trips_on_even_parity_points() {
        for world_x in -4..4 {
            for world_y in -4..4 {
                let (col, row) = world_to_grid(world_x, world_y);
                assert_eq!(grid_to_world(col, row), Some((world_x, world_y)));
            }
        }
    }

    #[test]
    fn odd_parity_grid_points_have_no_square() {
        assert_eq!(grid_to_world(0, 1), None);
        assert_eq!(grid_to_world(1, 0), None);
        assert_eq!(grid_to_world(-1, 4), None);
    }

    #[test]
    fn cell_bounds_match_the_hand_rotated_corners() {
        // A Build 42 cell at (0, 0) holds squares 0 to 255 on both axes.
        let bounds = cell_grid_bounds(0, 0, 256);
        assert_eq!(
            bounds,
            Bounds2D {
                left: -255,
                top: 0,
                right: 255,
                bottom: 510,
            }
        );
        // A Build 41 cell is 300 squares across.
        assert_eq!(cell_grid_bounds(0, 0, 300).right, 299);
    }

    #[test]
    fn the_origin_aligns_down_through_zero() {
        assert_eq!(align_origin(-255, 16), -256);
        assert_eq!(align_origin(255, 16), 240);
        assert_eq!(align_origin(-16, 16), -16);
        assert_eq!(align_origin(7, 0), 7);
    }

    #[test]
    fn a_tile_size_that_is_not_a_whole_grid_step_is_rejected() {
        let at = |scale, size| IsoLayout::new(Bounds2D::ZERO, scale, size, 3, Bounds2D::ZERO);

        assert!(at(IsoScale::X2, 100).is_none());
        assert!(at(IsoScale::X2, 0).is_none());
        assert!(at(IsoScale::X2, 1024).is_some());

        // 96 is 3 steps across and 6 down at 1x, but not a whole number of 2x steps.
        assert!(at(IsoScale::X1, 96).is_some());
        assert!(at(IsoScale::X2, 96).is_none());
    }

    #[test]
    fn a_tile_samples_its_own_footprint_plus_the_margin() {
        let layout = IsoLayout::new(Bounds2D::ZERO, IsoScale::X2, 1024, 1, Bounds2D::ZERO)
            .expect("1024 is a whole number of grid steps");

        // A tile spans 16 grid columns and 32 rows, both ends included. About half the points
        // fall between diamonds.
        let squares: Vec<_> = layout.squares_for_tile(0, 0, 0).collect();
        assert_eq!(squares.len(), (16 + 1) * (32 + 1) / 2 + 1);

        // The first sample sits at the tile's own origin.
        let first = squares.first().copied().expect("non-empty");
        assert_eq!(
            (first.anchor_x, first.anchor_y),
            (0, IsoScale::X2.diamond_height() / 2)
        );
    }

    /// The same tile at 1x must cover the same world squares, at half the pixel positions.
    #[test]
    fn halving_the_scale_halves_the_pixels_and_keeps_the_squares() {
        let bounds = cell_grid_bounds(30, 20, 256);
        let big = IsoLayout::new(bounds, IsoScale::X2, 1024, 3, Bounds2D::ZERO).expect("valid");
        let small = IsoLayout::new(bounds, IsoScale::X1, 512, 3, Bounds2D::ZERO).expect("valid");

        // Grid units do not depend on scale, so the two layouts agree on the origin and on the
        // grid area of a tile.
        assert_eq!(big.origin_col(), small.origin_col());
        assert_eq!(big.origin_row(), small.origin_row());
        assert_eq!(big.cols_per_tile(), small.cols_per_tile());
        assert_eq!(big.rows_per_tile(), small.rows_per_tile());

        let (bw, bh) = big.canonical_pixels();
        let (sw, sh) = small.canonical_pixels();
        assert_eq!((bw / 2, bh / 2), (sw, sh));

        let at_big: Vec<_> = big.squares_for_tile(1, 2, 0).collect();
        let at_small: Vec<_> = small.squares_for_tile(1, 2, 0).collect();
        assert_eq!(at_big.len(), at_small.len());

        for (b, s) in at_big.iter().zip(&at_small) {
            assert_eq!((b.world_x, b.world_y), (s.world_x, s.world_y));
            assert_eq!((b.anchor_x / 2, b.anchor_y / 2), (s.anchor_x, s.anchor_y));
        }
    }

    #[test]
    fn a_layer_step_shifts_the_window_by_one_layer_at_every_scale() {
        for scale in [IsoScale::X2, IsoScale::X1] {
            let layout = IsoLayout::new(Bounds2D::ZERO, scale, 512, 3, Bounds2D::ZERO)
                .expect("512 divides both scales");
            let (col0, row0) = layout.tile_origin(2, 3, 0);
            let (col1, row1) = layout.tile_origin(2, 3, 1);

            assert_eq!(col1, col0);
            assert_eq!(row1 - row0, GRID_ROWS_PER_LAYER);
            assert_eq!((row1 - row0) * scale.step_y(), scale.layer_height_px());
        }

        assert_eq!(IsoScale::X2.layer_height_px(), 192);
        assert_eq!(IsoScale::X1.layer_height_px(), 96);
    }

    #[test]
    fn tile_lookup_inverts_the_sampling_window() {
        let bounds = cell_grid_bounds(30, 20, 256);
        let layout = IsoLayout::new(bounds, IsoScale::X2, 1024, 3, Bounds2D::ZERO).expect("valid");

        for layer in [0, 1, 4] {
            for (world_x, world_y) in [(7680, 5120), (7900, 5300), (7935, 5375)] {
                let (col, row) = layout.tile_for_world_square(world_x, world_y, layer);
                let hit = layout
                    .squares_for_tile(col, row, layer)
                    .any(|s| (s.world_x, s.world_y) == (world_x, world_y));
                assert!(
                    hit,
                    "({world_x},{world_y}) at layer {layer} missed tile ({col},{row})"
                );
            }
        }
    }
}
