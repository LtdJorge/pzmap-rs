use crate::{
    build::repeat,
    error::{
        Format,
        ParseError,
        ParseErrorKind,
        Result,
    },
    input::Input,
    texturepack::{
        Page,
        Texture,
        TexturePack,
    },
};
use tracing::{
    debug,
    info,
};

/// The bytes at the end of each page's image in version 0 texture packs: `0xDEADBEEF` in little-endian.
const PAGE0_PNG_TERMINATOR: &[u8] = &[0xef, 0xbe, 0xad, 0xde];

/// The smallest size each repeated element can have in the file.
///
/// Counts needing more bytes than are left in the buffer are rejected before allocating.
mod min_bytes {
    /// A name length and eight `i32` fields.
    pub const TEXTURE: usize = 4 + 8 * 4;
    /// A name length, a texture count, and an alpha flag.
    pub const PAGE: usize = 4 + 4 + 4;
}

fn parse_texture<'inp>(input: &mut Input<'inp>) -> Result<Texture<'inp>> {
    Ok(Texture {
        name: input.tlv_string("texture name")?,
        x: input.i32_le()?,
        y: input.i32_le()?,
        width: input.i32_le()?,
        height: input.i32_le()?,
        offset_x: input.i32_le()?,
        offset_y: input.i32_le()?,
        original_width: input.i32_le()?,
        original_height: input.i32_le()?,
    })
}

/// Reads one page.
///
/// In version 0 the PNG ends with the bytes of `0xDEADBEEF`. In version 1 its length comes
/// first.
fn parse_page<'inp>(input: &mut Input<'inp>, version: u32) -> Result<Page<'inp>> {
    let name = input.tlv_string("page name")?;
    debug!(page_name = name);

    let texture_count = input.u32_le()?;
    debug!(textures = texture_count);
    let texture_count =
        input.checked_count("texture count", texture_count as i64, min_bytes::TEXTURE)?;

    let has_alpha = input.u32_le()? > 0;
    let textures = repeat(input, texture_count, parse_texture)?;

    let png = if version == 0 {
        input.take_until_and_skip(PAGE0_PNG_TERMINATOR)?
    } else {
        let png_length = input.u32_le()?;
        input.take(png_length as usize)?
    };

    Ok(Page {
        name,
        has_alpha,
        textures,
        png,
    })
}

impl<'inp> TexturePack<'inp> {
    /// Parses a `.pack` file.
    ///
    /// The parser works out the file's version itself. Newer files start with `PZPK` and a
    /// version number. Older files have no header and start with the page count. Packs do not
    /// depend on the game build, so there is no build parameter.
    ///
    /// ```no_run
    /// use libpzmap::texturepack::TexturePack;
    ///
    /// let bytes = std::fs::read("Tiles2x.pack")?;
    /// let pack = TexturePack::parse(&bytes)?;
    /// println!("{} pages, {} textures", pack.pages.len(), pack.texture_count());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the file is truncated, if a count is larger than the remaining data,
    /// if a name is not UTF-8, or if the `PZPK` version is not 0 or 1.
    pub fn parse(bytes: &'inp [u8]) -> Result<Self> {
        let mut input = Input::new(bytes, Format::TexturePack);

        if !input.starts_with(b"PZPK") {
            let page_count = input.u32_le()?;
            info!(page_count, pack_shape = "bare_legacy_v0");

            let page_count =
                input.checked_count("page count", page_count as i64, min_bytes::PAGE)?;
            let pages = repeat(&mut input, page_count, |inp| parse_page(inp, 0))?;

            return Ok(TexturePack { pages });
        }

        input.magic("PZPK")?;

        let at = input.offset();
        let envelope_version = input.u32_le()?;
        let page_count = input.u32_le()?;

        info!(envelope_version, page_count, pack_shape = "pzpk");

        let page_count = input.checked_count("page count", page_count as i64, min_bytes::PAGE)?;

        let pages = match envelope_version {
            version @ (0 | 1) => repeat(&mut input, page_count, |inp| parse_page(inp, version))?,
            found => {
                return Err(ParseError {
                    format: Format::TexturePack,
                    at,
                    kind: ParseErrorKind::Version {
                        what: "PZPK envelope",
                        found,
                    },
                });
            }
        };

        Ok(TexturePack { pages })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn test_pack() {
        let bytes = fixtures::read(&fixtures::texture_pack("Tiles2x.pack"));
        let pack = TexturePack::parse(&bytes).expect("Tiles2x.pack should parse");

        assert!(!pack.pages.is_empty());
        for page in &pack.pages {
            assert!(!page.png.is_empty(), "page {} has no PNG", page.name);
        }

        println!(
            "{} pages, {} textures",
            pack.pages.len(),
            pack.texture_count()
        );
    }

    #[test]
    fn an_unsupported_envelope_version_is_reported() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PZPK");
        bytes.extend_from_slice(&9_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());

        let err = TexturePack::parse(&bytes).expect_err("envelope version 9 is not supported");

        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::TexturePack,
                    kind: ParseErrorKind::Version { found: 9, .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn an_implausible_page_count_is_rejected_before_allocating() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PZPK");
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());

        let err = TexturePack::parse(&bytes).expect_err("4 billion pages cannot fit in 12 bytes");

        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::ImplausibleCount { .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_missing_v0_sentinel_is_an_error_not_a_hang() {
        // An old pack with no header: one page with an empty name, no textures and no alpha,
        // then a PNG without the end marker.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(b"not a terminated png");

        let err = TexturePack::parse(&bytes).expect_err("an unterminated v0 page must be rejected");

        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::UnexpectedEof { .. },
                    ..
                }
            ),
            "unexpected error: {err}"
        );
    }
}
