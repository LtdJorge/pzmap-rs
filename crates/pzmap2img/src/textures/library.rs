use crate::string_table::TileId;

/// Set when every pixel of the sprite is fully opaque, so drawing it is a plain copy.
pub const FLAG_OPAQUE: u16 = 1 << 0;
/// Set when no sprite was found for the tile, or the sprite is fully transparent.
pub const FLAG_EMPTY: u16 = 1 << 1;

/// Where one sprite's pixels are in a [`TextureLibrary`], and where to draw it.
///
/// It is 16 bytes, so four fit in a cache line. The pixel data's length isn't stored, because
/// it is always `width * height * 4`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct TexMeta {
    /// Where the sprite's pixels start in the library's pixel data, in bytes.
    pub offset: u32,
    /// Width, in pixels.
    pub width: u16,
    /// Height, in pixels.
    pub height: u16,
    /// Horizontal distance from the square's anchor point to the sprite's left edge, in
    /// pixels. The anchor is the bottom center of the square's drawing area.
    pub offset_x: i16,
    /// Vertical distance from the square's anchor point to the sprite's top edge, in pixels.
    pub offset_y: i16,
    /// [`FLAG_OPAQUE`] and [`FLAG_EMPTY`].
    pub flags: u16,
    pub(crate) __pad: u16,
}

const _: () = assert!(size_of::<TexMeta>() == 16);
const _: () = assert!(align_of::<TexMeta>() == 4);

impl TexMeta {
    /// The entry for a tile with no sprite. It has zero size and [`FLAG_EMPTY`] set.
    pub const MISSING: TexMeta = TexMeta {
        offset: 0,
        width: 0,
        height: 0,
        offset_x: 0,
        offset_y: 0,
        flags: FLAG_EMPTY,
        __pad: 0,
    };

    /// Returns true if [`FLAG_EMPTY`] is set.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.flags & FLAG_EMPTY != 0
    }

    /// Returns true if [`FLAG_OPAQUE`] is set.
    #[inline]
    pub const fn is_opaque(&self) -> bool {
        self.flags & FLAG_OPAQUE != 0
    }

    /// Returns the size of the sprite's pixel data, in bytes.
    #[inline]
    pub const fn len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// One sprite's pixels and position, borrowed from a [`TextureLibrary`].
#[derive(Copy, Clone, Debug)]
pub struct Sprite<'a> {
    /// RGBA pixels with premultiplied alpha, 4 bytes per pixel, row by row with no padding.
    ///
    /// Premultiplied means each color channel is already multiplied by the pixel's alpha.
    pub pixels: &'a [u8],
    /// Width, in pixels.
    pub width: u32,
    /// Height, in pixels.
    pub height: u32,
    /// Horizontal distance from the square's anchor point to the sprite's left edge. See
    /// [`TexMeta::offset_x`].
    pub offset_x: i32,
    /// Vertical distance from the square's anchor point to the sprite's top edge.
    pub offset_y: i32,
    /// True if every pixel is fully opaque.
    pub opaque: bool,
}

/// The sprites a map uses, with all their pixels in one allocation.
///
/// Sprites have premultiplied alpha and are cropped to the area where they have visible pixels.
/// The metadata table has one entry per tile id, so a lookup is one array index.
pub struct TextureLibrary {
    blob: Box<[u8]>,
    meta: Box<[TexMeta]>,
}

impl TextureLibrary {
    /// Creates a library from the sprites' pixel data and their metadata, indexed by tile id.
    ///
    /// The offsets in `meta` aren't checked against `blob` here. [`Self::get`] checks them on
    /// each lookup instead.
    pub fn new(blob: Box<[u8]>, meta: Box<[TexMeta]>) -> Self {
        TextureLibrary { blob, meta }
    }

    /// Returns the sprite for tile `id`.
    ///
    /// Returns `None` if the tile has no sprite, or if its metadata points outside the pixel
    /// data.
    #[inline]
    pub fn get(&self, id: TileId) -> Option<Sprite<'_>> {
        let meta = self.meta.get(id.index())?;
        if meta.is_empty() {
            return None;
        }

        let start = meta.offset as usize;
        let end = start.checked_add(meta.len())?;
        Some(Sprite {
            pixels: self.blob.get(start..end)?,
            width: meta.width as u32,
            height: meta.height as u32,
            offset_x: meta.offset_x as i32,
            offset_y: meta.offset_y as i32,
            opaque: meta.is_opaque(),
        })
    }

    /// Returns the metadata of every tile, indexed by tile id.
    #[inline]
    pub fn metas(&self) -> &[TexMeta] {
        &self.meta
    }

    /// Returns the size of all the pixel data, in bytes.
    pub fn blob_len(&self) -> usize {
        self.blob.len()
    }

    /// Returns the number of tile ids the library has entries for.
    pub fn len(&self) -> usize {
        self.meta.len()
    }

    /// Returns true if the library has no entries.
    pub fn is_empty(&self) -> bool {
        self.meta.is_empty()
    }
}
