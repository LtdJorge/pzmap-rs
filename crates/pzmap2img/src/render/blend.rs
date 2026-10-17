//! The canvas a tile is drawn on, and how sprites are blended onto it.
//!
//! A canvas stores four bytes per pixel: red, green, blue and alpha. The colors are stored
//! *premultiplied*, meaning each color channel has already been multiplied by alpha. A
//! half-transparent white pixel is stored as `[128, 128, 128, 128]`, not `[255, 255, 255, 128]`.
//!
//! Premultiplied colors make the two operations the renderer does most both simple and exact.
//! Drawing a sprite over the canvas is `dst = src + dst * (1 - src_alpha)` on every channel, with
//! no division. Averaging pixels to scale a tile down works without dark edges around sprites.
//!
//! Image files store straight (not premultiplied) alpha, so [`unpremultiply`] converts each tile
//! once before encoding, and [`premultiply`] converts back after decoding.

use crate::textures::Sprite;

/// Bytes per pixel: red, green, blue and alpha.
pub const BPP: usize = 4;

/// A tile's pixels, premultiplied RGBA, stored row by row from the top with no gaps.
#[derive(Clone)]
pub struct Canvas {
    width: u32,
    height: u32,
    pixels: Box<[u8]>,
}

impl Canvas {
    /// Creates a fully transparent canvas.
    pub fn new(width: u32, height: u32) -> Self {
        let len = width as usize * height as usize * BPP;
        Canvas {
            width,
            height,
            pixels: vec![0_u8; len].into_boxed_slice(),
        }
    }

    /// Creates a canvas with no pixels.
    ///
    /// [`crate::render::pool::PooledCanvas`] swaps this in when it gives its buffer back to the
    /// pool. That way it doesn't need an `Option` to unwrap on every access.
    #[must_use]
    pub(crate) fn empty() -> Self {
        Canvas {
            width: 0,
            height: 0,
            pixels: Box::new([]),
        }
    }

    /// Wraps existing premultiplied RGBA bytes, or returns `None` if there are not exactly
    /// `width * height * BPP` of them.
    pub fn from_pixels(width: u32, height: u32, pixels: Box<[u8]>) -> Option<Self> {
        if pixels.len() != width as usize * height as usize * BPP {
            return None;
        }
        Some(Canvas {
            width,
            height,
            pixels,
        })
    }

    /// The width in pixels.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes, premultiplied RGBA, row by row from the top.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixel bytes, for writing. They should stay premultiplied, with no color channel
    /// larger than alpha.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Consumes the canvas and returns its pixel buffer.
    #[must_use]
    pub(crate) fn into_pixels(self) -> Box<[u8]> {
        self.pixels
    }

    /// Makes every pixel fully transparent, keeping the same buffer.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Whether every pixel is fully transparent. Such tiles are not written.
    ///
    /// Only alpha is checked. With premultiplied colors, a pixel with zero alpha has zero color
    /// too.
    pub fn is_transparent(&self) -> bool {
        self.pixels.as_chunks::<BPP>().0.iter().all(|p| p[3] == 0)
    }

    /// Draws `sprite` over the canvas, with the sprite's anchor at pixel `(anchor_x, anchor_y)`.
    ///
    /// The sprite's own offsets say where its top-left corner is relative to the anchor. Any
    /// part of the sprite outside the canvas is skipped.
    pub fn draw(&mut self, sprite: &Sprite<'_>, anchor_x: i32, anchor_y: i32) {
        let dst_x = anchor_x + sprite.offset_x;
        let dst_y = anchor_y + sprite.offset_y;

        let Some(clip) = Clip::new(
            dst_x,
            dst_y,
            sprite.width,
            sprite.height,
            self.width,
            self.height,
        ) else {
            return;
        };

        let src_stride = sprite.width as usize * BPP;
        let dst_stride = self.width as usize * BPP;

        for row in 0..clip.rows {
            let src_row = (clip.src_y + row) * src_stride + clip.src_x * BPP;
            let dst_row = (clip.dst_y + row) * dst_stride + clip.dst_x * BPP;
            let len = clip.cols * BPP;

            let (Some(src), Some(dst)) = (
                sprite.pixels.get(src_row..src_row + len),
                self.pixels.get_mut(dst_row..dst_row + len),
            ) else {
                // `Clip` already keeps both ranges in bounds, so this cannot happen with a valid
                // sprite. A sprite with too few pixel bytes skips rows instead of panicking.
                continue;
            };

            if sprite.opaque {
                dst.copy_from_slice(src);
            } else {
                blend_row(dst, src);
            }
        }
    }
}

/// Draws one row of sprite pixels over one row of the canvas.
///
/// The slices must be the same length. That lets the compiler skip bounds checks per pixel.
#[inline]
fn blend_row(dst: &mut [u8], src: &[u8]) {
    let src = src.as_chunks::<BPP>().0;
    let dst = dst.as_chunks_mut::<BPP>().0;

    for (d, s) in dst.iter_mut().zip(src) {
        let src_alpha = u32::from(s[3]);
        if src_alpha == 0 {
            continue;
        }
        if src_alpha == 255 {
            *d = *s;
            continue;
        }

        let inverse = 255 - src_alpha;
        for (channel, source) in d.iter_mut().zip(s.iter()) {
            *channel = source.saturating_add(mul_255(*channel, inverse));
        }
    }
}

/// `value * factor / 255`, rounded to the nearest integer, without a division.
///
/// `(t + (t >> 8)) >> 8` is a well-known way to divide by 255. It gives the exactly rounded
/// result for every `u8` input.
#[inline]
const fn mul_255(value: u8, factor: u32) -> u8 {
    let t = value as u32 * factor + 128;
    // `t` is at most 255 * 255 + 128, so the result fits in a `u8` and the mask changes nothing.
    (((t + (t >> 8)) >> 8) & 0xff) as u8
}

/// Converts RGBA pixels from straight to premultiplied alpha, in place.
///
/// This is the reverse of [`unpremultiply`]. Use it on decoded image data before drawing with
/// it.
pub fn premultiply(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<BPP>().0 {
        let alpha = u32::from(p[3]);
        if alpha == 255 {
            continue;
        }
        if alpha == 0 {
            *p = [0, 0, 0, 0];
            continue;
        }
        let (colour, _) = p.split_at_mut(3);
        for channel in colour {
            *channel = mul_255(*channel, alpha);
        }
    }
}

/// Converts RGBA pixels from premultiplied to straight alpha, in place.
///
/// Image formats such as PNG store straight alpha, so encoders call this on a copy of each tile.
///
/// Converting a premultiplied pixel to straight alpha and back gives the same pixel. The other
/// way round is not exact: premultiplying loses precision in partly transparent pixels, so a
/// channel can come back off by one.
pub fn unpremultiply(pixels: &mut [u8]) {
    for p in pixels.as_chunks_mut::<BPP>().0 {
        let alpha = u32::from(p[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        let (colour, _) = p.split_at_mut(3);
        for channel in colour {
            let value = u32::from(*channel) * 255 + alpha / 2;
            // Rounding can give 256 when the premultiplied value was rounded up, so clamp.
            *channel = u8::try_from((value / alpha).min(255)).unwrap_or(255);
        }
    }
}

/// The part of a `w` x `h` sprite, placed at `(dst_x, dst_y)`, that lies on the canvas.
struct Clip {
    src_x: usize,
    src_y: usize,
    dst_x: usize,
    dst_y: usize,
    cols: usize,
    rows: usize,
}

impl Clip {
    /// Returns `None` if the sprite is entirely outside the canvas.
    fn new(dst_x: i32, dst_y: i32, w: u32, h: u32, canvas_w: u32, canvas_h: u32) -> Option<Self> {
        // Use `i64`, because the position can be any `i32` and adding the size could overflow.
        let left = i64::from(dst_x);
        let top = i64::from(dst_y);
        let right = left + i64::from(w);
        let bottom = top + i64::from(h);

        let canvas_w = i64::from(canvas_w);
        let canvas_h = i64::from(canvas_h);

        if right <= 0 || bottom <= 0 || left >= canvas_w || top >= canvas_h {
            return None;
        }

        let x0 = left.max(0);
        let y0 = top.max(0);
        let x1 = right.min(canvas_w);
        let y1 = bottom.min(canvas_h);

        Some(Clip {
            src_x: usize::try_from(x0 - left).ok()?,
            src_y: usize::try_from(y0 - top).ok()?,
            dst_x: usize::try_from(x0).ok()?,
            dst_y: usize::try_from(y0).ok()?,
            cols: usize::try_from(x1 - x0).ok()?,
            rows: usize::try_from(y1 - y0).ok()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sprite<'a>(pixels: &'a [u8], w: u32, h: u32, ox: i32, oy: i32) -> Sprite<'a> {
        Sprite {
            pixels,
            width: w,
            height: h,
            offset_x: ox,
            offset_y: oy,
            opaque: pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
        }
    }

    #[test]
    fn an_opaque_sprite_replaces_what_it_covers() {
        let mut canvas = Canvas::new(2, 2);
        let src = [
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 255,
        ];
        canvas.draw(&sprite(&src, 2, 2, 0, 0), 0, 0);
        assert_eq!(canvas.pixels(), &src);
    }

    #[test]
    fn a_half_transparent_sprite_composites_exactly() {
        // Canvas: opaque white. Sprite: 50% gray, premultiplied (128 * 128/255 ≈ 64).
        let mut canvas = Canvas::new(1, 1);
        canvas.pixels_mut().copy_from_slice(&[255, 255, 255, 255]);

        let src = [64, 64, 64, 128];
        canvas.draw(&sprite(&src, 1, 1, 0, 0), 0, 0);

        // d = s + d * (1 - 128/255) = 64 + round(255 * 127 / 255) = 64 + 127 = 191.
        assert_eq!(canvas.pixels(), &[191, 191, 191, 255]);
    }

    #[test]
    fn a_fully_transparent_sprite_changes_nothing() {
        let mut canvas = Canvas::new(1, 1);
        canvas.pixels_mut().copy_from_slice(&[9, 9, 9, 200]);
        canvas.draw(&sprite(&[0, 0, 0, 0], 1, 1, 0, 0), 0, 0);
        assert_eq!(canvas.pixels(), &[9, 9, 9, 200]);
    }

    #[test]
    fn sprites_clip_on_every_side() {
        let src = [255_u8; 4 * 4];
        let s = sprite(&src, 2, 2, 0, 0);

        for (ax, ay) in [(-1, -1), (1, 1), (-2, 0), (0, -2), (2, 0), (0, 2)] {
            let mut canvas = Canvas::new(2, 2);
            canvas.draw(&s, ax, ay);
            let covered = canvas
                .pixels()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] == 255)
                .count();
            let expected = match (ax, ay) {
                (-1, -1) | (1, 1) => 1,
                (-2, 0) | (0, -2) | (2, 0) | (0, 2) => 0,
                _ => unreachable!(),
            };
            assert_eq!(covered, expected, "anchor ({ax},{ay})");
        }
    }

    #[test]
    fn an_absurd_anchor_does_not_overflow() {
        let mut canvas = Canvas::new(4, 4);
        let src = [255_u8; 4];
        canvas.draw(&sprite(&src, 1, 1, 0, 0), i32::MAX, i32::MAX);
        canvas.draw(&sprite(&src, 1, 1, 0, 0), i32::MIN, i32::MIN);
        assert!(canvas.is_transparent());
    }

    #[test]
    fn the_sprite_offset_moves_the_draw() {
        let mut canvas = Canvas::new(4, 4);
        let src = [255_u8; 4];
        canvas.draw(&sprite(&src, 1, 1, 2, 1), 0, 0);
        // Anchor (0, 0) plus offset (2, 1) puts the pixel at (2, 1), which is byte
        // 4 * (1 * 4 + 2) = 24.
        assert_eq!(canvas.pixels().get(24..28), Some(&[255, 255, 255, 255][..]));
    }

    #[test]
    fn transparency_detection_sees_a_single_pixel() {
        let mut canvas = Canvas::new(8, 8);
        assert!(canvas.is_transparent());
        canvas.draw(&sprite(&[0, 0, 0, 1], 1, 1, 0, 0), 7, 7);
        assert!(!canvas.is_transparent());
    }

    #[test]
    fn unpremultiply_inverts_a_clean_premultiply() {
        // 200 at alpha 128 premultiplies to round(200 * 128 / 255) = 100.
        let mut pixels = [100, 100, 100, 128];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [199, 199, 199, 128]);

        // Opaque and fully transparent pixels are left exactly alone.
        let mut edges = [7, 8, 9, 255, 1, 2, 3, 0];
        unpremultiply(&mut edges);
        assert_eq!(edges, [7, 8, 9, 255, 1, 2, 3, 0]);
    }

    /// A premultiplied pixel converted to straight alpha and back must not change. This is what
    /// lets a lossless tile be read back exactly.
    #[test]
    fn premultiply_inverts_unpremultiply() {
        for alpha in 1_u8..=255 {
            for premultiplied in 0..=alpha {
                let original = [premultiplied, premultiplied, premultiplied, alpha];
                let mut px = original;
                unpremultiply(&mut px);
                premultiply(&mut px);
                assert_eq!(px, original, "{premultiplied} at alpha {alpha}");
            }
        }
    }

    #[test]
    fn unpremultiply_never_exceeds_full_scale() {
        let mut pixels = [255, 255, 255, 1];
        unpremultiply(&mut pixels);
        assert_eq!(pixels, [255, 255, 255, 1]);
    }
}
