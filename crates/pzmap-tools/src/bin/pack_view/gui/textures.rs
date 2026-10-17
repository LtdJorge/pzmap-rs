//! GPU copies of atlas pages, uploaded once per page and reused across frames.

use crate::{
    model::Viewer,
    render::{
        self,
        Background,
    },
};
use eframe::egui::{
    self,
    ColorImage,
    TextureFilter,
    TextureHandle,
    TextureOptions,
    TextureWrapMode,
};
use std::collections::VecDeque;

/// How many uploaded pages to keep. A 2048 by 2048 page takes 16 MB of video memory.
const KEEP_PAGES: usize = 4;

/// Side of the checkerboard texture in pixels: two of the 8-pixel squares.
pub const CHECKER_SIZE: u32 = 16;

/// Pages are drawn with crisp pixels when magnified, and smoothed when shrunk.
const PAGE_OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(TextureFilter::Linear),
};

/// One page's upload, or why it failed. Failures are kept too, so a broken page is not decoded
/// again on every frame.
struct Entry {
    page: usize,
    texture: Result<TextureHandle, String>,
}

/// The open pack's pages uploaded to the GPU. When it is full, the page used least recently is
/// dropped first.
#[derive(Default)]
pub struct PageTextures {
    generation: u64,
    entries: VecDeque<Entry>,
    checker: Option<TextureHandle>,
}

impl PageTextures {
    /// Drops every uploaded page.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the selected page's texture, decoding and uploading it on first use.
    pub fn get(&mut self, ctx: &egui::Context, viewer: &Viewer) -> Result<TextureHandle, String> {
        if self.generation != viewer.generation {
            // A different pack is open, so every upload belongs to the wrong file.
            self.entries.clear();
            self.generation = viewer.generation;
        }

        if let Some(position) = self.entries.iter().position(|e| e.page == viewer.page) {
            // Move it to the back, so the front stays the least recently used.
            if let Some(entry) = self.entries.remove(position) {
                let texture = entry.texture.clone();
                self.entries.push_back(entry);
                return texture;
            }
        }

        let texture = upload(ctx, viewer);
        self.entries.push_back(Entry {
            page: viewer.page,
            texture: texture.clone(),
        });
        while self.entries.len() > KEEP_PAGES {
            self.entries.pop_front();
        }
        texture
    }

    /// The pixel size of `page`, if it has been uploaded.
    pub fn size_of(&self, page: usize) -> Option<(usize, usize)> {
        self.entries
            .iter()
            .find(|e| e.page == page)
            .and_then(|e| e.texture.as_ref().ok())
            .map(|texture| {
                let [w, h] = texture.size();
                (w, h)
            })
    }

    /// A small checkerboard that tiles, drawn under transparent pixels.
    pub fn checker(&mut self, ctx: &egui::Context) -> TextureHandle {
        self.checker
            .get_or_insert_with(|| {
                let rgb: Vec<u8> = (0..CHECKER_SIZE)
                    .flat_map(|y| {
                        (0..CHECKER_SIZE).flat_map(move |x| Background::Checker.sample(x, y))
                    })
                    .collect();
                let side = CHECKER_SIZE as usize;
                ctx.load_texture(
                    "checker",
                    ColorImage::from_rgb([side, side], &rgb),
                    TextureOptions {
                        magnification: TextureFilter::Nearest,
                        minification: TextureFilter::Nearest,
                        wrap_mode: TextureWrapMode::Repeat,
                        mipmap_mode: None,
                    },
                )
            })
            .clone()
    }
}

/// Decodes the selected page and hands it to the GPU.
fn upload(ctx: &egui::Context, viewer: &Viewer) -> Result<TextureHandle, String> {
    let page = viewer
        .current_page()
        .ok_or_else(|| format!("pack has no page {}", viewer.page))?;
    let image = render::decode_page(page.png).map_err(|error| format!("{error:#}"))?;

    let width = usize::try_from(image.width()).map_err(|error| error.to_string())?;
    let height = usize::try_from(image.height()).map_err(|error| error.to_string())?;
    let max_side = ctx.input(|input| input.max_texture_side);
    if width > max_side || height > max_side {
        return Err(format!(
            "page is {width}x{height}, larger than this graphics driver's {max_side} px limit"
        ));
    }

    let pixels = ColorImage::from_rgba_unmultiplied([width, height], image.as_raw());
    Ok(ctx.load_texture(format!("page {}", viewer.page), pixels, PAGE_OPTIONS))
}
