//! What the viewer is showing. Both the terminal and the window frontend share this state.

use crate::{
    pack::{
        Pack,
        Target,
        TextureRef,
    },
    render::Background,
};
use anyhow::{
    Context,
    Result,
};
use libpzmap::texturepack::{
    Page,
    Texture,
};
use std::path::{
    Path,
    PathBuf,
};

/// What fills the image pane.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// The whole atlas page, scaled down to fit.
    Page,
    /// One texture, magnified.
    Texture,
}

/// Zoom for the texture view.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Zoom {
    /// The largest whole-number factor that fits the pane.
    Fit,
    /// A fixed whole-number factor.
    Fixed(u32),
}

/// Largest fixed zoom factor.
pub const MAX_ZOOM: u32 = 32;

/// The open pack and the selection within it.
pub struct Viewer {
    /// The open pack.
    pub pack: Pack,
    /// Every pack in the open pack's folder. Empty when a single file was opened.
    pub siblings: Vec<PathBuf>,
    /// Index of the open pack in `siblings`.
    pub current_sibling: usize,
    /// Goes up by one whenever `pack` is replaced. The frontends' page caches compare it to tell
    /// when their pages belong to an older pack.
    pub generation: u64,

    /// Index of the selected page.
    pub page: usize,
    /// Index of the selected texture on that page.
    pub texture: usize,
    /// Whether the image pane shows the page or the texture.
    pub view: View,
    /// Zoom of the texture view.
    pub zoom: Zoom,
    /// What transparent pixels are drawn over.
    pub background: Background,

    /// Results of the last search.
    pub matches: Vec<TextureRef>,
    /// Where `n` and `N` are in `matches`.
    pub match_cursor: usize,
    /// The text of the last search.
    pub last_query: String,

    /// Status message, cleared by the next successful action.
    pub status: Option<String>,
}

impl Viewer {
    /// Shows `pack`, starting on its first page and texture. `siblings` are the packs it can
    /// switch to.
    pub fn new(pack: Pack, siblings: Vec<PathBuf>) -> Self {
        let current_sibling = siblings.iter().position(|p| *p == pack.path).unwrap_or(0);
        Self {
            pack,
            siblings,
            current_sibling,
            generation: 0,
            page: 0,
            texture: 0,
            view: View::Page,
            zoom: Zoom::Fit,
            background: Background::default(),
            matches: Vec::new(),
            match_cursor: 0,
            last_query: String::new(),
            status: None,
        }
    }

    /// Opens `path`, which is a `.pack` file or a folder of them.
    ///
    /// For a folder, `name` picks the pack to show first. Without it, the first pack is shown.
    pub fn open(path: &Path, name: Option<&str>) -> Result<Self> {
        let (first, siblings) = match crate::pack::resolve(path)? {
            Target::File(file) => (file, Vec::new()),
            Target::Directory(packs) => {
                let chosen = match name {
                    Some(name) => packs
                        .iter()
                        .find(|p| p.file_name().is_some_and(|f| f.eq_ignore_ascii_case(name)))
                        .cloned()
                        .with_context(|| format!("no pack named {name} in {}", path.display()))?,
                    None => packs.first().cloned().context("directory has no packs")?,
                };
                (chosen, packs)
            }
        };
        Ok(Self::new(Pack::open(&first)?, siblings))
    }

    /// Returns the selected page, if the pack has it.
    pub fn current_page(&self) -> Option<&Page<'static>> {
        self.pack.page(self.page)
    }

    /// Returns how many textures the selected page holds.
    pub fn texture_count(&self) -> usize {
        self.current_page().map_or(0, |p| p.textures.len())
    }

    /// Returns the selected texture, if the page has any.
    pub fn selected_texture(&self) -> Option<TextureRef> {
        (self.texture < self.texture_count()).then_some(TextureRef {
            page: self.page,
            texture: self.texture,
        })
    }

    /// Returns the selected texture's record, if the page has any.
    pub fn selected_texture_data(&self) -> Option<&Texture<'static>> {
        self.selected_texture().and_then(|at| self.pack.texture(at))
    }

    /// Selects page `index` and its first texture. Returns whether anything changed.
    pub fn select_page(&mut self, index: usize) -> bool {
        if self.page == index || index >= self.pack.page_count() {
            return false;
        }
        self.page = index;
        self.texture = 0;
        true
    }

    /// Selects texture `index` on the current page. Returns whether anything changed.
    pub fn select_texture(&mut self, index: usize) -> bool {
        if self.texture == index || index >= self.texture_count() {
            return false;
        }
        self.texture = index;
        true
    }

    /// Selects a texture anywhere in the pack and shows it.
    pub fn select(&mut self, at: TextureRef) {
        if self.pack.texture(at).is_none() {
            return;
        }
        self.page = at.page;
        self.texture = at.texture;
        self.view = View::Texture;
    }

    /// Switches to the next background.
    pub fn cycle_background(&mut self) {
        self.background = self.background.next();
    }

    /// Switches between the page view and the texture view.
    pub fn toggle_view(&mut self) {
        self.view = match self.view {
            View::Page => View::Texture,
            View::Texture => View::Page,
        };
    }

    /// Steps the fixed zoom by `delta`, switching to the texture view.
    pub fn zoom_by(&mut self, delta: i32) {
        // Stepping off `Fit` starts from 1, since the resolved fit factor is not known here.
        let current = match self.zoom {
            Zoom::Fit => 1,
            Zoom::Fixed(factor) => factor,
        };
        let next = i64::from(current)
            .saturating_add(i64::from(delta))
            .clamp(1, i64::from(MAX_ZOOM));
        self.zoom = Zoom::Fixed(u32::try_from(next).unwrap_or(1));
        self.view = View::Texture;
    }

    /// Searches the pack and jumps to the first match. Returns whether it moved.
    pub fn run_search(&mut self, query: &str) -> bool {
        self.last_query = query.to_owned();
        self.matches = self.pack.search(query);
        self.match_cursor = 0;
        if self.matches.is_empty() {
            self.status = Some(format!("no texture matches \"{query}\""));
            false
        } else {
            let count = self.matches.len();
            self.status = Some(format!("{count} match{} for \"{query}\"", plural(count)));
            self.go_to_match(0)
        }
    }

    /// Steps through the last search's results, wrapping. Returns whether it moved.
    pub fn step_match(&mut self, delta: isize) -> bool {
        if self.matches.is_empty() {
            return false;
        }
        // Wrap, so `n` cycles rather than sticking at the end.
        let next = wrap_index(self.match_cursor, delta, self.matches.len());
        self.go_to_match(next)
    }

    fn go_to_match(&mut self, cursor: usize) -> bool {
        let Some(found) = self.matches.get(cursor).copied() else {
            return false;
        };
        self.match_cursor = cursor;
        self.select(found);
        if let Some(texture) = self.pack.texture(found) {
            let position = cursor + 1;
            let total = self.matches.len();
            self.status = Some(format!("{} ({position}/{total})", texture.name));
        }
        true
    }

    /// Opens another pack from the same folder, starting on its first page and texture. Returns
    /// whether a new pack was opened.
    pub fn switch_pack(&mut self, index: usize) -> bool {
        let Some(path) = self.siblings.get(index).cloned() else {
            return false;
        };
        if path == self.pack.path {
            return false;
        }
        match Pack::open(&path) {
            Ok(pack) => {
                self.pack = pack;
                self.current_sibling = index;
                self.generation = self.generation.wrapping_add(1);
                self.page = 0;
                self.texture = 0;
                self.view = View::Page;
                self.matches.clear();
                self.match_cursor = 0;
                self.status = None;
                true
            }
            Err(error) => {
                self.status = Some(format!("cannot open {}: {error}", path.display()));
                false
            }
        }
    }

    /// Steps to the previous or next pack in the folder, wrapping around at the ends. Returns
    /// whether a new pack was opened.
    pub fn cycle_pack(&mut self, delta: isize) -> bool {
        if self.siblings.len() < 2 {
            return false;
        }
        let next = wrap_index(self.current_sibling, delta, self.siblings.len());
        self.switch_pack(next)
    }
}

/// Moves `current` by `delta` within `0..len`, wrapping around at both ends. Returns 0 for an
/// empty list.
pub fn wrap_index(current: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let step = delta.unsigned_abs() % len;
    if delta >= 0 {
        (current + step) % len
    } else {
        (current + len - step) % len
    }
}

/// The `es` in "3 matches", or nothing for one match.
fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "es" }
}

/// The file name of a pack path.
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Describes a texture's rectangle on its page, as `x,y WxH`.
///
/// When the packer trimmed transparent margins off the sprite, the second string gives the
/// sprite's original size and where the trimmed rectangle sits inside it.
pub fn describe_rect(texture: &Texture<'_>) -> (String, Option<String>) {
    let rect = format!(
        "{},{} {}x{}",
        texture.x, texture.y, texture.width, texture.height
    );
    let untrimmed = (texture.original_width != texture.width
        || texture.original_height != texture.height)
        .then(|| {
            format!(
                "in {}x{} at +{}+{}",
                texture.original_width, texture.original_height, texture.offset_x, texture.offset_y
            )
        });
    (rect, untrimmed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_moves_within_the_list() {
        assert_eq!(wrap_index(0, 1, 5), 1);
        assert_eq!(wrap_index(3, 1, 5), 4);
        assert_eq!(wrap_index(2, -1, 5), 1);
    }

    #[test]
    fn wrapping_cycles_at_both_ends() {
        assert_eq!(wrap_index(4, 1, 5), 0);
        assert_eq!(wrap_index(0, -1, 5), 4);
        assert_eq!(wrap_index(0, -1, 1), 0);
    }

    #[test]
    fn wrapping_survives_an_empty_list_and_a_huge_step() {
        assert_eq!(wrap_index(0, 1, 0), 0);
        assert_eq!(wrap_index(0, -1, 0), 0);
        // Negating `isize::MIN` would overflow. Its magnitude leaves 3 when divided by 5, so
        // stepping back that far from 2 lands on 4.
        assert_eq!(wrap_index(2, isize::MAX, 5), wrap_index(2, 2, 5));
        assert_eq!(wrap_index(2, isize::MIN, 5), 4);
    }

    #[test]
    fn plural_agrees_with_the_count() {
        assert_eq!(plural(1), "");
        assert_eq!(plural(0), "es");
        assert_eq!(plural(7), "es");
    }
}
