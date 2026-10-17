//! Builders for made-up input files and maps, so tests don't need a game install.

// A failure here is a bug in the test fixture, so panicking is the right response.
#![allow(clippy::expect_used)]

use crate::{
    runtime::raster::{
        CellBuilder,
        CellGeometry,
        Map,
    },
    string_table::{
        StringTableBuilder,
        TileNames,
    },
    textures::{
        FLAG_OPAQUE,
        TexMeta,
        TextureLibrary,
    },
};
use image::{
    ExtendedColorType,
    ImageEncoder as _,
    codecs::png::PngEncoder,
};
use libpzmap::B42;

/// PNG's CRC-32, computed without a lookup table.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;

    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }

    !crc
}

/// Adler-32, the checksum a zlib stream ends with.
fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);

    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }

    (b << 16) | a
}

/// Wraps `payload` in a zlib stream using a single stored (uncompressed) deflate block. A PNG
/// decoder accepts this exactly as it accepts a compressed one.
fn zlib_stored(payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("fixture payload fits in one stored block");

    let mut out = Vec::with_capacity(payload.len() + 11);
    // CMF/FLG: deflate, 32 KiB window, no preset dictionary. 0x7801 is divisible by 31, the header
    // check zlib requires.
    out.extend_from_slice(&[0x78, 0x01]);
    // One final stored block.
    out.push(0x01);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&adler32(payload).to_be_bytes());

    out
}

/// Appends one PNG chunk: length, type, data, CRC over type and data.
fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).expect("fixture chunk fits in a u32");

    let mut typed = Vec::with_capacity(4 + data.len());
    typed.extend_from_slice(kind);
    typed.extend_from_slice(data);

    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&typed);
    out.extend_from_slice(&crc32(&typed).to_be_bytes());
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// An RGBA8 IHDR chunk body for the given dimensions.
fn ihdr(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity(13);
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.push(8); // bit depth
    data.push(6); // color type 6: RGBA
    data.push(0); // deflate
    data.push(0); // adaptive filtering
    data.push(0); // no interlace
    data
}

/// A PNG whose header claims `width` x `height` pixels, but which holds only a few bytes of
/// pixel data.
///
/// A decoder that trusts the header would allocate `width * height * 4` bytes for a file under
/// 100 bytes. The pixel data is valid compressed data, so code that checks the size first fails
/// on the size, not on bad compression.
#[must_use]
pub fn oversized_png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(96);
    out.extend_from_slice(&PNG_SIGNATURE);
    push_chunk(&mut out, b"IHDR", &ihdr(width, height));

    // Filter byte 0 (None), then two RGBA pixels, far short of one scanline.
    let scanline = [0_u8, 255, 0, 0, 255, 0, 255, 0, 255];
    push_chunk(&mut out, b"IDAT", &zlib_stored(&scanline));
    push_chunk(&mut out, b"IEND", &[]);

    out
}

/// A valid RGBA PNG filled with one color.
///
/// The valid counterpart to [`oversized_png`], for checking that a page at the size limit still
/// decodes.
#[must_use]
pub fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for _ in 0..(width as usize) * (height as usize) {
        pixels.extend_from_slice(&rgba);
    }

    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
        .expect("a solid RGBA8 image encodes");

    out
}

/// One sprite's entry in a page's texture table.
///
/// The same fields as `libpzmap::texturepack::Texture`. `x`, `y`, `w` and `h` are the sprite's
/// rectangle inside the page. `ox`, `oy`, `ow` and `oh` are the offset and size of the uncropped
/// original.
#[derive(Clone, Debug)]
pub struct TextureSpec {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub ox: i32,
    pub oy: i32,
    pub ow: i32,
    pub oh: i32,
}

impl TextureSpec {
    /// A sprite that fills a whole `w` x `h` page, uncropped.
    #[must_use]
    pub fn whole_page(name: &str, w: i32, h: i32) -> Self {
        TextureSpec {
            name: name.to_owned(),
            x: 0,
            y: 0,
            w,
            h,
            ox: 0,
            oy: 0,
            ow: w,
            oh: h,
        }
    }
}

/// One texture page: a name, its sprites, and the encoded PNG.
#[derive(Clone, Debug)]
pub struct PageSpec {
    pub name: String,
    pub has_alpha: bool,
    pub textures: Vec<TextureSpec>,
    pub png: Vec<u8>,
}

impl PageSpec {
    /// A page carrying `png` and one sprite covering all of it.
    #[must_use]
    pub fn single_sprite(name: &str, sprite: &str, w: i32, h: i32, png: Vec<u8>) -> Self {
        PageSpec {
            name: name.to_owned(),
            has_alpha: true,
            textures: vec![TextureSpec::whole_page(sprite, w, h)],
            png,
        }
    }
}

/// A length-prefixed string, the form every name in a `.pack` takes.
fn push_tlv_string(out: &mut Vec<u8>, value: &str) {
    let len = u32::try_from(value.len()).expect("fixture string fits in a u32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

/// A texture pack file in the `PZPK` format, version 1.
///
/// Version 1 stores each page's PNG length before the PNG. Version 0 instead ends each PNG with
/// a `0xDEADBEEF` marker.
#[must_use]
pub fn texture_pack(pages: &[PageSpec]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PZPK");
    out.extend_from_slice(&1_u32.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(pages.len())
            .expect("fixture page count fits in a u32")
            .to_le_bytes(),
    );

    for page in pages {
        push_tlv_string(&mut out, &page.name);
        out.extend_from_slice(
            &u32::try_from(page.textures.len())
                .expect("fixture texture count fits in a u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::from(page.has_alpha).to_le_bytes());

        for texture in &page.textures {
            push_tlv_string(&mut out, &texture.name);
            for field in [
                texture.x, texture.y, texture.w, texture.h, texture.ox, texture.oy, texture.ow,
                texture.oh,
            ] {
                out.extend_from_slice(&field.to_le_bytes());
            }
        }

        out.extend_from_slice(
            &u32::try_from(page.png.len())
                .expect("fixture PNG fits in a u32")
                .to_le_bytes(),
        );
        out.extend_from_slice(&page.png);
    }

    out
}

/// Returns the game install directory named by `PZ_INSTALL_DIR`.
///
/// # Panics
///
/// Panics if `PZ_INSTALL_DIR` is unset.
pub fn install_dir() -> std::path::PathBuf {
    std::env::var_os("PZ_INSTALL_DIR").map(std::path::PathBuf::from).expect(
        "set PZ_INSTALL_DIR to a Project Zomboid Build 42 installation to run the ignored tests",
    )
}

/// Pseudo-random bytes from splitmix64. They don't compress, and the same seed always gives
/// the same bytes.
pub fn noise(seed: u64) -> impl FnMut() -> u8 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)).to_le_bytes()[0]
    }
}

/// A small in-memory [`Map`] with one opaque sprite, for testing rendering without a game
/// install.
///
/// `cells` are cell coordinates. Every square in each cell gets the same sprite. Keep
/// `sprite_px` small, because every square is drawn and a large sprite makes the test slow.
#[must_use]
pub fn synthetic_map(cells: &[(i32, i32)], sprite_px: u16) -> Map {
    let names = StringTableBuilder::<TileNames>::new();
    let id = names.intern("test_sprite").expect("interning a tile name");

    // One opaque RGBA sprite. Premultiplying a fully opaque pixel doesn't change it.
    let len = usize::from(sprite_px) * usize::from(sprite_px) * 4;
    let blob = vec![255_u8; len].into_boxed_slice();

    let meta = vec![TexMeta {
        offset: 0,
        width: sprite_px,
        height: sprite_px,
        offset_x: -(i16::try_from(sprite_px).unwrap_or(0) / 2),
        offset_y: -i16::try_from(sprite_px).unwrap_or(0),
        flags: FLAG_OPAQUE,
        ..Default::default()
    }]
    .into_boxed_slice();

    let geometry = CellGeometry::of::<B42>();
    let squares = geometry.cell_squares() as usize;

    let built = cells
        .iter()
        .map(|&(x, y)| {
            let mut builder = CellBuilder::new(x, y, geometry);
            for sub_y in 0..squares {
                for sub_x in 0..squares {
                    builder.push_square(0, sub_x, sub_y, std::iter::once(id));
                }
            }
            builder.build().expect("a synthetic cell builds")
        })
        .collect();

    Map::new(
        built,
        geometry,
        TextureLibrary::new(blob, meta),
        names.build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageDecoder as _;
    use libpzmap::texturepack::TexturePack;

    #[test]
    fn an_oversized_png_is_small_but_claims_to_be_huge() {
        let bytes = oversized_png(65535, 65535);
        assert!(bytes.len() < 128, "fixture grew to {} bytes", bytes.len());

        let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&bytes))
            .expect("the header is well-formed even though it lies");

        assert_eq!(decoder.dimensions(), (65535, 65535));
        // This is what a decoder that trusts the header would allocate.
        assert!(decoder.total_bytes() > 17_000_000_000);
    }

    #[test]
    fn a_solid_png_round_trips() {
        let bytes = solid_png(4, 2, [1, 2, 3, 255]);
        let image = image::load_from_memory(&bytes)
            .expect("a solid PNG decodes")
            .into_rgba8();

        assert_eq!(image.dimensions(), (4, 2));
        assert!(image.pixels().all(|p| p.0 == [1, 2, 3, 255]));
    }

    #[test]
    fn a_built_pack_parses_back() {
        let png = solid_png(8, 8, [9, 9, 9, 255]);
        let pack_bytes = texture_pack(&[
            PageSpec::single_sprite("page_0", "sprite_a", 8, 8, png.clone()),
            PageSpec::single_sprite("page_1", "sprite_b", 8, 8, png),
        ]);

        let pack = TexturePack::parse(&pack_bytes).expect("a generated pack parses");

        assert_eq!(pack.pages.len(), 2);
        assert_eq!(pack.pages[0].name, "page_0");
        assert_eq!(pack.pages[0].textures[0].name, "sprite_a");
        assert_eq!(pack.pages[1].textures[0].name, "sprite_b");
        assert_eq!(pack.texture_count(), 2);

        for page in &pack.pages {
            image::load_from_memory(page.png).expect("each page's PNG survives the round trip");
        }
    }

    #[test]
    fn a_pack_can_carry_a_hostile_page() {
        let pack_bytes = texture_pack(&[PageSpec::single_sprite(
            "huge",
            "sprite",
            64,
            64,
            oversized_png(65535, 65535),
        )]);

        let pack = TexturePack::parse(&pack_bytes)
            .expect("the pack itself is well-formed; only the page lies");

        assert_eq!(pack.pages.len(), 1);
        assert!(pack.pages[0].png.len() < 128);
    }
}
