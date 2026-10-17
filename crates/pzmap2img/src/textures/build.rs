//! Builds a [`TextureLibrary`] from the game's texture packs.
//!
//! Only the sprites the map uses are decoded, in two passes:
//!
//! 1. Read every pack's sprite tables without decoding any PNG. When several packs have a
//!    sprite with the same name, the one loaded last wins. Add up the sprite sizes so each page
//!    gets its own range in one shared pixel buffer.
//! 2. Allocate the pixel buffer once and split it into one slice per page. Decode the pages in
//!    parallel, each writing only into its own slice.
//!
//! This way memory holds the finished buffer plus one decoded page per thread, never every
//! page at once.
//!
//! Each sprite is copied with premultiplied alpha, and cropped to the area where it has visible
//! pixels. The pack's own crop is often looser.

use crate::{
    budget::BuildCtx,
    config::{
        BuildLimits,
        InputTrust,
    },
    error::{
        BuildError,
        Result,
    },
    source::SourceBytes,
    string_table::{
        StringTableReader,
        TileId,
        TileNames,
    },
    textures::{
        library::{
            FLAG_OPAQUE,
            TexMeta,
            TextureLibrary,
        },
        plants::{
            PlantsConfig,
            plant_mapping,
        },
    },
};
use ahash::{
    HashMap,
    HashMapExt as _,
};
use image::{
    ExtendedColorType,
    ImageDecoder,
    codecs::png::PngDecoder,
};
use libpzmap::texturepack::{
    Page,
    Texture,
    TexturePack,
};
use rayon::prelude::*;
use std::{
    io::Cursor,
    path::Path,
};
use tracing::{
    debug,
    info,
    instrument,
    warn,
};

/// Returns the position of the sprite's top-left corner relative to its anchor point.
///
/// The anchor is the bottom center of the sprite's uncropped image, which is where it sits on
/// its square. pzmap2dzi computes the same offset.
#[inline]
pub(crate) fn anchor(texture: &Texture<'_>) -> (i32, i32) {
    (
        texture.offset_x - (texture.original_width >> 1),
        texture.offset_y - texture.original_height,
    )
}

/// Returns the size of the sprite's cropped area once decoded to RGBA, in bytes.
#[inline]
pub(crate) fn rgba_len(texture: &Texture<'_>) -> usize {
    // `extract_selected` skips textures with a negative size, so these conversions succeed. If
    // one did slip through, 0 is safer than wrapping to a huge size.
    let w = usize::try_from(texture.width).unwrap_or(0);
    let h = usize::try_from(texture.height).unwrap_or(0);
    w * h * 4
}

/// A texture pack's file name and bytes. The bytes are memory-mapped or read into memory
/// depending on [`InputTrust`].
pub(crate) struct LoadedPack {
    pub(crate) name: String,
    bytes: SourceBytes,
}

impl LoadedPack {
    /// Opens a pack, and refuses one larger than [`BuildLimits::max_input_bytes`].
    pub(crate) fn open(
        path: impl AsRef<Path>,
        trust: InputTrust,
        limits: &BuildLimits,
    ) -> Result<Self> {
        let path = path.as_ref();
        let bytes = SourceBytes::open(path, trust, limits.max_input_bytes, "texture pack")?;

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        Ok(LoadedPack { name, bytes })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Counts from building the texture library.
#[derive(Copy, Clone, Debug, Default)]
#[non_exhaustive]
pub struct TextureStats {
    /// The size of all sprite pixels, in bytes.
    pub blob_bytes: usize,
    /// Tiles that got a sprite.
    pub textures: usize,
    /// Tiles with no sprite in any loaded pack.
    pub missing: usize,
    /// Pages that were decoded.
    pub pages_decoded: usize,
    /// Pages in all loaded packs.
    pub pages_total: usize,
    /// Bytes set aside but left unused, because cropping made sprites smaller than the pack
    /// said.
    pub slack_bytes: usize,
    /// Vegetation sprites made by combining other sprites.
    pub composited: usize,
}

/// One page to decode, and the sprites to copy out of it.
struct PageJob<'a> {
    pack: usize,
    page: usize,
    /// Where this page's slice starts in the pixel buffer.
    base: usize,
    /// The length of this page's slice.
    len: usize,
    /// `(slot, texture, offset within this page's slice)` for each sprite.
    entries: Vec<(usize, Texture<'a>, usize)>,
}

/// One copied sprite's metadata. The calling thread puts these into the metadata table.
struct Extracted {
    slot: usize,
    meta: TexMeta,
    slack: usize,
}

/// Builds the texture library for the tile names in `tiles`, from `packs` in load order.
///
/// When `plants` is given, it also combines the vegetation sprites.
#[cfg_attr(debug_assertions, instrument(skip_all, fields(packs = packs.len(), names = tiles.len())))]
pub(crate) fn build_texture_library(
    tiles: &StringTableReader<TileNames>,
    packs: &[LoadedPack],
    plants: Option<&PlantsConfig>,
    ctx: BuildCtx<'_>,
) -> Result<(TextureLibrary, TextureStats)> {
    // Read the sprite tables only. No PNG is decoded here.
    let parsed: Vec<TexturePack<'_>> = packs
        .iter()
        .map(|pack| {
            tracing::info_span!("texture_pack", name = pack.name.as_str())
                .in_scope(|| TexturePack::parse(pack.bytes()))
                .map_err(|source| BuildError::ParsePack {
                    pack: pack.name.clone(),
                    source,
                })
        })
        .collect::<Result<_>>()?;

    for (pack, parsed) in packs.iter().zip(&parsed) {
        debug!(
            pack = pack.name,
            pages = parsed.pages.len(),
            textures = parsed.texture_count(),
        );
    }

    let pages_total: usize = parsed.iter().map(|p| p.pages.len()).sum();

    // Copy out the sprites the map uses.
    let mut extracted = extract_selected(
        &parsed,
        |name| tiles.resolve_string(name).map(|key| key.index()),
        tiles.len(),
        ctx,
    )?;

    let mut blob = std::mem::take(&mut extracted.blob);
    let mut meta = std::mem::take(&mut extracted.meta);

    let composited = match plants {
        Some(config) => composite_plants(config, tiles, &parsed, &mut blob, &mut meta, ctx)?,
        None => 0,
    };

    let missing = meta.iter().filter(|m| m.is_empty()).count();
    if missing > 0 {
        warn!(
            missing,
            total = meta.len(),
            "tile names with no sprite in the loaded packs"
        );
    }

    let stats = TextureStats {
        blob_bytes: blob.len(),
        textures: meta.iter().filter(|m| !m.is_empty()).count(),
        missing,
        pages_decoded: extracted.pages_decoded,
        pages_total,
        slack_bytes: extracted.slack_bytes,
        composited,
    };

    Ok((
        TextureLibrary::new(blob.into_boxed_slice(), meta.into_boxed_slice()),
        stats,
    ))
}

/// Sprites copied out of the packs.
#[derive(Debug)]
struct Extraction {
    blob: Vec<u8>,
    /// One entry per slot. Slots with no sprite are [`TexMeta::MISSING`].
    meta: Vec<TexMeta>,
    pages_decoded: usize,
    slack_bytes: usize,
}

/// Copies every sprite whose name `resolve` maps to a slot into a new pixel buffer.
///
/// `resolve` returns the slot for a sprite name, or `None` to skip it. Slots run from 0 to
/// `slots`. This is used for the map's tiles, and for the parts of the vegetation sprites.
fn extract_selected(
    parsed: &[TexturePack<'_>],
    resolve: impl Fn(&str) -> Option<usize>,
    slots: usize,
    ctx: BuildCtx<'_>,
) -> Result<Extraction> {
    // The pack, page and entry that fills each slot. A later pack replaces an earlier one, as
    // in pzmap2dzi.
    let mut winner: Vec<Option<(usize, usize, usize)>> = vec![None; slots];
    for (pack_ix, pack) in parsed.iter().enumerate() {
        for (page_ix, page) in pack.pages.iter().enumerate() {
            for (tex_ix, texture) in page.textures.iter().enumerate() {
                if texture.width <= 0 || texture.height <= 0 {
                    continue;
                }
                if let Some(slot) = resolve(texture.name) {
                    // `resolve` should only return slots below `slots`. `get_mut` ignores one
                    // that doesn't.
                    if let Some(entry) = winner.get_mut(slot) {
                        *entry = Some((pack_ix, page_ix, tex_ix));
                    }
                }
            }
        }
    }

    // Sorting by pack and page puts each page's sprites next to each other in the buffer. It
    // also gives the same layout on every run.
    let mut claims: Vec<(usize, usize, usize, usize)> = winner
        .iter()
        .enumerate()
        .filter_map(|(slot, w)| {
            let (pack, page, tex) = (*w)?;
            Some((pack, page, tex, slot))
        })
        .collect();
    claims.sort_unstable();

    let mut jobs: Vec<PageJob<'_>> = Vec::new();
    let mut cursor = 0_usize;

    for group in claims.chunk_by(|a, b| a.0 == b.0 && a.1 == b.1) {
        let Some(&(pack, page, ..)) = group.first() else {
            continue;
        };

        let base = cursor;
        let mut entries = Vec::with_capacity(group.len());

        for &(_, _, tex_ix, slot) in group {
            let Some(texture) = parsed
                .get(pack)
                .and_then(|p| p.pages.get(page))
                .and_then(|p| p.textures.get(tex_ix))
                .copied()
            else {
                continue;
            };

            entries.push((slot, texture, cursor - base));

            // Checking the limit here rejects a malicious pack before the buffer is allocated.
            // `checked_add` also catches a total that would overflow.
            cursor = cursor
                .checked_add(rgba_len(&texture))
                .ok_or(BuildError::TextureBudget {
                    bytes: u64::MAX,
                    max: ctx.limits.max_blob_bytes,
                })?;

            if cursor as u64 > ctx.limits.max_blob_bytes {
                return Err(BuildError::TextureBudget {
                    bytes: cursor as u64,
                    max: ctx.limits.max_blob_bytes,
                });
            }
        }

        jobs.push(PageJob {
            pack,
            page,
            base,
            len: cursor - base,
            entries,
        });
    }

    info!(
        blob_mb = cursor as f64 / 1e6,
        pages = jobs.len(),
        textures = claims.len(),
        "extraction sized"
    );

    // Never released, because this buffer becomes the `TextureLibrary`.
    ctx.charge("texture blob", cursor as u64)?;

    let mut blob = vec![0_u8; cursor];

    // The pages' ranges are in order with no gaps, so `split_at_mut` can cut one separate
    // `&mut [u8]` per page. Each thread then writes its own slice without locking.
    let mut chunks: Vec<&mut [u8]> = Vec::with_capacity(jobs.len());
    {
        let mut rest = blob.as_mut_slice();
        for job in &jobs {
            let (head, tail) = rest.split_at_mut(job.len);
            chunks.push(head);
            rest = tail;
        }
        debug_assert!(rest.is_empty());
    }

    // `pack` and `page` come from enumerating `parsed`, so the page always exists. Looking it
    // up with `get` avoids a panic path all the same.
    let page_of = |job: &PageJob<'_>| {
        parsed
            .get(job.pack)
            .and_then(|pack| pack.pages.get(job.page))
    };

    let results: Vec<Vec<Extracted>> = jobs
        .par_iter()
        .zip(chunks.par_iter_mut())
        .map(|(job, chunk)| match page_of(job) {
            Some(page) => extract_page(job, page, chunk, ctx),
            None => Ok(Vec::new()),
        })
        .collect::<Result<_>>()?;

    let mut meta = vec![TexMeta::MISSING; slots];
    let mut slack_bytes = 0_usize;

    for page in results {
        for item in page {
            // `resolve` should only return slots below `slots`, the length of `meta`. `get_mut`
            // ignores one that doesn't.
            if let Some(entry) = meta.get_mut(item.slot) {
                *entry = item.meta;
            }
            slack_bytes += item.slack;
        }
    }

    Ok(Extraction {
        blob,
        meta,
        pages_decoded: jobs.len(),
        slack_bytes,
    })
}

/// The width of the canvas vegetation sprites are combined on. pzmap2dzi uses the same size.
const COMPOSITE_W: i32 = 384;
/// The height of the canvas vegetation sprites are combined on.
const COMPOSITE_H: i32 = 512;

/// Builds the `vegetation_*` and `jumbo_tree_*` sprites and adds them to the pixel buffer.
///
/// No pack has sprites with these names. The game draws them from several parts, such as a
/// trunk and a seasonal layer of leaves. This draws the parts onto one 384x512 canvas, anchored
/// at the bottom center, as pzmap2dzi does. The result replaces any entry `meta` already has
/// for these names. Returns the number of sprites built.
#[instrument(skip_all)]
fn composite_plants(
    config: &PlantsConfig,
    tiles: &StringTableReader<TileNames>,
    parsed: &[TexturePack<'_>],
    blob: &mut Vec<u8>,
    meta: &mut [TexMeta],
    ctx: BuildCtx<'_>,
) -> Result<usize> {
    // Only build the sprites the map uses.
    let mapping: Vec<(TileId, Vec<String>)> = plant_mapping(config)
        .into_iter()
        .filter_map(|(key, sources)| Some((tiles.resolve_string(&key)?, sources)))
        .filter(|(_, sources)| !sources.is_empty())
        .collect();

    if mapping.is_empty() {
        return Ok(0);
    }

    // The map never names the parts, so they have no tile ids. They get their own slots and
    // their own temporary pixel buffer.
    let mut source_slot: HashMap<&str, usize> = HashMap::new();
    for (_, sources) in &mapping {
        for name in sources {
            let next = source_slot.len();
            source_slot.entry(name.as_str()).or_insert(next);
        }
    }

    let sources = extract_selected(
        parsed,
        |name| source_slot.get(name).copied(),
        source_slot.len(),
        ctx,
    )?;

    let missing_sources = sources.meta.iter().filter(|m| m.is_empty()).count();
    if missing_sources > 0 {
        warn!(
            missing_sources,
            total = source_slot.len(),
            "vegetation source sprites absent from the loaded packs"
        );
    }

    let anchor_x = COMPOSITE_W / 2;
    let anchor_y = COMPOSITE_H;
    let mut canvas = vec![0_u8; (COMPOSITE_W * COMPOSITE_H * 4) as usize];
    let mut built = 0_usize;

    for (key, names) in &mapping {
        canvas.fill(0);

        for name in names {
            let Some(&slot) = source_slot.get(name.as_str()) else {
                continue;
            };
            let Some(source) = sources.meta.get(slot).copied() else {
                continue;
            };
            if source.is_empty() {
                continue;
            }

            let start = source.offset as usize;
            let Some(pixels) = sources.blob.get(start..start + source.len()) else {
                continue;
            };

            blit_over(
                &mut canvas,
                pixels,
                source.width as i32,
                source.height as i32,
                anchor_x + source.offset_x as i32,
                anchor_y + source.offset_y as i32,
            );
        }

        // Crop to the visible pixels and adjust the anchor offset to match, as pzmap2dzi
        // does.
        let Some(slot) = meta.get_mut(key.index()) else {
            continue;
        };

        let Some((x0, y0, x1, y1)) = alpha_bbox(&canvas, COMPOSITE_W, COMPOSITE_H) else {
            *slot = TexMeta::MISSING;
            continue;
        };

        let (w, h) = (x1 - x0, y1 - y0);
        let offset = blob.len();
        let mut opaque = true;

        // The box is inside the canvas, so these conversions succeed. Converting once here
        // keeps the copy loop free of casts.
        let (Ok(ux0), Ok(uy0), Ok(uy1), Ok(uw)) = (
            usize::try_from(x0),
            usize::try_from(y0),
            usize::try_from(y1),
            usize::try_from(w),
        ) else {
            continue;
        };
        let stride = COMPOSITE_W as usize * 4;

        for y in uy0..uy1 {
            let row = y * stride + ux0 * 4;
            let Some(slice) = canvas.get(row..row + uw * 4) else {
                continue;
            };
            opaque &= slice
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p.last() == Some(&255));
            blob.extend_from_slice(slice);
        }

        // The canvas is 384x512, so the size and anchor offset always fit in `u16` and `i16`.
        // Only the buffer offset can overflow, with enough mod packs.
        *slot = TexMeta {
            offset: blob_offset(offset)?,
            width: u16::try_from(w).unwrap_or(u16::MAX),
            height: u16::try_from(h).unwrap_or(u16::MAX),
            offset_x: i16::try_from(x0 - anchor_x).unwrap_or(0),
            offset_y: i16::try_from(y0 - anchor_y).unwrap_or(0),
            flags: if opaque { FLAG_OPAQUE } else { 0 },
            ..Default::default()
        };
        built += 1;
    }

    info!(
        composited = built,
        sources = source_slot.len(),
        blob_mb = (blob.len() as f64) / 1e6,
        "vegetation composites built"
    );

    Ok(built)
}

/// Draws a sprite over the vegetation canvas at (`dx`, `dy`), clipped to the canvas.
///
/// Both images have premultiplied alpha, so each channel is `d = s + d * (1 - sa)`. This gives
/// the same result as PIL's `alpha_composite`, which pzmap2dzi uses. The indexing can't go out
/// of bounds: `x` and `y` are clipped to the canvas, and `row` and `col` stay inside `src`,
/// which is `w * h * 4` bytes.
#[allow(
    clippy::indexing_slicing,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn blit_over(canvas: &mut [u8], src: &[u8], w: i32, h: i32, dx: i32, dy: i32) {
    for row in 0..h {
        let y = dy + row;
        if !(0..COMPOSITE_H).contains(&y) {
            continue;
        }

        for col in 0..w {
            let x = dx + col;
            if !(0..COMPOSITE_W).contains(&x) {
                continue;
            }

            let s = ((row * w + col) * 4) as usize;
            let d = ((y * COMPOSITE_W + x) * 4) as usize;

            let inverse = 255 - src[s + 3] as u32;
            if inverse == 0 {
                canvas[d..d + 4].copy_from_slice(&src[s..s + 4]);
                continue;
            }

            for channel in 0..4 {
                let t = canvas[d + channel] as u32 * inverse + 128;
                let under = (t + (t >> 8)) >> 8;
                canvas[d + channel] = (src[s + channel] as u32 + under).min(255) as u8;
            }
        }
    }
}

/// Returns the smallest box holding every visible pixel, as `(x0, y0, x1, y1)`.
///
/// `x1` and `y1` are exclusive. Returns `None` if every pixel is transparent. Callers pass
/// `pixels` of `w * h * 4` bytes, so the indexing stays in bounds.
#[allow(clippy::indexing_slicing, clippy::cast_sign_loss)]
fn alpha_bbox(pixels: &[u8], w: i32, h: i32) -> Option<(i32, i32, i32, i32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);

    for y in 0..h {
        let row = (y * w * 4) as usize;
        for x in 0..w {
            if pixels[row + (x * 4) as usize + 3] != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }

    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Releases its bytes from the budget when dropped, so an early return can't leak them.
struct ReleaseOnDrop<'a> {
    ctx: BuildCtx<'a>,
    bytes: u64,
}

impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        self.ctx.release(self.bytes);
    }
}

/// Decodes one page and copies its selected sprites into `chunk`.
fn extract_page(
    job: &PageJob<'_>,
    page: &Page<'_>,
    chunk: &mut [u8],
    ctx: BuildCtx<'_>,
) -> Result<Vec<Extracted>> {
    let (page_pixels, page_w, page_h) = decode_page(page.png, page.name, ctx)?;

    // The decoded page is only needed until its sprites are copied. Releasing it from the
    // budget when this function returns means the budget counts one page per thread, not every
    // page.
    let scratch_bytes = page_pixels.len() as u64;
    let _release = ReleaseOnDrop {
        ctx,
        bytes: scratch_bytes,
    };

    let mut out = Vec::with_capacity(job.entries.len());

    for (slot, texture, local) in &job.entries {
        let reserved = rgba_len(texture);
        let Some(dst) = chunk.get_mut(*local..*local + reserved) else {
            // Can't happen: `local` and `reserved` come from the same loop that computed
            // `job.len`, and the chunk has exactly that length.
            unreachable_chunk(job.page, *local, reserved, chunk.len());
        };

        let item = match extract_sprite(&page_pixels, page_w, page_h, texture, dst)? {
            Some(sprite) => Extracted {
                slot: *slot,
                meta: TexMeta {
                    offset: blob_offset(job.base + *local)?,
                    width: sprite.width,
                    height: sprite.height,
                    offset_x: sprite.offset_x,
                    offset_y: sprite.offset_y,
                    flags: if sprite.opaque { FLAG_OPAQUE } else { 0 },
                    ..Default::default()
                },
                slack: reserved - sprite.written,
            },
            // Fully transparent, so there is nothing to draw.
            None => Extracted {
                slot: *slot,
                meta: TexMeta::MISSING,
                slack: reserved,
            },
        };

        out.push(item);
    }

    Ok(out)
}

/// Panics because a sprite didn't fit in its page's slice of the pixel buffer.
///
/// That would be a bug in this crate's sizing code, not something a bad input file can cause,
/// so it panics instead of returning an error. It is a separate function to keep `panic!` out
/// of `extract_page`.
#[cold]
#[inline(never)]
#[allow(clippy::panic)]
fn unreachable_chunk(page: usize, local: usize, reserved: usize, chunk: usize) -> ! {
    panic!("page {page} entry at {local} + {reserved} exceeds its {chunk}-byte chunk");
}

/// Converts a pixel buffer offset to the `u32` that [`TexMeta`] stores.
///
/// # Errors
///
/// Returns [`BuildError::BlobTooLarge`] if `offset` exceeds `u32::MAX`.
#[inline]
fn blob_offset(offset: usize) -> Result<u32> {
    u32::try_from(offset).map_err(|_| BuildError::BlobTooLarge {
        bytes: offset,
        max: u32::MAX,
    })
}

/// Decodes a page's PNG to RGBA, refusing one larger than the page limits allow.
///
/// Returns the pixels, the width and the height. The size from the PNG header is checked
/// before any pixels are decoded, so the error can name the page. `image`'s own limits are set
/// as well, for the slower path that converts pages not stored as 8-bit RGBA.
///
/// # Errors
///
/// Returns [`BuildError::PageTooLarge`] if the page's width, height, pixel count or decoded
/// size is over `ctx.limits`. Returns [`BuildError::Decode`] if the PNG is malformed.
fn decode_page(png: &[u8], page_name: &str, ctx: BuildCtx<'_>) -> Result<(Vec<u8>, u32, u32)> {
    let too_large = |w: u32, h: u32, pixels: u64, max: u64| BuildError::PageTooLarge {
        page: page_name.to_owned(),
        width: w,
        height: h,
        pixels,
        max,
    };

    // Limits what the decoder allocates while it reads the header. The checks below give
    // clearer errors for everything else.
    let mut header_limits = image::Limits::no_limits();
    header_limits.max_alloc = Some(ctx.limits.max_page_bytes);

    let mut decoder =
        PngDecoder::with_limits(Cursor::new(png), header_limits).map_err(|source| {
            BuildError::Decode {
                page: page_name.to_owned(),
                source: Box::new(source),
            }
        })?;

    let (w, h) = decoder.dimensions();
    let pixels = u64::from(w) * u64::from(h);

    if w > ctx.limits.max_page_dimension || h > ctx.limits.max_page_dimension {
        return Err(too_large(
            w,
            h,
            pixels,
            u64::from(ctx.limits.max_page_dimension),
        ));
    }

    if pixels > ctx.limits.max_page_pixels {
        return Err(too_large(w, h, pixels, ctx.limits.max_page_pixels));
    }

    let declared = decoder.total_bytes();
    if declared > ctx.limits.max_page_bytes {
        return Err(too_large(w, h, declared, ctx.limits.max_page_bytes));
    }

    let decode = |source: image::ImageError| BuildError::Decode {
        page: page_name.to_owned(),
        source: Box::new(source),
    };

    // These limits also bound `DynamicImage::from_decoder` below, which allocates its own
    // buffers.
    let mut full_limits = image::Limits::no_limits();
    full_limits.max_image_width = Some(ctx.limits.max_page_dimension);
    full_limits.max_image_height = Some(ctx.limits.max_page_dimension);
    full_limits.max_alloc = Some(ctx.limits.max_page_bytes);
    decoder.set_limits(full_limits).map_err(decode)?;

    if decoder.original_color_type() == ExtendedColorType::Rgba8
        && decoder.color_type() == image::ColorType::Rgba8
    {
        // `declared` is at most `max_page_bytes`, so this only fails on a 32-bit target where
        // that limit is over `usize::MAX`.
        let len = usize::try_from(declared)
            .map_err(|_| too_large(w, h, declared, ctx.limits.max_page_bytes))?;

        // Each thread holds one decoded page at a time. The caller releases it when it is done
        // with the page.
        ctx.charge("atlas page", declared)?;

        let mut buf = vec![0_u8; len];
        if let Err(err) = decoder.read_image(&mut buf) {
            ctx.release(declared);
            return Err(decode(err));
        }

        return Ok((buf, w, h));
    }

    ctx.charge("atlas page", declared)?;

    let image = match image::DynamicImage::from_decoder(decoder) {
        Ok(image) => image.into_rgba8(),
        Err(err) => {
            ctx.release(declared);
            return Err(decode(err));
        }
    };

    Ok((image.into_raw(), w, h))
}

/// One sprite copied out of its page: its cropped size, its anchor offset, and how many bytes
/// it wrote.
struct ExtractedSprite {
    width: u16,
    height: u16,
    /// Position relative to the anchor point. See [`TexMeta::offset_x`].
    offset_x: i16,
    offset_y: i16,
    /// True if every pixel is fully opaque.
    opaque: bool,
    /// Bytes written into `dst`. The caller set aside `rgba_len(texture)` bytes, and cropping
    /// often uses fewer. The difference is unused space in the pixel buffer.
    written: usize,
}

/// Copies one sprite out of a decoded page into `dst`, with premultiplied alpha.
///
/// The pack's crop of the sprite isn't always tight, so this crops it again to its visible
/// pixels and adjusts the anchor offset to match, as pzmap2dzi does. Returns `None` if the
/// sprite is fully transparent or its rectangle is outside the page.
///
/// The indexing stays in bounds: the `fits` check keeps every read inside the page, and the
/// caller makes `dst` `rgba_len(texture)` bytes long.
///
/// # Errors
///
/// Returns [`BuildError::SpriteTooLarge`] if the cropped size doesn't fit in a `u16` or the
/// anchor offset doesn't fit in an `i16`.
#[allow(clippy::indexing_slicing, clippy::cast_possible_truncation)]
fn extract_sprite(
    page: &[u8],
    page_w: u32,
    page_h: u32,
    texture: &Texture<'_>,
    dst: &mut [u8],
) -> Result<Option<ExtractedSprite>> {
    // The pack stores these as `i32`. A negative value means a malformed entry. It becomes 0
    // here, and the check below rejects a zero size.
    let sx = texture.x.max(0).cast_unsigned();
    let sy = texture.y.max(0).cast_unsigned();
    let sw = texture.width.max(0).cast_unsigned();
    let sh = texture.height.max(0).cast_unsigned();

    // These values come from the pack file. With plain `+`, a malicious pack could pick values
    // whose sum overflows, pass the check, and make the reads below run off the page.
    let fits = sx
        .checked_add(sw)
        .zip(sy.checked_add(sh))
        .is_some_and(|(right, bottom)| right <= page_w && bottom <= page_h);

    if sw == 0 || sh == 0 || !fits {
        warn!(
            name = texture.name,
            sx, sy, sw, sh, page_w, page_h, "texture rect falls outside its page"
        );
        return Ok(None);
    }

    let stride = page_w as usize * 4;
    let px = |x: u32, y: u32| -> usize { (sy + y) as usize * stride + (sx + x) as usize * 4 };

    // Find the smallest box holding every visible pixel.
    let (mut x0, mut y0, mut x1, mut y1) = (sw, sh, 0_u32, 0_u32);
    for y in 0..sh {
        let row = px(0, y);
        for x in 0..sw {
            if page[row + x as usize * 4 + 3] != 0 {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }

    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }

    let (tw, th) = (x1 - x0, y1 - y0);
    let mut opaque = true;
    let mut out = 0_usize;

    for y in y0..y1 {
        let row = px(x0, y);
        for x in 0..tw {
            let s = row + x as usize * 4;
            let a = page[s + 3];
            opaque &= a == 255;

            // Computes `c * a / 255`, rounded to nearest, without a division.
            let mul = |c: u8| -> u8 {
                let t = c as u32 * a as u32 + 128;
                (((t + (t >> 8)) >> 8) & 0xff) as u8
            };

            dst[out] = mul(page[s]);
            dst[out + 1] = mul(page[s + 1]);
            dst[out + 2] = mul(page[s + 2]);
            dst[out + 3] = a;
            out += 4;
        }
    }

    // The pack's anchor offset, plus what the new crop removed from the top and left.
    let (ax, ay) = anchor(texture);
    let ox = ax + x0.cast_signed();
    let oy = ay + y0.cast_signed();

    // A very large page could give values that don't fit in `u16` or `i16`, so each is checked
    // instead of cast.
    let too_large = || BuildError::SpriteTooLarge {
        name: texture.name.to_owned(),
        width: tw,
        height: th,
        offset_x: ox,
        offset_y: oy,
    };

    Ok(Some(ExtractedSprite {
        width: u16::try_from(tw).map_err(|_| too_large())?,
        height: u16::try_from(th).map_err(|_| too_large())?,
        offset_x: i16::try_from(ox).map_err(|_| too_large())?,
        offset_y: i16::try_from(oy).map_err(|_| too_large())?,
        opaque,
        written: out,
    }))
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use crate::budget::BuildBudget;

    /// Bundles the limits and budget for a test.
    fn ctx<'a>(limits: &'a BuildLimits, budget: &'a BuildBudget) -> BuildCtx<'a> {
        BuildCtx::new(limits, budget)
    }
    use crate::{
        string_table::StringTableBuilder,
        testing::{
            PageSpec,
            oversized_png,
            solid_png,
            texture_pack,
        },
    };

    /// A pack whose single page lies about its size in the PNG header.
    #[test]
    fn a_page_over_the_cap_is_rejected_before_it_allocates() {
        let budget = BuildBudget::unlimited();
        let limits = BuildLimits::default();
        let png = oversized_png(65535, 65535);

        // The file is tiny, but decoding it without limits would allocate about 17 GB.
        assert!(png.len() < 128);

        let err = decode_page(&png, "hostile", ctx(&limits, &budget))
            .expect_err("a 65535 square page is over every default limit");

        match err {
            BuildError::PageTooLarge {
                page,
                width,
                height,
                max,
                ..
            } => {
                assert_eq!(page, "hostile");
                assert_eq!((width, height), (65535, 65535));
                assert_eq!(max, u64::from(limits.max_page_dimension));
            }
            other => panic!("expected PageTooLarge, got {other:?}"),
        }
    }

    /// A page exactly at every limit must be accepted.
    #[test]
    fn a_page_at_the_cap_still_decodes() {
        let budget = BuildBudget::unlimited();
        let limits = BuildLimits {
            max_page_dimension: 64,
            max_page_pixels: 64 * 64,
            max_page_bytes: 64 * 64 * 4,
            ..BuildLimits::default()
        };

        let png = solid_png(64, 64, [7, 8, 9, 255]);
        let (pixels, w, h) =
            decode_page(&png, "at_the_cap", ctx(&limits, &budget)).expect("64x64 is allowed");

        assert_eq!((w, h), (64, 64));
        assert_eq!(pixels.len(), 64 * 64 * 4);
    }

    /// A page inside the dimension cap can still be over the pixel cap if it is a long strip.
    #[test]
    fn the_pixel_cap_catches_a_strip_the_dimension_cap_allows() {
        let budget = BuildBudget::unlimited();
        let limits = BuildLimits {
            max_page_dimension: 4096,
            max_page_pixels: 1024,
            max_page_bytes: 4096 * 4096 * 4,
            ..BuildLimits::default()
        };

        let png = solid_png(64, 64, [0, 0, 0, 255]);
        let err = decode_page(&png, "strip", ctx(&limits, &budget))
            .expect_err("4096 pixels is over the cap");

        assert!(
            matches!(
                err,
                BuildError::PageTooLarge {
                    pixels: 4096,
                    max: 1024,
                    ..
                }
            ),
            "unexpected error: {err:?}"
        );
    }

    /// The sprite size limit must be checked before the pixel buffer is allocated.
    #[test]
    fn a_blob_over_the_budget_is_rejected_while_sizing() {
        let budget = BuildBudget::unlimited();
        let limits = BuildLimits {
            max_blob_bytes: 1024,
            ..BuildLimits::default()
        };

        // Two 64x64 sprites reserve 64*64*4 = 16384 bytes each, well past the 1 KiB budget.
        let png = solid_png(64, 64, [1, 1, 1, 255]);
        let pack_bytes = texture_pack(&[PageSpec::single_sprite("page", "sprite_a", 64, 64, png)]);

        let parsed = vec![TexturePack::parse(&pack_bytes).expect("the fixture pack parses")];

        let err = extract_selected(
            &parsed,
            |name| (name == "sprite_a").then_some(0),
            1,
            ctx(&limits, &budget),
        )
        .expect_err("16 KiB of sprites do not fit a 1 KiB budget");

        match err {
            BuildError::TextureBudget { bytes, max } => {
                assert_eq!(max, 1024);
                assert!(bytes > 1024, "reported {bytes} bytes");
            }
            other => panic!("expected TextureBudget, got {other:?}"),
        }
    }

    /// A pack that would need more memory than the machine has must fail without allocating
    /// it.
    #[test]
    fn a_hostile_pack_fails_the_build_without_a_huge_allocation() {
        let budget = BuildBudget::unlimited();
        let limits = BuildLimits::default();
        let pack_bytes = texture_pack(&[PageSpec::single_sprite(
            "huge",
            "sprite_a",
            64,
            64,
            oversized_png(65535, 65535),
        )]);

        let parsed =
            vec![TexturePack::parse(&pack_bytes).expect("the pack envelope is well-formed")];

        let tiles = StringTableBuilder::<TileNames>::new();
        tiles.intern("sprite_a").expect("interning a name");
        let reader = tiles.into_reader();

        let err = extract_selected(
            &parsed,
            |name| reader.resolve_string(name).map(|k| k.index()),
            reader.len(),
            ctx(&limits, &budget),
        )
        .expect_err("the page is over the dimension cap");

        assert!(
            matches!(err, BuildError::PageTooLarge { .. }),
            "unexpected error: {err:?}"
        );
    }
}

/// Tests the memory budget through the texture build.
#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::{
        budget::BuildBudget,
        string_table::{
            StringTableBuilder,
            TileNames,
        },
        testing::{
            PageSpec,
            solid_png,
            texture_pack,
        },
    };

    fn one_sprite_pack(px: u32) -> Vec<TexturePack<'static>> {
        // Leaked so the parsed pack can borrow it for the whole test.
        let bytes: &'static [u8] = Box::leak(
            texture_pack(&[PageSpec::single_sprite(
                "page",
                "sprite_a",
                i32::try_from(px).unwrap_or(0),
                i32::try_from(px).unwrap_or(0),
                solid_png(px, px, [1, 2, 3, 255]),
            )])
            .into_boxed_slice(),
        );

        vec![TexturePack::parse(bytes).expect("the fixture pack parses")]
    }

    fn resolver() -> impl Fn(&str) -> Option<usize> {
        |name| (name == "sprite_a").then_some(0)
    }

    #[test]
    fn extracting_charges_the_blob() {
        let parsed = one_sprite_pack(32);
        let limits = BuildLimits::default();
        let budget = BuildBudget::new(1 << 30);

        extract_selected(&parsed, resolver(), 1, BuildCtx::new(&limits, &budget))
            .expect("a 32x32 sprite fits a 1 GiB budget");

        // 32 * 32 * 4 = 4096 bytes of pixels, still charged because the buffer is kept.
        assert_eq!(budget.charged(), 4096);
    }

    /// The decoded page must be released when the page is done, but the pixel buffer must not.
    #[test]
    fn the_page_scratch_is_released_but_the_blob_is_not() {
        let parsed = one_sprite_pack(32);
        let limits = BuildLimits::default();
        let budget = BuildBudget::new(1 << 30);

        extract_selected(&parsed, resolver(), 1, BuildCtx::new(&limits, &budget))
            .expect("extraction succeeds");

        // The decoded page was also 4096 bytes. If it were still charged the total would be 8192.
        assert_eq!(budget.charged(), 4096, "the page scratch leaked");
        assert_eq!(budget.high_water(), 8192, "the peak should include both");
    }

    #[test]
    fn a_build_over_its_budget_fails_closed() {
        let parsed = one_sprite_pack(32);
        let limits = BuildLimits::default();
        let budget = BuildBudget::new(1024);

        let err = extract_selected(&parsed, resolver(), 1, BuildCtx::new(&limits, &budget))
            .expect_err("4096 bytes do not fit a 1024 byte budget");

        match err {
            BuildError::OutOfBudget { what, ceiling, .. } => {
                assert_eq!((what, ceiling), ("texture blob", 1024));
            }
            other => panic!("expected OutOfBudget, got {other:?}"),
        }
    }

    /// A rejected page must not stay charged to the budget.
    #[test]
    fn a_rejected_page_does_not_leak_budget() {
        let limits = BuildLimits {
            max_page_pixels: 16,
            ..BuildLimits::default()
        };
        let budget = BuildBudget::new(1 << 30);
        let png = solid_png(32, 32, [0, 0, 0, 255]);

        decode_page(&png, "too_big", BuildCtx::new(&limits, &budget))
            .expect_err("1024 pixels is over the 16 pixel cap");

        assert_eq!(budget.charged(), 0, "a rejected page left budget charged");
    }

    #[test]
    fn a_full_texture_library_build_stays_within_budget() {
        let parsed_bytes = texture_pack(&[PageSpec::single_sprite(
            "page",
            "sprite_a",
            16,
            16,
            solid_png(16, 16, [4, 5, 6, 255]),
        )]);

        let names = StringTableBuilder::<TileNames>::new();
        names.intern("sprite_a").expect("interning");
        let reader = names.into_reader();

        let pack = LoadedPackBytes(parsed_bytes);
        let limits = BuildLimits::default();
        let budget = BuildBudget::new(1 << 20);

        let parsed = vec![TexturePack::parse(pack.0.as_slice()).expect("parses")];

        let extraction = extract_selected(
            &parsed,
            |name| reader.resolve_string(name).map(|k| k.index()),
            reader.len(),
            BuildCtx::new(&limits, &budget),
        )
        .expect("a 16x16 sprite fits a 1 MiB budget");

        assert_eq!(extraction.blob.len(), 16 * 16 * 4);
        assert!(budget.high_water() <= 1 << 20);
    }

    /// Owns the pack bytes for the duration of a test.
    struct LoadedPackBytes(Vec<u8>);
}
