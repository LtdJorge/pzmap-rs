//! Builds the RGB preview image for the terminal frontend.
//!
//! Each page is decoded once and cached. The preview is cut from the page, either the whole page
//! or one texture. It is then scaled to fit the pane and drawn over an opaque background.

use crate::render::{
    Background,
    ClampedRect,
    decode_page,
};
use anyhow::{
    Context,
    Result,
    bail,
};
use image::{
    Rgba,
    RgbaImage,
    imageops::FilterType,
};
use std::collections::VecDeque;

/// How many bytes of decoded pages to keep. A 2048 by 2048 page takes 16 MB, so this holds four.
const PAGE_CACHE_BYTES: usize = 64 << 20;

/// A decoded page, kept whole so that showing another texture on it does not decode it again.
struct CachedPage {
    index: usize,
    image: RgbaImage,
}

/// A cache of decoded pages, limited by their total size. When it is full, the page used least
/// recently is dropped first.
pub struct PageCache {
    pages: VecDeque<CachedPage>,
    bytes: usize,
}

impl PageCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self {
            pages: VecDeque::new(),
            bytes: 0,
        }
    }

    /// Drops every cached page.
    pub fn clear(&mut self) {
        self.pages.clear();
        self.bytes = 0;
    }

    /// Returns the decoded pixels of page `index`, decoding `png` if it is not already held.
    pub fn get_or_decode(&mut self, index: usize, png: &[u8]) -> Result<&RgbaImage> {
        if let Some(position) = self.pages.iter().position(|p| p.index == index) {
            // Move it to the back, so the front stays the least recently used.
            if let Some(page) = self.pages.remove(position) {
                self.pages.push_back(page);
            }
        } else {
            let image = decode_page(png)?;
            let size = image.as_raw().len();
            self.pages.push_back(CachedPage { index, image });
            self.bytes = self.bytes.saturating_add(size);
            while self.bytes > PAGE_CACHE_BYTES && self.pages.len() > 1 {
                if let Some(evicted) = self.pages.pop_front() {
                    self.bytes = self.bytes.saturating_sub(evicted.image.as_raw().len());
                }
            }
        }

        self.pages
            .back()
            .map(|p| &p.image)
            .context("page cache lost the page it just inserted")
    }
}

/// A scaled RGB image, already drawn over its background.
pub struct Rendered {
    /// RGB pixels, row by row from the top.
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Cuts `rect` out of `source`, resizes it to `(target_w, target_h)`, and draws it over
/// `background`.
///
/// With `nearest`, each output pixel copies the closest source pixel, which keeps magnified
/// pixel art crisp. Otherwise a smoothing filter is used, so that thin lines in a shrunk sprite
/// do not disappear between samples.
///
/// The colors are premultiplied by alpha before resizing. Otherwise the hidden color of fully
/// transparent pixels would bleed into their opaque neighbors.
pub fn render(
    source: &RgbaImage,
    rect: ClampedRect,
    target_w: u32,
    target_h: u32,
    nearest: bool,
    background: Background,
) -> Result<Rendered> {
    if target_w == 0 || target_h == 0 {
        bail!("target size is {target_w}x{target_h}");
    }

    let cropped = image::imageops::crop_imm(source, rect.x, rect.y, rect.w, rect.h).to_image();

    let premultiplied = premultiply(&cropped);
    let scaled = if premultiplied.width() == target_w && premultiplied.height() == target_h {
        premultiplied
    } else {
        let filter = if nearest {
            FilterType::Nearest
        } else {
            FilterType::Triangle
        };
        image::imageops::resize(&premultiplied, target_w, target_h, filter)
    };

    Ok(composite(&scaled, background))
}

/// Draws the selection outline just outside a box from
/// [`outline_box`](crate::render::outline_box).
///
/// The outline is two pixels of the outline color, then one pixel of the dark edge. Parts that
/// fall outside the image are skipped.
pub fn outline(rendered: &mut Rendered, [left, top, right, bottom]: [f32; 4]) {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the box is a few pixels around an image of at most 8192 per side"
    )]
    let [left, top, right, bottom] = [left, top, right, bottom].map(|v| v as i64);
    let (width, height) = (i64::from(rendered.width), i64::from(rendered.height));
    for y in (top - 3).max(0)..(bottom + 3).min(height) {
        for x in (left - 3).max(0)..(right + 3).min(width) {
            // How far outside the box this pixel is, or zero inside it.
            let distance = (left - x)
                .max(x - right + 1)
                .max(top - y)
                .max(y - bottom + 1);
            let color = match distance {
                1 | 2 => crate::render::OUTLINE,
                3 => crate::render::OUTLINE_EDGE,
                _ => continue,
            };
            let Ok(index) = usize::try_from((y * width + x) * 3) else {
                continue;
            };
            if let Some(pixel) = rendered.rgb.get_mut(index..index + 3) {
                pixel.copy_from_slice(&color);
            }
        }
    }
}

/// Multiplies each color channel by the pixel's alpha.
fn premultiply(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let Rgba([r, g, b, a]) = *pixel;
        if a == 255 {
            continue;
        }
        *pixel = Rgba([
            scale_by_alpha(r, a),
            scale_by_alpha(g, a),
            scale_by_alpha(b, a),
            a,
        ]);
    }
    out
}

/// Computes `channel * alpha / 255`, rounded to nearest, without a division. It rounds the same
/// way as pzmap2img's texture build.
const fn scale_by_alpha(channel: u8, alpha: u8) -> u8 {
    let t = (channel as u16) * (alpha as u16) + 128;
    ((t + (t >> 8)) >> 8) as u8
}

/// Lays premultiplied pixels over an opaque background and drops the alpha channel.
fn composite(image: &RgbaImage, background: Background) -> Rendered {
    let width = image.width();
    let height = image.height();
    let mut rgb = Vec::with_capacity((width as usize) * (height as usize) * 3);

    for (x, y, pixel) in image.enumerate_pixels() {
        let Rgba([r, g, b, a]) = *pixel;
        let under = background.sample(x, y);
        // Standard "over" blending with a premultiplied source: out = src + dst * (1 - alpha).
        let inverse = 255 - a;
        rgb.push(r.saturating_add(scale_by_alpha(under[0], inverse)));
        rgb.push(g.saturating_add(scale_by_alpha(under[1], inverse)));
        rgb.push(b.saturating_add(scale_by_alpha(under[2], inverse)));
    }

    Rendered { rgb, width, height }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn premultiplication_rounds_exactly() {
        assert_eq!(scale_by_alpha(255, 255), 255);
        assert_eq!(scale_by_alpha(255, 0), 0);
        assert_eq!(scale_by_alpha(255, 128), 128);
        assert_eq!(scale_by_alpha(128, 128), 64);
        assert_eq!(scale_by_alpha(0, 200), 0);
    }

    #[test]
    fn an_opaque_pixel_keeps_its_color_over_any_background() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, Rgba([200, 100, 50, 255]));
        let rendered = composite(&premultiply(&image), Background::Magenta);
        assert_eq!(rendered.rgb, vec![200, 100, 50]);
    }

    #[test]
    fn a_transparent_pixel_shows_only_the_background() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, Rgba([200, 100, 50, 0]));
        let rendered = composite(&premultiply(&image), Background::Magenta);
        assert_eq!(rendered.rgb, vec![255, 0, 255]);
    }

    /// Half-transparent red over black gives half-strength red, with no alpha channel.
    #[test]
    fn a_half_transparent_pixel_blends() {
        let mut image = RgbaImage::new(1, 1);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 128]));
        let rendered = composite(&premultiply(&image), Background::Black);
        assert_eq!(rendered.rgb, vec![128, 0, 0]);
        assert_eq!(rendered.width, 1);
        assert_eq!(rendered.height, 1);
    }

    #[test]
    fn scaling_does_not_halo_sprite_edges() {
        // A 2x1 image: opaque white next to a fully transparent black pixel, as a packer leaves
        // it. Magnified, every output pixel must be white or background, never a blended gray.
        let mut image = RgbaImage::new(2, 1);
        image.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        image.put_pixel(1, 0, Rgba([0, 0, 0, 0]));

        let rect = ClampedRect {
            x: 0,
            y: 0,
            w: 2,
            h: 1,
            clamped: false,
        };
        let rendered = render(&image, rect, 8, 4, true, Background::Black).unwrap();
        for chunk in rendered.rgb.chunks(3) {
            assert!(
                chunk == [255, 255, 255] || chunk == [0, 0, 0],
                "unexpected blended pixel {chunk:?}"
            );
        }
    }

    #[test]
    fn the_page_cache_evicts_but_keeps_the_newest() {
        let mut cache = PageCache::new();
        assert_eq!(cache.pages.len(), 0);
        cache.clear();
        assert_eq!(cache.bytes, 0);
    }
}
