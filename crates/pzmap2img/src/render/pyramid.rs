//! The shape of a Deep Zoom pyramid: its levels, their sizes, and their tiles.
//!
//! A Deep Zoom image is stored as a pyramid of levels. The highest-numbered level, `max_level`,
//! is the full-resolution image, called the native level or canonical size. Each level below it
//! is half the width and height of the one above, rounded up, down to level 0 at 1x1 pixel.
//! Viewers load whichever level matches the zoom.
//!
//! Every level is cut into square tiles of the same size, starting at the top-left corner. The
//! tiles on the right and bottom edges are cropped to the level, so they can be smaller. A tile
//! is named by a [`TileAddress`]: its level, column and row. Tile `(col, row)` on one level is
//! built from the four tiles `(2col, 2row)` to `(2col + 1, 2row + 1)` on the level below.
//!
//! This module also builds a tile from its four children, by averaging each 2x2 block of
//! pixels.

use std::borrow::Cow;

use crate::render::blend::{
    BPP,
    Canvas,
};

/// `ceil(log2(n))`, or 0 for `n <= 1`.
///
/// This rounds up, unlike `n.ilog2()`. Rounding down would leave a pyramid whose size is not a
/// power of two one level short, with level 0 at 2x2 instead of 1x1.
#[inline]
pub(crate) const fn ceil_log2(n: u32) -> u32 {
    if n <= 1 {
        return 0;
    }
    (n - 1).ilog2() + 1
}

/// The size and levels of a Deep Zoom pyramid.
///
/// # Examples
///
/// ```
/// use pzmap2img::render::pyramid::Pyramid;
///
/// let pyramid = Pyramid::new(5000, 3000, 1024);
/// // 5000 pixels needs 13 halvings to reach 1, so there are levels 0 to 13.
/// assert_eq!(pyramid.max_level(), 13);
/// assert_eq!(pyramid.tiles_across(13), (5, 3));
/// assert_eq!(pyramid.tiles_across(0), (1, 1));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pyramid {
    width: u32,
    height: u32,
    tile_size: u32,
    max_level: u32,
}

impl Pyramid {
    /// A pyramid for a `width` x `height` image with square tiles of `tile_size` pixels. Each
    /// value is raised to at least 1.
    pub const fn new(width: u32, height: u32, tile_size: u32) -> Self {
        let width = if width == 0 { 1 } else { width };
        let height = if height == 0 { 1 } else { height };
        let longest = if width > height { width } else { height };

        Pyramid {
            width,
            height,
            tile_size: if tile_size == 0 { 1 } else { tile_size },
            max_level: ceil_log2(longest),
        }
    }

    /// The full-resolution image size in pixels, as `(width, height)`.
    pub const fn canonical(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The width and height of a tile in pixels.
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }

    /// The number of the native, full-resolution level. It is also the number of levels minus
    /// one.
    pub const fn max_level(&self) -> u32 {
        self.max_level
    }

    /// The pixel size of `level`: the full size halved `max_level - level` times, rounding up.
    ///
    /// Halving `k` times with rounding up gives the same result as dividing once by `2^k` with
    /// rounding up, so this matches building the levels one at a time.
    pub(crate) const fn level_pixels(&self, level: u32) -> (u32, u32) {
        let halvings = self.max_level.saturating_sub(level);
        // After 32 halvings, every `u32` size is 1.
        if halvings >= 32 {
            return (1, 1);
        }
        let divisor = 1_u32 << halvings;

        let width = self.width.div_ceil(divisor);
        let height = self.height.div_ceil(divisor);
        (
            if width == 0 { 1 } else { width },
            if height == 0 { 1 } else { height },
        )
    }

    /// How many tiles wide and high `level` is.
    pub const fn tiles_across(&self, level: u32) -> (u32, u32) {
        let (width, height) = self.level_pixels(level);
        let cols = width.div_ceil(self.tile_size);
        let rows = height.div_ceil(self.tile_size);
        (
            if cols == 0 { 1 } else { cols },
            if rows == 0 { 1 } else { rows },
        )
    }

    /// The XML of the `.dzi` descriptor file, with `format` as the tile file extension.
    ///
    /// The descriptor says `Overlap="0"`: neighboring tiles do not repeat each other's edge
    /// pixels.
    pub fn descriptor(&self, format: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Image xmlns=\"http://schemas.microsoft.com/deepzoom/2008\" \
             TileSize=\"{tile}\" Overlap=\"0\" Format=\"{format}\">\n  \
             <Size Width=\"{width}\" Height=\"{height}\"/>\n\
             </Image>\n",
            tile = self.tile_size,
            width = self.width,
            height = self.height,
        )
    }
}

/// One tile's address in the pyramid: its level, and its column and row within that level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileAddress {
    /// The pyramid level. 0 is the smallest.
    pub level: u32,
    /// The column, counting from 0 at the left.
    pub col: u32,
    /// The row, counting from 0 at the top.
    pub row: u32,
}

impl TileAddress {
    /// The tile at column `col` and row `row` of `level`.
    pub const fn new(level: u32, col: u32, row: u32) -> Self {
        TileAddress { level, col, row }
    }

    /// The four tiles on the next level that this tile is built from.
    ///
    /// They come in reading order: top left, top right, bottom left, bottom right.
    pub const fn children(&self) -> [TileAddress; 4] {
        let level = self.level + 1;
        let (col, row) = (self.col * 2, self.row * 2);
        [
            TileAddress::new(level, col, row),
            TileAddress::new(level, col + 1, row),
            TileAddress::new(level, col, row + 1),
            TileAddress::new(level, col + 1, row + 1),
        ]
    }
}

impl std::fmt::Display for TileAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}_{}", self.level, self.col, self.row)
    }
}

/// Crops a tile to the part that lies inside its level, or returns `None` if none of it does.
///
/// Only tiles on the right and bottom edges of a level get smaller. Every other tile is returned
/// as a borrow, without copying it.
pub(crate) fn crop_to_level<'a>(
    tile: &'a Canvas,
    pyramid: &Pyramid,
    address: TileAddress,
) -> Option<Cow<'a, Canvas>> {
    let (level_w, level_h) = pyramid.level_pixels(address.level);
    let tile_size = pyramid.tile_size;

    let origin_x = address.col.checked_mul(tile_size)?;
    let origin_y = address.row.checked_mul(tile_size)?;

    let keep_w = level_w.saturating_sub(origin_x).min(tile.width());
    let keep_h = level_h.saturating_sub(origin_y).min(tile.height());

    if keep_w == 0 || keep_h == 0 {
        return None;
    }
    if keep_w == tile.width() && keep_h == tile.height() {
        return Some(Cow::Borrowed(tile));
    }

    let mut cropped = Canvas::new(keep_w, keep_h);
    let src_stride = tile.width() as usize * BPP;
    let dst_stride = keep_w as usize * BPP;

    for row in 0..keep_h as usize {
        let src = row * src_stride;
        let dst = row * dst_stride;
        let (Some(from), Some(into)) = (
            tile.pixels().get(src..src + dst_stride),
            cropped.pixels_mut().get_mut(dst..dst + dst_stride),
        ) else {
            continue;
        };
        into.copy_from_slice(from);
    }

    Some(Cow::Owned(cropped))
}

/// Builds a parent tile from its four children, or returns `None` if all four are missing.
///
/// A missing child counts as transparent. The averaging works on premultiplied pixels.
/// Averaging straight-alpha pixels would mix in the color of transparent pixels, which is
/// usually black, and leave dark fringes around sprites.
pub(crate) fn merge_children(children: [Option<&Canvas>; 4], tile_size: u32) -> Option<Canvas> {
    if children.iter().all(Option::is_none) {
        return None;
    }

    let mut parent = Canvas::new(tile_size, tile_size);
    for (quadrant, child) in children.iter().enumerate() {
        if let Some(child) = child {
            halve_into(child, &mut parent, quadrant);
        }
    }

    Some(parent)
}

/// Scales one child down by half into its quarter of `parent`, which must start transparent.
///
/// Quarters are numbered as in [`TileAddress::children`]: 0 is top left, 1 top right, 2 bottom
/// left, 3 bottom right. Each output pixel is the average of a 2x2 block of child pixels. A child
/// smaller than the tile size is a cropped edge tile. It fills only the top-left part of its
/// quarter, and the rest stays transparent.
///
/// The result is the same as pasting all four children into one image twice the tile size and
/// halving that. No 2x2 block spans two children, so each child can be halved on its own, without
/// the large temporary image.
pub(crate) fn halve_into(child: &Canvas, parent: &mut Canvas, quadrant: usize) {
    let half_w = parent.width() / 2;
    let half_h = parent.height() / 2;
    if half_w == 0 || half_h == 0 {
        return;
    }

    // A child larger than the tile size is cut down to it, as pasting it into the double-size
    // image would.
    let src_w = child.width().min(half_w * 2) as usize;
    let src_h = child.height().min(half_h * 2) as usize;

    // Round up. A child with an odd width has a last 2x2 block that is half inside it. That
    // block still gives a real, partly transparent pixel.
    let out_w = src_w.div_ceil(2).min(half_w as usize);
    let out_h = src_h.div_ceil(2).min(half_h as usize);

    let origin_x = (quadrant % 2) * half_w as usize;
    let origin_y = (quadrant / 2) * half_h as usize;

    let src_stride = child.width() as usize * BPP;
    let dst_stride = parent.width() as usize * BPP;

    // Full-size children take a faster path. Cropped edge children use the general loop
    // below, which checks each sample.
    if src_w == half_w as usize * 2
        && src_h == half_h as usize * 2
        && child.width() as usize == src_w
    {
        halve_full_into(child, parent, origin_x, origin_y);
        return;
    }

    for row in 0..out_h {
        let top = row * 2;
        let bottom = top + 1;

        for col in 0..out_w {
            let left = col * 2;
            let right = left + 1;

            // Samples outside the child count as transparent.
            let sample = |y: usize, x: usize| -> Option<&[u8]> {
                if y >= src_h || x >= src_w {
                    return None;
                }
                let at = y * src_stride + x * BPP;
                child.pixels().get(at..at + BPP)
            };

            let quad = [
                sample(top, left),
                sample(top, right),
                sample(bottom, left),
                sample(bottom, right),
            ];

            let at = (origin_y + row) * dst_stride + (origin_x + col) * BPP;
            let Some(target) = parent.pixels_mut().get_mut(at..at + BPP) else {
                continue;
            };

            for channel in 0..BPP {
                let sum: u32 = quad
                    .iter()
                    .flatten()
                    .filter_map(|p| p.get(channel))
                    .map(|v| u32::from(*v))
                    .sum();
                // The average of four samples, rounding halves up.
                let mean = (sum + 2) >> 2;
                if let Some(slot) = target.get_mut(channel) {
                    *slot = u8::try_from(mean.min(255)).unwrap_or(255);
                }
            }
        }
    }
}

/// The fast path of [`halve_into`], for a child exactly the tile size.
///
/// Every 2x2 block lies inside the child, so the loop reads two rows at a time with no checks
/// per sample.
fn halve_full_into(child: &Canvas, parent: &mut Canvas, origin_x: usize, origin_y: usize) {
    let half_w = child.width() as usize / 2;
    let half_h = child.height() as usize / 2;
    let src_stride = child.width() as usize * BPP;
    let dst_stride = parent.width() as usize * BPP;

    let src = child.pixels();
    let dst = parent.pixels_mut();

    for (row, src_pair) in src.chunks_exact(src_stride * 2).take(half_h).enumerate() {
        let (top, bottom) = src_pair.split_at(src_stride);

        let at = (origin_y + row) * dst_stride + origin_x * BPP;
        let Some(dst_row) = dst.get_mut(at..at + half_w * BPP) else {
            // The quarter always fits inside the parent. Skipping the row instead of panicking
            // matches the general path.
            continue;
        };

        let top_pairs = top.as_chunks::<{ 2 * BPP }>().0;
        let bottom_pairs = bottom.as_chunks::<{ 2 * BPP }>().0;
        let out = dst_row.as_chunks_mut::<BPP>().0;

        for ((target, above), below) in out.iter_mut().zip(top_pairs).zip(bottom_pairs) {
            let [a0, a1, a2, a3, a4, a5, a6, a7] = *above;
            let [b0, b1, b2, b3, b4, b5, b6, b7] = *below;
            *target = [
                mean4(a0, a4, b0, b4),
                mean4(a1, a5, b1, b5),
                mean4(a2, a6, b2, b6),
                mean4(a3, a7, b3, b7),
            ];
        }
    }
}

/// The average of four samples, rounding halves up, the same as the general path.
#[inline]
const fn mean4(a: u8, b: u8, c: u8, d: u8) -> u8 {
    let sum = a as u32 + b as u32 + c as u32 + d as u32;
    // The sum is at most 1020, so after the shift the value fits in a `u8` and the mask changes
    // nothing.
    (((sum + 2) >> 2) & 0xff) as u8
}

/// Scales a canvas down by half, averaging each 2x2 block.
///
/// Tests compare [`halve_into`] against this simpler version. The render does not use it.
#[cfg(test)]
fn halve(source: &Canvas) -> Canvas {
    let width = (source.width() / 2).max(1);
    let height = (source.height() / 2).max(1);

    let mut out = Canvas::new(width, height);
    let src_stride = source.width() as usize * BPP;
    let dst_stride = width as usize * BPP;

    for row in 0..height as usize {
        let top = row * 2 * src_stride;
        let bottom = top + src_stride;

        for col in 0..width as usize {
            let left = col * 2 * BPP;
            let right = left + BPP;

            let quad = [
                source.pixels().get(top + left..top + left + BPP),
                source.pixels().get(top + right..top + right + BPP),
                source.pixels().get(bottom + left..bottom + left + BPP),
                source.pixels().get(bottom + right..bottom + right + BPP),
            ];

            let dst = row * dst_stride + col * BPP;
            let Some(target) = out.pixels_mut().get_mut(dst..dst + BPP) else {
                continue;
            };

            for channel in 0..BPP {
                let sum: u32 = quad
                    .iter()
                    .flatten()
                    .filter_map(|p| p.get(channel))
                    .map(|v| u32::from(*v))
                    .sum();
                // The average of four samples, rounding halves up.
                let mean = (sum + 2) >> 2;
                if let Some(slot) = target.get_mut(channel) {
                    *slot = u8::try_from(mean.min(255)).unwrap_or(255);
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pastes all four children into one double-size canvas, then halves it with [`halve`].
    /// Tests compare [`merge_children`] against this.
    fn merge_reference(children: [Option<&Canvas>; 4], tile_size: u32) -> Option<Canvas> {
        if children.iter().all(Option::is_none) {
            return None;
        }

        let double = tile_size.checked_mul(2)?;
        let mut composed = Canvas::new(double, double);
        let composed_stride = double as usize * BPP;

        for (index, child) in children.iter().enumerate() {
            let Some(child) = child else { continue };

            let offset_x = (index % 2) * tile_size as usize;
            let offset_y = (index / 2) * tile_size as usize;

            let child_stride = child.width() as usize * BPP;
            let copy_w = (child.width() as usize).min(tile_size as usize) * BPP;

            for row in 0..(child.height() as usize).min(tile_size as usize) {
                let src = row * child_stride;
                let dst = (offset_y + row) * composed_stride + offset_x * BPP;
                let (Some(from), Some(into)) = (
                    child.pixels().get(src..src + copy_w),
                    composed.pixels_mut().get_mut(dst..dst + copy_w),
                ) else {
                    continue;
                };
                into.copy_from_slice(from);
            }
        }

        Some(halve(&composed))
    }

    /// A canvas of random bytes. A smooth gradient would hide most averaging bugs.
    fn noise(width: u32, height: u32, seed: u64) -> Canvas {
        let mut canvas = Canvas::new(width, height);
        canvas.pixels_mut().fill_with(crate::testing::noise(seed));
        canvas
    }

    /// `halve_into` must give exactly the same bytes as pasting and then halving. The two ways of
    /// building a pyramid must agree, and one pyramid may use both.
    #[test]
    fn halving_into_a_quadrant_matches_composing_then_halving() {
        let tile = 16;

        // Every combination of present and missing children.
        for mask in 0_u8..16 {
            let owned: Vec<Option<Canvas>> = (0_u64..4)
                .map(|i| (mask & (1 << i) != 0).then(|| noise(tile, tile, 0x5EED + i)))
                .collect();
            let children: [Option<&Canvas>; 4] = [
                owned[0].as_ref(),
                owned[1].as_ref(),
                owned[2].as_ref(),
                owned[3].as_ref(),
            ];

            let reference = merge_reference(children, tile);
            let actual = merge_children(children, tile);

            match (reference, actual) {
                (None, None) => assert_eq!(mask, 0),
                (Some(want), Some(got)) => {
                    assert_eq!(want.width(), got.width());
                    assert_eq!(want.height(), got.height());
                    assert_eq!(want.pixels(), got.pixels(), "mask {mask:b}");
                }
                _ => panic!("mask {mask:b}: one produced a tile and the other did not"),
            }
        }
    }

    /// Edge tiles are cropped, so a child can be any size up to the tile size. Odd sizes matter
    /// most, because the last 2x2 block is then half inside the child.
    #[test]
    fn halving_agrees_for_cropped_children_of_any_size() {
        let tile = 16;

        for size in [1_u32, 2, 3, 7, 8, 15, 16] {
            let cropped = noise(size, size, u64::from(size) * 977);
            let full = noise(tile, tile, 4242);

            // In every quarter, since each puts the cropped child in a different place.
            for quadrant in 0..4 {
                let mut owned: [Option<&Canvas>; 4] = [Some(&full); 4];
                if let Some(slot) = owned.get_mut(quadrant) {
                    *slot = Some(&cropped);
                }

                let want = merge_reference(owned, tile).expect("children present");
                let got = merge_children(owned, tile).expect("children present");
                assert_eq!(
                    want.pixels(),
                    got.pixels(),
                    "{size}x{size} child in quadrant {quadrant}"
                );
            }
        }
    }

    #[test]
    fn ceil_log2_is_the_ceiling() {
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(1024), 10);
        assert_eq!(ceil_log2(1025), 11);
        assert_eq!(ceil_log2(u32::MAX), 32);
    }

    #[test]
    fn every_level_is_the_ceil_half_of_the_one_above() {
        let pyramid = Pyramid::new(4321, 2109, 256);
        for level in 0..pyramid.max_level() {
            let (w, h) = pyramid.level_pixels(level);
            let (child_w, child_h) = pyramid.level_pixels(level + 1);
            assert_eq!((w, h), (child_w.div_ceil(2), child_h.div_ceil(2)));
        }
    }

    #[test]
    fn tile_counts_cover_the_level() {
        let pyramid = Pyramid::new(4321, 2109, 256);
        let level = pyramid.max_level();
        let (cols, rows) = pyramid.tiles_across(level);
        assert_eq!(
            (cols, rows),
            (4321_u32.div_ceil(256), 2109_u32.div_ceil(256))
        );
        assert_eq!(pyramid.tiles_across(0), (1, 1));
    }

    #[test]
    fn children_tile_their_parent() {
        let parent = TileAddress::new(3, 2, 5);
        assert_eq!(
            parent.children(),
            [
                TileAddress::new(4, 4, 10),
                TileAddress::new(4, 5, 10),
                TileAddress::new(4, 4, 11),
                TileAddress::new(4, 5, 11),
            ]
        );
    }

    #[test]
    fn edge_tiles_are_cropped_to_the_level() {
        let pyramid = Pyramid::new(300, 200, 256);
        let level = pyramid.max_level();
        let tile = Canvas::new(256, 256);

        let interior = crop_to_level(&tile, &pyramid, TileAddress::new(level, 0, 0))
            .expect("tile (0,0) overlaps the image");
        assert_eq!((interior.width(), interior.height()), (256, 200));

        let edge = crop_to_level(&tile, &pyramid, TileAddress::new(level, 1, 0))
            .expect("tile (1,0) overlaps the image");
        assert_eq!((edge.width(), edge.height()), (44, 200));

        assert!(crop_to_level(&tile, &pyramid, TileAddress::new(level, 2, 0)).is_none());
    }

    #[test]
    fn a_uniform_child_survives_the_halving_exactly() {
        let mut child = Canvas::new(8, 8);
        for p in child.pixels_mut().as_chunks_mut::<4>().0 {
            *p = [40, 80, 120, 255];
        }

        let merged =
            merge_children([Some(&child), None, None, None], 8).expect("one child is enough");
        assert_eq!((merged.width(), merged.height()), (8, 8));

        // The top-left quarter is the child halved. It is one color, so it is unchanged.
        assert_eq!(merged.pixels().get(0..4), Some(&[40, 80, 120, 255][..]));
        // The other three quarters had no child and stay transparent.
        let last = merged.pixels().len() - 4;
        assert_eq!(merged.pixels().get(last..), Some(&[0, 0, 0, 0][..]));
    }

    #[test]
    fn the_descriptor_names_the_canonical_size() {
        let xml = Pyramid::new(4321, 2109, 256).descriptor("png");
        assert!(xml.contains("TileSize=\"256\""));
        assert!(xml.contains("Width=\"4321\""));
        assert!(xml.contains("Height=\"2109\""));
        assert!(xml.contains("Format=\"png\""));
    }
}
