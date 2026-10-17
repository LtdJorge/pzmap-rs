//! Decodes pages and works out the preview geometry for both frontends.
//!
//! A texture's rectangle comes from the file, so it is clamped to the decoded page before use.
//! The preview is drawn over an opaque background, so that a fully transparent sprite does not
//! look the same as a black one.

use crate::model::{
    View,
    Zoom,
};
use anyhow::{
    Context,
    Result,
    bail,
};
use image::RgbaImage;
use libpzmap::texturepack::Texture;
use std::io::Cursor;

/// Largest page side, in pixels, that the viewer will decode. It stops a corrupt or malicious
/// file from causing a huge allocation.
const MAX_PAGE_DIMENSION: u32 = 8192;

/// What transparent pixels are drawn over.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Background {
    /// A gray checkerboard. It reads as empty space and cannot be mistaken for black pixels.
    #[default]
    Checker,
    /// Plain black, to see how a sprite looks over dark ground.
    Black,
    /// Plain magenta. The game art does not use this color, so it makes stray, nearly
    /// transparent pixels easy to see.
    Magenta,
}

impl Background {
    /// The background that follows this one when cycling.
    pub fn next(self) -> Self {
        match self {
            Background::Checker => Background::Black,
            Background::Black => Background::Magenta,
            Background::Magenta => Background::Checker,
        }
    }

    /// The background's display name.
    pub fn label(self) -> &'static str {
        match self {
            Background::Checker => "checker",
            Background::Black => "black",
            Background::Magenta => "magenta",
        }
    }

    /// The background color at an output pixel.
    pub fn sample(self, x: u32, y: u32) -> [u8; 3] {
        match self {
            Background::Checker => {
                // 8-pixel squares are easy to see but do not distract from the art.
                if ((x / 8) + (y / 8)).is_multiple_of(2) {
                    [0x3A, 0x3A, 0x3A]
                } else {
                    [0x24, 0x24, 0x24]
                }
            }
            Background::Black => [0, 0, 0],
            Background::Magenta => [0xFF, 0x00, 0xFF],
        }
    }
}

/// Decodes one page's PNG to 8-bit RGBA, with alpha not premultiplied.
pub fn decode_page(png: &[u8]) -> Result<RgbaImage> {
    let reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    let decoded = reader.decode().context("decoding page PNG")?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 {
        bail!("page PNG has a zero dimension ({width}x{height})");
    }
    if width > MAX_PAGE_DIMENSION || height > MAX_PAGE_DIMENSION {
        bail!("page is {width}x{height}, over the {MAX_PAGE_DIMENSION} px limit");
    }
    Ok(decoded.into_rgba8())
}

/// A texture's rectangle, cut down where needed to fit inside its page.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClampedRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Whether the rectangle in the file did not fit the page and had to be cut down.
    pub clamped: bool,
}

/// Cuts a texture's rectangle down to fit inside its page.
///
/// The parser checks counts but not coordinates, so a modded or corrupt pack can hold a
/// rectangle that sticks out of its page. Such a rectangle is cut down to fit rather than
/// rejected.
///
/// # Errors
///
/// Returns an error if the clamped rect has no overlap with the page.
pub fn clamp_rect(texture: &Texture<'_>, page_w: u32, page_h: u32) -> Result<ClampedRect> {
    if texture.width <= 0 || texture.height <= 0 {
        bail!(
            "texture rect is {}x{}, which is empty",
            texture.width,
            texture.height
        );
    }

    // Work in i64 so that a rectangle near i32::MAX cannot overflow while being clipped.
    let left = i64::from(texture.x);
    let top = i64::from(texture.y);
    let right = left + i64::from(texture.width);
    let bottom = top + i64::from(texture.height);

    let clipped_left = left.max(0);
    let clipped_top = top.max(0);
    let clipped_right = right.min(i64::from(page_w));
    let clipped_bottom = bottom.min(i64::from(page_h));

    if clipped_right <= clipped_left || clipped_bottom <= clipped_top {
        bail!(
            "texture rect {},{} {}x{} lies outside the {page_w}x{page_h} page",
            texture.x,
            texture.y,
            texture.width,
            texture.height
        );
    }

    let clamped = clipped_left != left
        || clipped_top != top
        || clipped_right != right
        || clipped_bottom != bottom;

    Ok(ClampedRect {
        x: u32::try_from(clipped_left).unwrap_or(0),
        y: u32::try_from(clipped_top).unwrap_or(0),
        w: u32::try_from(clipped_right - clipped_left).unwrap_or(0),
        h: u32::try_from(clipped_bottom - clipped_top).unwrap_or(0),
        clamped,
    })
}

/// Returns the largest size within `(max_w, max_h)` that keeps the aspect ratio of `(w, h)`.
///
/// It never scales up. It returns at least 1x1, so a small image never shrinks to nothing.
pub fn fit_within(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 {
        return (0, 0);
    }
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    // Multiply in u64 so that an 8192-pixel page cannot overflow the product.
    let wide = u64::from(w) * u64::from(max_h) >= u64::from(h) * u64::from(max_w);
    if wide {
        let height = (u64::from(h) * u64::from(max_w) / u64::from(w)).max(1);
        (
            max_w,
            u32::try_from(height).unwrap_or(max_h).min(max_h).max(1),
        )
    } else {
        let width = (u64::from(w) * u64::from(max_h) / u64::from(h)).max(1);
        (
            u32::try_from(width).unwrap_or(max_w).min(max_w).max(1),
            max_h,
        )
    }
}

/// Returns the largest whole-number zoom at which `(w, h)` still fits `(max_w, max_h)`.
///
/// A fractional zoom would draw some of the sprite's pixels larger than others.
pub fn fit_zoom(w: u32, h: u32, max_w: u32, max_h: u32) -> u32 {
    if w == 0 || h == 0 {
        return 1;
    }
    (max_w / w).min(max_h / h).max(1)
}

/// Which part of the page the preview shows, and at what size.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The part of the page to show.
    pub rect: ClampedRect,
    /// Width of the preview, in output pixels.
    pub width: u32,
    /// Height of the preview, in output pixels.
    pub height: u32,
    /// Whether to scale with nearest-neighbor, which keeps magnified pixel art crisp.
    pub nearest: bool,
}

/// Works out what to show of a page, and at what size, in a pane of the given size in pixels.
///
/// The page view shrinks the whole page to fit. The texture view magnifies the texture by a
/// whole-number factor. A zoom too large for the pane is shrunk to fit instead of being cut off.
///
/// # Errors
///
/// In the texture view, returns an error when there is no texture or its rectangle lies
/// entirely outside the page.
pub fn place(
    view: View,
    zoom: Zoom,
    texture: Option<&Texture<'_>>,
    (page_w, page_h): (u32, u32),
    (max_w, max_h): (u32, u32),
) -> Result<Placement> {
    match view {
        View::Page => {
            let (width, height) = fit_within(page_w, page_h, max_w, max_h);
            Ok(Placement {
                rect: ClampedRect {
                    x: 0,
                    y: 0,
                    w: page_w,
                    h: page_h,
                    clamped: false,
                },
                width,
                height,
                nearest: false,
            })
        }
        View::Texture => {
            let texture = texture.context("this page has no textures")?;
            let rect = clamp_rect(texture, page_w, page_h)?;
            let factor = match zoom {
                Zoom::Fit => fit_zoom(rect.w, rect.h, max_w, max_h),
                Zoom::Fixed(factor) => factor,
            };
            let scaled_w = rect.w.saturating_mul(factor);
            let scaled_h = rect.h.saturating_mul(factor);
            let (width, height) = fit_within(scaled_w, scaled_h, max_w, max_h);
            Ok(Placement {
                rect,
                width,
                height,
                nearest: true,
            })
        }
    }
}

/// Color of the selected sprite's outline on the page view. Yellow stands out on all three
/// backgrounds.
pub const OUTLINE: [u8; 3] = [0xFF, 0xD8, 0x00];

/// Color of the thin edge around the outline, which keeps it visible over pale art.
pub const OUTLINE_EDGE: [u8; 3] = [0x00, 0x00, 0x00];

/// Smallest side of the outlined box in output pixels, so a tiny sprite stays easy to find.
pub const OUTLINE_MIN_SIDE: f32 = 6.0;

/// Returns the box to outline around `rect`, as `[left, top, right, bottom]`.
///
/// `scale` is how many output pixels each page pixel covers. The box is measured in whole
/// output pixels from the page's top-left corner. A box smaller than [`OUTLINE_MIN_SIDE`] grows
/// around its center.
pub fn outline_box(rect: ClampedRect, (scale_x, scale_y): (f32, f32)) -> [f32; 4] {
    let edge = |value: u32, scale: f32| (value as f32 * scale).round();
    let (left, right) = grow(
        edge(rect.x, scale_x),
        edge(rect.x.saturating_add(rect.w), scale_x),
    );
    let (top, bottom) = grow(
        edge(rect.y, scale_y),
        edge(rect.y.saturating_add(rect.h), scale_y),
    );
    [left, top, right, bottom]
}

fn grow(low: f32, high: f32) -> (f32, f32) {
    let short = OUTLINE_MIN_SIDE - (high - low);
    if short <= 0.0 {
        return (low, high);
    }
    let before = (short / 2.0).floor();
    (low - before, high + (short - before))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn texture(x: i32, y: i32, w: i32, h: i32) -> Texture<'static> {
        Texture::new("t", x, y, w, h)
    }

    #[test]
    fn a_rect_inside_the_page_is_untouched() {
        let rect = clamp_rect(&texture(10, 20, 30, 40), 2048, 2048).unwrap();
        assert_eq!(
            rect,
            ClampedRect {
                x: 10,
                y: 20,
                w: 30,
                h: 40,
                clamped: false
            }
        );
    }

    #[test]
    fn a_rect_overrunning_the_page_is_cut_down() {
        let rect = clamp_rect(&texture(100, 100, 100, 100), 150, 120).unwrap();
        assert_eq!(
            rect,
            ClampedRect {
                x: 100,
                y: 100,
                w: 50,
                h: 20,
                clamped: true
            }
        );
    }

    #[test]
    fn a_rect_starting_before_the_origin_is_cut_down() {
        let rect = clamp_rect(&texture(-10, -5, 40, 40), 100, 100).unwrap();
        assert_eq!(
            rect,
            ClampedRect {
                x: 0,
                y: 0,
                w: 30,
                h: 35,
                clamped: true
            }
        );
    }

    #[test]
    fn a_rect_with_no_overlap_is_an_error() {
        assert!(clamp_rect(&texture(500, 500, 10, 10), 100, 100).is_err());
        assert!(clamp_rect(&texture(0, 0, 0, 10), 100, 100).is_err());
        assert!(clamp_rect(&texture(0, 0, 10, -1), 100, 100).is_err());
    }

    /// `x + w` can overflow `i32`, so clipping must happen in `i64`.
    #[test]
    fn an_extreme_rect_does_not_overflow() {
        // Starts past the right edge, so nothing to show.
        let rect = clamp_rect(&texture(i32::MAX - 5, 0, i32::MAX, 10), 100, 100);
        assert!(rect.is_err());

        // Starts far to the left and is wide enough to cover the page, so it clips to all of it.
        let rect = clamp_rect(&texture(-1000, -1000, i32::MAX, i32::MAX), 100, 100).unwrap();
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (0, 0, 100, 100));
        assert!(rect.clamped);

        // A rectangle starting at `i32::MIN` cannot reach the page. Its right edge is at most
        // `i32::MIN + i32::MAX`, which is -1, so it is off the page.
        let rect = clamp_rect(&texture(i32::MIN, i32::MIN, i32::MAX, i32::MAX), 100, 100);
        assert!(rect.is_err());
    }

    #[test]
    fn fitting_preserves_aspect_and_never_upscales() {
        assert_eq!(fit_within(2048, 2048, 800, 400), (400, 400));
        assert_eq!(fit_within(2048, 1024, 800, 800), (800, 400));
        assert_eq!(fit_within(1024, 2048, 800, 800), (400, 800));
        assert_eq!(fit_within(64, 128, 800, 800), (64, 128));
        assert_eq!(fit_within(0, 10, 800, 800), (0, 0));
    }

    #[test]
    fn fitting_a_sliver_keeps_one_pixel() {
        let (w, h) = fit_within(4000, 3, 100, 100);
        assert_eq!(w, 100);
        assert_eq!(h, 1);
    }

    #[test]
    fn zoom_is_the_largest_whole_factor_that_fits() {
        assert_eq!(fit_zoom(64, 128, 800, 600), 4);
        assert_eq!(fit_zoom(64, 128, 100, 100), 1);
        assert_eq!(fit_zoom(10, 10, 105, 55), 5);
    }

    #[test]
    fn the_checkerboard_alternates_on_an_eight_pixel_grid() {
        assert_eq!(
            Background::Checker.sample(0, 0),
            Background::Checker.sample(7, 7)
        );
        assert_ne!(
            Background::Checker.sample(0, 0),
            Background::Checker.sample(8, 0)
        );
        assert_eq!(
            Background::Checker.sample(8, 0),
            Background::Checker.sample(0, 8)
        );
    }
}
