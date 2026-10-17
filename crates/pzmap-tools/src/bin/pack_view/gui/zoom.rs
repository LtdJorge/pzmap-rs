//! Zoom of the page view in the window.

use crate::render;

/// Largest factor that fit mode magnifies a small page by. It keeps a tiny page from turning
/// into a few giant blocks.
pub const MAX_FIT: u32 = 8;

/// The fixed scales the page view steps through, in physical pixels per page pixel.
const SCALES: [f32; 11] = [0.125, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];

/// How the page view is scaled.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PageZoom {
    /// As large as fits the pane, magnifying only by whole factors.
    Fit,
    /// One of the fixed scales, by index.
    Step(usize),
}

impl PageZoom {
    /// The fixed zoom closest to `factor`.
    pub fn fixed(factor: u32) -> Self {
        let wanted = factor as f32;
        let index = SCALES
            .iter()
            .position(|&scale| scale >= wanted)
            .unwrap_or(SCALES.len() - 1);
        PageZoom::Step(index)
    }

    /// Physical pixels per page pixel for a page of `page` pixels shown in `pane` pixels.
    pub fn scale(self, page: (u32, u32), pane: (u32, u32)) -> f32 {
        match self {
            PageZoom::Fit => fit_scale(page, pane),
            PageZoom::Step(index) => SCALES.get(index).copied().unwrap_or(1.0),
        }
    }

    /// The next fixed scale above or below `current`, the scale now on screen.
    pub fn step(current: f32, delta: i32) -> Self {
        const SLACK: f32 = 1e-3;
        let index = if delta > 0 {
            SCALES
                .iter()
                .position(|&scale| scale > current + SLACK)
                .unwrap_or(SCALES.len() - 1)
        } else {
            SCALES
                .iter()
                .rposition(|&scale| scale < current - SLACK)
                .unwrap_or(0)
        };
        PageZoom::Step(index)
    }
}

/// Returns the scale for fit mode.
///
/// A page larger than the pane shrinks to fit it. A smaller page grows by the largest whole
/// factor that fits, up to [`MAX_FIT`], so all its pixels stay the same size.
pub fn fit_scale((page_w, page_h): (u32, u32), (pane_w, pane_h): (u32, u32)) -> f32 {
    if page_w == 0 || page_h == 0 || pane_w == 0 || pane_h == 0 {
        return 1.0;
    }
    if page_w <= pane_w && page_h <= pane_h {
        render::fit_zoom(page_w, page_h, pane_w, pane_h).min(MAX_FIT) as f32
    } else {
        (pane_w as f32 / page_w as f32).min(pane_h as f32 / page_h as f32)
    }
}

/// A scale for the toolbar: `3x` when magnified, `45%` when shrunk.
pub fn describe(scale: f32) -> String {
    if scale >= 1.0 {
        format!("{}x", (scale * 100.0).round() / 100.0)
    } else {
        format!("{}%", (scale * 100.0).round())
    }
}
