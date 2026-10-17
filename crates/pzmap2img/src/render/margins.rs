//! How far past a tile, or past the map, the renderer has to look for sprites.
//!
//! Each sprite belongs to one square but can be much larger than the square's diamond. A tree
//! or a tall building sticks up well above its square. So the renderer needs two margins:
//!
//! - The render margin. When drawing one tile, the renderer also checks squares just outside
//!   the tile, because their sprites can reach into it.
//! - The output margin. The image extends past the map's edge, so sprites on the edge squares
//!   are not cut off by the image border.
//!
//! Both margins come from a [`SpriteExtent`]: how far, in pixels, the largest sprites reach
//! from their square. [`SpriteExtent::measured`] finds it from the loaded sprites.
//! [`SpriteExtent::legacy_standard`] and [`SpriteExtent::legacy_large`] use the fixed sizes that
//! `pzmap2dzi` assumes, which gives the same image layout as `pzmap2dzi`. Turning pixels into
//! grid units needs the [`IsoScale`], so every function here takes one.

use crate::{
    render::geometry::{
        Bounds2D,
        GRID_ROWS_PER_LAYER,
        IsoScale,
    },
    textures::TextureLibrary,
};

/// How far, in pixels, sprites reach from the anchor point of their square, in each direction.
///
/// The anchor is the middle of the square's diamond. `down` is usually zero. Anything a sprite
/// drew below its anchor would be covered by the squares in front of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpriteExtent {
    /// Pixels to the left of the anchor.
    pub left: u32,
    /// Pixels above the anchor.
    pub up: u32,
    /// Pixels to the right of the anchor.
    pub right: u32,
    /// Pixels below the anchor.
    pub down: u32,
}

impl SpriteExtent {
    /// The reach of a `w` x `h` sprite whose anchor is the middle of its bottom edge. This is
    /// how `pzmap2dzi` places every sprite.
    pub const fn centered(w: u32, h: u32) -> Self {
        SpriteExtent {
            left: w / 2,
            up: h,
            right: w / 2,
            down: 0,
        }
    }

    /// The sprite size `pzmap2dzi` assumes by default: two grid steps wide and eight high.
    pub const fn legacy_standard(scale: IsoScale) -> Self {
        SpriteExtent::centered(
            (scale.step_x() * 2).cast_unsigned(),
            (scale.step_y() * 8).cast_unsigned(),
        )
    }

    /// The sprite size `pzmap2dzi` assumes when jumbo trees are on: six grid steps wide and
    /// sixteen high.
    pub const fn legacy_large(scale: IsoScale) -> Self {
        SpriteExtent::centered(
            (scale.step_x() * 6).cast_unsigned(),
            (scale.step_y() * 16).cast_unsigned(),
        )
    }

    /// The furthest any sprite in `textures` reaches, in each direction.
    pub fn measured(textures: &TextureLibrary) -> Self {
        let mut extent = SpriteExtent::default();

        for meta in textures.metas() {
            if meta.is_empty() {
                continue;
            }

            let (ox, oy) = (i32::from(meta.offset_x), i32::from(meta.offset_y));
            let (w, h) = (i32::from(meta.width), i32::from(meta.height));

            extent.left = extent.left.max((-ox).max(0).cast_unsigned());
            extent.up = extent.up.max((-oy).max(0).cast_unsigned());
            extent.right = extent.right.max((ox + w).max(0).cast_unsigned());
            extent.down = extent.down.max((oy + h).max(0).cast_unsigned());
        }

        extent
    }

    /// The reach in whole grid steps at `scale`, rounded up.
    fn in_grid_cells(self, scale: IsoScale) -> Bounds2D {
        // A reach of half a step still needs a whole step of margin, or sprites get cut off at
        // tile edges.
        let (step_x, step_y) = (
            scale.step_x().cast_unsigned(),
            scale.step_y().cast_unsigned(),
        );

        Bounds2D {
            left: self.left.div_ceil(step_x).cast_signed(),
            top: self.up.div_ceil(step_y).cast_signed(),
            right: self.right.div_ceil(step_x).cast_signed(),
            bottom: self.down.div_ceil(step_y).cast_signed(),
        }
    }
}

/// How far past each edge of a tile, in grid units, to look for squares whose sprites reach
/// into it.
///
/// A sprite that reaches up can draw into a tile from a square below the tile. So the upward
/// reach sets the `bottom` margin, and the downward reach sets `top`. The small adjustments of 1
/// and 2 match `pzmap2dzi`.
pub fn render_margin(extent: SpriteExtent, scale: IsoScale) -> Bounds2D {
    let cells = extent.in_grid_cells(scale);
    Bounds2D {
        left: -(cells.left - 1).max(0),
        top: -(cells.bottom - 2).max(0),
        right: (cells.right - 1).max(0),
        bottom: (cells.top - 2).max(0),
    }
}

/// How far, in grid units, the image extends past the map's edges.
///
/// Besides the sprite reach, this leaves room for every layer. Each layer above the ground is
/// drawn [`GRID_ROWS_PER_LAYER`] rows higher, and each basement layer that much lower.
/// `min_layer` is the lowest layer, and `max_layer` is one past the highest, as in a lotheader.
pub fn output_margin(
    extent: SpriteExtent,
    scale: IsoScale,
    min_layer: i32,
    max_layer: i32,
) -> Bounds2D {
    let cells = extent.in_grid_cells(scale);
    Bounds2D {
        left: -cells.left,
        top: 1 - GRID_ROWS_PER_LAYER.saturating_mul(max_layer) - cells.top,
        right: cells.right,
        bottom: GRID_ROWS_PER_LAYER.saturating_mul(-min_layer) + 1 + cells.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `pzmap2dzi`'s sprite size, the margins must match `pzmap2dzi`'s exactly. Otherwise
    /// every tile in the pyramid would be at a different address.
    #[test]
    fn the_large_texture_box_reproduces_pzmap2dzi() {
        let large = SpriteExtent::legacy_large(IsoScale::X2);
        assert_eq!(large, SpriteExtent::centered(384, 512));

        assert_eq!(
            render_margin(large, IsoScale::X2),
            Bounds2D {
                left: -2,
                top: 0,
                right: 2,
                bottom: 14,
            }
        );

        assert_eq!(
            output_margin(large, IsoScale::X2, 0, 8),
            Bounds2D {
                left: -3,
                top: 1 - 6 * 8 - 16,
                right: 3,
                bottom: 1,
            }
        );
    }

    #[test]
    fn the_standard_texture_box_reproduces_pzmap2dzi() {
        let standard = SpriteExtent::legacy_standard(IsoScale::X2);
        assert_eq!(standard, SpriteExtent::centered(128, 256));

        assert_eq!(
            render_margin(standard, IsoScale::X2),
            Bounds2D {
                left: 0,
                top: 0,
                right: 0,
                bottom: 6,
            }
        );
    }

    /// Margins are in grid units, so halving both the sprites and the grid step must not change
    /// them. Otherwise a 1x render would not be the 2x picture at half size.
    #[test]
    fn the_margins_are_scale_invariant() {
        let big = SpriteExtent::legacy_large(IsoScale::X2);
        let small = SpriteExtent::legacy_large(IsoScale::X1);
        assert_eq!(small, SpriteExtent::centered(192, 256));

        assert_eq!(
            render_margin(big, IsoScale::X2),
            render_margin(small, IsoScale::X1)
        );

        for (min, max) in [(0, 8), (-32, 32)] {
            assert_eq!(
                output_margin(big, IsoScale::X2, min, max),
                output_margin(small, IsoScale::X1, min, max)
            );
        }
    }

    /// 1x sprites used with the 2x scale give a margin half as deep as needed, so tall sprites
    /// get cut off at tile edges.
    #[test]
    fn a_mismatched_scale_produces_too_small_a_margin() {
        let small = SpriteExtent::legacy_large(IsoScale::X1);
        let correct = render_margin(small, IsoScale::X1);
        let mismatched = render_margin(small, IsoScale::X2);
        assert!(
            mismatched.bottom < correct.bottom,
            "{mismatched:?} should be tighter than {correct:?}"
        );
    }

    /// Basement layers, which Build 42 maps have, extend the image downward, not upward.
    #[test]
    fn negative_layers_extend_the_bottom() {
        let large = SpriteExtent::legacy_large(IsoScale::X2);
        let shallow = output_margin(large, IsoScale::X2, 0, 8);
        let deep = output_margin(large, IsoScale::X2, -32, 8);
        assert_eq!(deep.bottom - shallow.bottom, 6 * 32);
        assert_eq!(deep.top, shallow.top);
    }

    #[test]
    fn a_sprite_reaching_half_a_cell_still_gets_a_whole_cell() {
        let extent = SpriteExtent {
            left: 65,
            up: 33,
            right: 1,
            down: 0,
        };
        let cells = extent.in_grid_cells(IsoScale::X2);
        assert_eq!((cells.left, cells.top, cells.right), (2, 2, 1));
    }
}
