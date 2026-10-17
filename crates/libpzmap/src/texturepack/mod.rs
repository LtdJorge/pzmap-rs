//! Texture packs (`.pack` files), which hold the images for tiles.
//!
//! A pack contains a few large PNG images, called pages. Each page comes with a table that gives
//! the name and rectangle of every sprite on it. To draw a tile, find the texture with the tile's
//! name and copy its rectangle out of the page image.
//!
//! This crate does not decode the PNGs. [`Page::png`] is the compressed image as stored in the
//! file.
//!
//! Parse a pack with [`TexturePack::parse`], or with [`OwnedTexturePack::parse`] to keep the
//! buffer with it.

mod parse;

use crate::error::Result;
use yoke::{
    Yoke,
    Yokeable,
};

/// One sprite on a page.
///
/// The pack stores sprites with their transparent margins cut off. `x`, `y`, `width` and `height`
/// give the trimmed rectangle within the page image. To draw the sprite at its original size,
/// place that rectangle at (`offset_x`, `offset_y`) inside an empty box of `original_width` by
/// `original_height`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Texture<'inp> {
    /// The texture's name, which is the same as the name of its tile.
    pub name: &'inp str,
    /// X of the trimmed sprite inside the spritesheet
    pub x: i32,
    /// Y of the trimmed sprite inside the spritesheet
    pub y: i32,
    /// Width of the trimmed sprite
    pub width: i32,
    /// Height of the trimmed sprite
    pub height: i32,
    /// X of the trimmed rectangle within the original sprite.
    pub offset_x: i32,
    /// Y of the trimmed rectangle within the original sprite.
    pub offset_y: i32,
    /// Width of the original sprite before trimming.
    pub original_width: i32,
    /// Height of the original sprite before trimming.
    pub original_height: i32,
}

/// One page of a pack: a PNG spritesheet containing many sprites.
#[derive(Clone, Debug, PartialEq, Eq, Yokeable)]
#[non_exhaustive]
pub struct Page<'inp> {
    /// The page's name.
    pub name: &'inp str,
    /// Whether the pack says the page's PNG has an alpha channel.
    pub has_alpha: bool,
    /// Every sprite on this page.
    pub textures: Vec<Texture<'inp>>,
    /// The page image as PNG bytes, borrowed from the pack buffer.
    pub png: &'inp [u8],
}

/// A parsed `.pack` file.
///
/// Names and page images borrow the input buffer. To keep the pack and its buffer together, use
/// [`OwnedTexturePack`].
#[derive(Clone, Debug, PartialEq, Eq, Yokeable)]
#[non_exhaustive]
pub struct TexturePack<'inp> {
    /// Every page in the pack, in file order.
    pub pages: Vec<Page<'inp>>,
}

impl<'inp> Texture<'inp> {
    /// A texture with no trimmed margins, so its original size is its rectangle's size.
    ///
    /// To describe a trimmed sprite, set the offset and original size fields afterwards.
    pub fn new(name: &'inp str, x: i32, y: i32, width: i32, height: i32) -> Self {
        Texture {
            name,
            x,
            y,
            width,
            height,
            offset_x: 0,
            offset_y: 0,
            original_width: width,
            original_height: height,
        }
    }
}

impl TexturePack<'_> {
    /// The number of textures on all pages together.
    pub fn texture_count(&self) -> usize {
        self.pages.iter().map(|page| page.textures.len()).sum()
    }
}

/// A parsed `.pack` that owns its buffer.
///
/// Page images still point into the buffer, so no image data is copied.
///
/// ```no_run
/// use libpzmap::texturepack::OwnedTexturePack;
///
/// let pack = OwnedTexturePack::parse(std::fs::read("Tiles2x.pack")?)?;
/// println!("{} textures", pack.get().texture_count());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct OwnedTexturePack(Yoke<TexturePack<'static>, Vec<u8>>);

impl OwnedTexturePack {
    /// Parses `bytes` as a `.pack` file keeping `bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error under the same conditions as [`TexturePack::parse`].
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        Yoke::try_attach_to_cart(bytes, |bytes| TexturePack::parse(bytes)).map(OwnedTexturePack)
    }

    /// The parsed pack.
    pub fn get(&self) -> &TexturePack<'_> {
        self.0.get()
    }

    /// The buffer the pack was parsed from.
    pub fn bytes(&self) -> &[u8] {
        self.0.backing_cart()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn owned_packs_borrow_their_pages_from_the_buffer() {
        let bytes = fixtures::read(&fixtures::texture_pack("Tiles2x.pack"));
        let borrowed = TexturePack::parse(&bytes).expect("pack should parse");
        let owned = OwnedTexturePack::parse(bytes.clone()).expect("pack should parse");

        assert_eq!(owned.get(), &borrowed);
        let buffer = owned.bytes().as_ptr_range();
        for page in &owned.get().pages {
            assert!(
                buffer.contains(&page.png.as_ptr()),
                "page {} was copied",
                page.name
            );
        }
    }
}
