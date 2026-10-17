//! The built-in encoders and tile store.
//!
//! [`PngEncoder`] writes PNG tiles. `WebpEncoder`, behind the `webp` feature, writes WebP tiles,
//! lossless or lossy. [`BuiltinEncoder`] holds either one, for when the format is chosen at run
//! time.
//!
//! [`DziStore`] writes tiles as files in the Deep Zoom layout and reads them back. It is both
//! the [`TileSink`] and the [`TileSource`] for an ordinary render to disk.
//!
//! The canvas holds premultiplied alpha, but PNG and WebP store straight alpha. The encoders
//! convert a copy of each tile on the way out, and [`decode_tile`] converts back on the way in.

#[cfg(feature = "webp")]
use crate::render::EncodedAs;
use crate::render::{
    EncodeReport,
    TileEncoder,
    TileSink,
    TileSource,
    blend::{
        BPP,
        Canvas,
        premultiply,
        unpremultiply,
    },
    pyramid::{
        Pyramid,
        TileAddress,
    },
};
use std::{
    fs,
    io::{
        Cursor,
        Write as _,
    },
    path::{
        Path,
        PathBuf,
    },
};
use thiserror::Error;

/// Runs `f` on a straight-alpha copy of `tile`.
///
/// The copy goes in a buffer that each thread reuses from call to call. `tile` itself is left
/// premultiplied, because the caller may still need it, for example to build its parent tile.
fn with_straight_alpha<R>(tile: &Canvas, f: impl FnOnce(&[u8]) -> R) -> R {
    use std::cell::RefCell;

    thread_local! {
        static STRAIGHT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    STRAIGHT.with(|cell| {
        let mut straight = cell.borrow_mut();
        straight.clear();
        straight.extend_from_slice(tile.pixels());
        unpremultiply(&mut straight);
        f(&straight)
    })
}

/// Encodes tiles as 8-bit RGBA PNG files.
///
/// Works with any build of the crate. The `webp` feature's `WebpEncoder` usually gives smaller
/// files.
#[derive(Clone, Copy, Debug, Default)]
pub struct PngEncoder {
    compression: PngCompression,
}

/// How hard the PNG encoder works to make files small.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PngCompression {
    /// Fastest to encode, with larger files.
    #[default]
    Fast,
    /// The `image` crate's default compression level. Slower, with smaller files.
    Balanced,
}

impl PngEncoder {
    /// A PNG encoder with the given compression level.
    pub const fn new(compression: PngCompression) -> Self {
        PngEncoder { compression }
    }

    /// The compression level.
    pub const fn compression(&self) -> PngCompression {
        self.compression
    }
}

/// An error from [`PngEncoder`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PngError {
    /// The `image` crate failed to encode the tile.
    #[error("encoding PNG")]
    Encode(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl TileEncoder for PngEncoder {
    type Error = PngError;

    fn format(&self) -> &str {
        "png"
    }

    fn encode(&self, tile: &Canvas, out: &mut Vec<u8>) -> Result<EncodeReport, Self::Error> {
        let compression = match self.compression {
            PngCompression::Fast => image::codecs::png::CompressionType::Fast,
            PngCompression::Balanced => image::codecs::png::CompressionType::Default,
        };

        with_straight_alpha(tile, |straight| {
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                Cursor::new(out),
                compression,
                image::codecs::png::FilterType::Adaptive,
            );

            image::ImageEncoder::write_image(
                encoder,
                straight,
                tile.width(),
                tile.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|err| PngError::Encode(Box::new(err)))?;

            Ok(EncodeReport::single())
        })
    }
}

/// Encodes tiles as WebP files, lossless or lossy, using `libwebp`.
///
/// This is the default encoder. On map tiles it is faster than PNG and gives smaller files.
/// Lossless tiles read back through [`decode_tile`] exactly as they were drawn.
///
/// Start from [`WebpEncoder::LOSSLESS`], [`WebpEncoder::lossy`] or [`WebpEncoder::adaptive`].
/// Then adjust it with the `with_*` methods, which clamp each setting to its valid range.
///
/// # Examples
///
/// ```
/// use pzmap2img::render::WebpEncoder;
///
/// // Try both modes on every tile, and keep lossy only when it is clearly smaller.
/// let encoder = WebpEncoder::adaptive(90.0).with_method(4);
/// assert_eq!(encoder.method(), 4);
/// ```
#[cfg(feature = "webp")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebpEncoder {
    mode: WebpMode,
    method: u8,
    lossless_quality: u8,
    alpha_quality: u8,
    sharp_yuv: bool,
    lossy_margin: u32,
    sns_strength: u8,
    filter_strength: u8,
    dither: bool,
}

#[cfg(feature = "webp")]
impl Default for WebpEncoder {
    fn default() -> Self {
        WebpEncoder::LOSSLESS
    }
}

/// Whether WebP output is exact, approximate, or chosen per tile.
#[cfg(feature = "webp")]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WebpMode {
    /// Exact. This is the default.
    #[default]
    Lossless,

    /// Approximate, at a `quality` from 0.0 to 100.0. Lower quality gives smaller files with less
    /// detail.
    Lossy { quality: f32 },

    /// Encodes each tile both ways and keeps lossy only if it is smaller by at least
    /// [`WebpEncoder::lossy_margin`]. This can take up to twice as long as either mode alone.
    Adaptive { quality: f32 },
}

#[cfg(feature = "webp")]
impl WebpEncoder {
    /// How many bytes per megapixel lossy must save before [`WebpMode::Adaptive`] picks it over
    /// lossless.
    ///
    /// Chosen from blind pairwise judging of lossless against lossy output on sampled map tiles.
    pub const DEFAULT_LOSSY_MARGIN_PER_MEGAPIXEL: u32 = 143 * 1024;

    /// [`WebpMode::Adaptive`] skips the lossy pass when the lossless file is smaller than this,
    /// in bytes per megapixel.
    ///
    /// Tiles that compress this well are mostly flat or repeating, and lossy does not save
    /// enough on them to be worth the time.
    pub const LOSSY_WORTH_TRYING_BYTES_PER_MEGAPIXEL: u32 = 285_000;

    /// The default spatial noise shaping strength, the same as libwebp's. See
    /// [`Self::with_tone`].
    pub const DEFAULT_SNS_STRENGTH: u8 = 50;
    /// The default deblocking filter strength, the same as libwebp's. See [`Self::with_tone`].
    pub const DEFAULT_FILTER_STRENGTH: u8 = 60;

    /// Lossless encoding with `method` 0, the fastest setting. The other constructors start from
    /// this value.
    pub const LOSSLESS: WebpEncoder = WebpEncoder {
        mode: WebpMode::Lossless,
        method: 0,
        lossless_quality: 100,
        alpha_quality: 100,
        // Lossless encoding ignores this. It is on because the other constructors copy this
        // value, and the lossy modes want it.
        sharp_yuv: true,
        sns_strength: WebpEncoder::DEFAULT_SNS_STRENGTH,
        filter_strength: WebpEncoder::DEFAULT_FILTER_STRENGTH,
        dither: false,
        lossy_margin: WebpEncoder::DEFAULT_LOSSY_MARGIN_PER_MEGAPIXEL,
    };

    /// Lossy encoding at `quality`, clamped to 0.0 to 100.0.
    pub fn lossy(quality: f32) -> Self {
        WebpEncoder {
            mode: WebpMode::Lossy {
                quality: quality.clamp(0.0, 100.0),
            },
            ..WebpEncoder::LOSSLESS
        }
    }

    /// Chooses lossless or lossy per tile, with lossy at `quality`. See [`WebpMode::Adaptive`].
    pub fn adaptive(quality: f32) -> Self {
        WebpEncoder {
            mode: WebpMode::Adaptive {
                quality: quality.clamp(0.0, 100.0),
            },
            ..WebpEncoder::LOSSLESS
        }
    }

    /// Sets libwebp's `method`, clamped to 0 to 6. 0 is fastest. Higher values take longer and
    /// give smaller files.
    #[must_use]
    pub fn with_method(mut self, method: u8) -> Self {
        self.method = method.min(6);
        self
    }

    /// Sets how hard lossless encoding searches for repeated patterns, clamped to 0 to 100.
    /// Higher values take longer and give smaller files. The output is exact at any setting.
    #[must_use]
    pub fn with_lossless_quality(mut self, lossless_quality: u8) -> Self {
        self.lossless_quality = lossless_quality.min(100);
        self
    }

    /// Sets the quality of the alpha channel in lossy tiles, clamped to 0 to 100. At 100,
    /// transparency is kept exact. Lower values give smaller files with less exact edges.
    #[must_use]
    pub fn with_alpha_quality(mut self, alpha_quality: u8) -> Self {
        self.alpha_quality = alpha_quality.min(100);
        self
    }

    /// Turns libwebp's sharp YUV conversion on or off for lossy tiles. It is slower, but keeps
    /// colors more accurate along sharp edges, such as between a sprite and transparency.
    #[must_use]
    pub fn with_sharp_yuv(mut self, sharp_yuv: bool) -> Self {
        self.sharp_yuv = sharp_yuv;
        self
    }

    /// Sets how many bytes per megapixel lossy must save before [`WebpMode::Adaptive`] picks
    /// it. At 0, adaptive keeps whichever file is smaller. See
    /// [`WebpEncoder::DEFAULT_LOSSY_MARGIN_PER_MEGAPIXEL`].
    #[must_use]
    pub const fn with_lossy_margin(mut self, bytes_per_megapixel: u32) -> Self {
        self.lossy_margin = bytes_per_megapixel;
        self
    }

    /// Sets the spatial noise shaping and deblocking filter strengths for lossy tiles, each
    /// clamped to 0 to 100.
    ///
    /// Stronger noise shaping gives smaller files but loses fine detail. Stronger deblocking
    /// smooths the edges between compression blocks but blurs sharp detail.
    #[must_use]
    pub fn with_tone(mut self, sns_strength: u8, filter_strength: u8) -> Self {
        self.sns_strength = sns_strength.min(100);
        self.filter_strength = filter_strength.min(100);
        self
    }

    /// Turns dithering on or off for lossy tiles. Dithering adds slight noise, which makes
    /// gradients look smoother.
    #[must_use]
    pub fn with_dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }

    /// The same settings, but with lossless encoding.
    #[must_use]
    pub fn to_lossless(mut self) -> Self {
        self.mode = WebpMode::Lossless;
        self
    }

    /// The encoding mode.
    pub const fn mode(&self) -> WebpMode {
        self.mode
    }

    /// libwebp's `method`. See [`Self::with_method`].
    pub const fn method(&self) -> u8 {
        self.method
    }

    /// The lossless search effort. See [`Self::with_lossless_quality`].
    pub const fn lossless_quality(&self) -> u8 {
        self.lossless_quality
    }

    /// The alpha quality for lossy tiles. See [`Self::with_alpha_quality`].
    pub const fn alpha_quality(&self) -> u8 {
        self.alpha_quality
    }

    /// Whether sharp YUV conversion is on. See [`Self::with_sharp_yuv`].
    pub const fn sharp_yuv(&self) -> bool {
        self.sharp_yuv
    }

    /// How many bytes per megapixel lossy must save before [`WebpMode::Adaptive`] picks it.
    pub const fn lossy_margin(&self) -> u32 {
        self.lossy_margin
    }

    /// The spatial noise shaping strength. See [`Self::with_tone`].
    pub const fn sns_strength(&self) -> u8 {
        self.sns_strength
    }

    /// The deblocking filter strength. See [`Self::with_tone`].
    pub const fn filter_strength(&self) -> u8 {
        self.filter_strength
    }

    /// Whether dithering is on. See [`Self::with_dither`].
    pub const fn dither(&self) -> bool {
        self.dither
    }
}

/// An error from [`WebpEncoder`].
#[cfg(feature = "webp")]
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WebpError {
    /// The tile is too large. WebP images can be at most 16383 pixels on each side.
    #[error("a {width}x{height} tile is outside WebP's {max}-pixel limit")]
    TooLarge { width: u32, height: u32, max: u32 },

    /// libwebp rejected its own default settings. This means the linked libwebp does not match
    /// the version the bindings were built for.
    #[error("libwebp rejected its own default configuration; the linked library is mismatched")]
    Config,

    /// libwebp could not copy the pixels in. The size was already checked, so this means it ran
    /// out of memory.
    #[error("libwebp could not import a {width}x{height} tile into a picture")]
    Import { width: u32, height: u32 },

    /// libwebp failed to encode the tile.
    #[error("libwebp failed to encode the tile: {reason}")]
    Encode { reason: WebpEncodeFailure },
}

/// Why libwebp failed to encode a tile, as reported by libwebp.
#[cfg(feature = "webp")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WebpEncodeFailure {
    #[error("out of memory")]
    OutOfMemory,
    #[error("out of memory while flushing the bitstream")]
    BitstreamOutOfMemory,
    #[error("a required pointer was null")]
    NullParameter,
    #[error("invalid configuration")]
    InvalidConfiguration,
    #[error("bad picture dimensions")]
    BadDimension,
    #[error("the first partition exceeded 512 KiB")]
    Partition0Overflow,
    #[error("a partition exceeded 16 MiB")]
    PartitionOverflow,
    #[error("writing the output failed")]
    BadWrite,
    #[error("the output exceeded 4 GiB")]
    FileTooBig,
    #[error("aborted by the progress hook")]
    UserAbort,
    /// The encode failed without setting an error code.
    #[error("no error code was reported")]
    Unknown,
}

#[cfg(feature = "webp")]
impl WebpEncodeFailure {
    fn from_code(code: libwebp_sys::WebPEncodingError) -> Self {
        use libwebp_sys::WebPEncodingError as E;
        match code {
            E::VP8_ENC_ERROR_OUT_OF_MEMORY => Self::OutOfMemory,
            E::VP8_ENC_ERROR_BITSTREAM_OUT_OF_MEMORY => Self::BitstreamOutOfMemory,
            E::VP8_ENC_ERROR_NULL_PARAMETER => Self::NullParameter,
            E::VP8_ENC_ERROR_INVALID_CONFIGURATION => Self::InvalidConfiguration,
            E::VP8_ENC_ERROR_BAD_DIMENSION => Self::BadDimension,
            E::VP8_ENC_ERROR_PARTITION0_OVERFLOW => Self::Partition0Overflow,
            E::VP8_ENC_ERROR_PARTITION_OVERFLOW => Self::PartitionOverflow,
            E::VP8_ENC_ERROR_BAD_WRITE => Self::BadWrite,
            E::VP8_ENC_ERROR_FILE_TOO_BIG => Self::FileTooBig,
            E::VP8_ENC_ERROR_USER_ABORT => Self::UserAbort,
            E::VP8_ENC_OK | E::VP8_ENC_ERROR_LAST => Self::Unknown,
        }
    }
}

#[cfg(feature = "webp")]
impl TileEncoder for WebpEncoder {
    type Error = WebpError;

    fn format(&self) -> &str {
        "webp"
    }

    fn encode(&self, tile: &Canvas, out: &mut Vec<u8>) -> Result<EncodeReport, Self::Error> {
        if tile.width() > WEBP_MAX_SIDE || tile.height() > WEBP_MAX_SIDE {
            return Err(WebpError::TooLarge {
                width: tile.width(),
                height: tile.height(),
                max: WEBP_MAX_SIDE,
            });
        }

        // WebP stores straight alpha, like PNG.
        with_straight_alpha(tile, |straight| {
            self.encode_with(
                WebpInput {
                    rgba: straight,
                    width: tile.width(),
                    height: tile.height(),
                },
                out,
            )
        })
    }
}

#[cfg(feature = "webp")]
impl WebpEncoder {
    /// Encodes one tile in the encoder's [`WebpMode`], running one or two passes.
    fn encode_with(
        &self,
        input: WebpInput<'_>,
        out: &mut Vec<u8>,
    ) -> Result<EncodeReport, WebpError> {
        match self.mode {
            WebpMode::Lossless => {
                let encoded = self.run(input, None)?;
                out.extend_from_slice(&encoded);
                Ok(EncodeReport::new(EncodedAs::Lossless, 1))
            }
            WebpMode::Lossy { quality } => {
                let encoded = self.run(input, Some(quality))?;
                out.extend_from_slice(&encoded);
                Ok(EncodeReport::new(EncodedAs::Lossy, 1))
            }
            WebpMode::Adaptive { quality } => {
                // Lossless runs first, because its size decides whether lossy is worth trying.
                let lossless = self.run(input, None)?;

                let megapixels = f64::from(input.width) * f64::from(input.height) / 1_000_000.0;
                let margin = f64::from(self.lossy_margin) * megapixels;
                // Lossy cannot save more bytes than the lossless file has. If that file is
                // already smaller than the margin, lossy can never win.
                if (lossless.len() as f64) <= margin {
                    out.extend_from_slice(&lossless);
                    return Ok(EncodeReport::new(EncodedAs::Lossless, 1));
                }

                let worth_trying =
                    f64::from(WebpEncoder::LOSSY_WORTH_TRYING_BYTES_PER_MEGAPIXEL) * megapixels;
                if (lossless.len() as f64) < worth_trying {
                    out.extend_from_slice(&lossless);
                    return Ok(EncodeReport::new(EncodedAs::Lossless, 1));
                }

                let lossy = self.run(input, Some(quality))?;
                // Lossy has to win by the margin, not by a single byte. Lossless is exact, while
                // lossy blurs color along every edge between a sprite and transparency.
                let saved = lossless.len().saturating_sub(lossy.len()) as f64;
                if saved > margin {
                    out.extend_from_slice(&lossy);
                    Ok(EncodeReport::new(EncodedAs::Lossy, 2))
                } else {
                    out.extend_from_slice(&lossless);
                    Ok(EncodeReport::new(EncodedAs::Lossless, 2))
                }
            }
        }
    }
}

/// One of the built-in encoders, chosen at run time.
///
/// [`render_pyramid`](crate::render::render_pyramid) accepts any [`TileEncoder`]. If the format
/// is known when writing the code, pass [`PngEncoder`] or `WebpEncoder` directly. Use
/// `BuiltinEncoder` when the format comes from a command-line flag or a config file.
#[derive(Clone, Copy, Debug)]
pub enum BuiltinEncoder {
    /// PNG output.
    Png(PngEncoder),
    /// WebP output.
    #[cfg(feature = "webp")]
    Webp(WebpEncoder),
}

/// An error from [`BuiltinEncoder`], wrapping the error of the encoder it holds.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BuiltinEncoderError {
    /// The PNG encoder failed.
    #[error(transparent)]
    Png(#[from] PngError),

    /// The WebP encoder failed.
    #[cfg(feature = "webp")]
    #[error(transparent)]
    Webp(#[from] WebpError),
}

impl TileEncoder for BuiltinEncoder {
    type Error = BuiltinEncoderError;

    fn format(&self) -> &str {
        match self {
            BuiltinEncoder::Png(encoder) => encoder.format(),
            #[cfg(feature = "webp")]
            BuiltinEncoder::Webp(encoder) => encoder.format(),
        }
    }

    fn encode(&self, tile: &Canvas, out: &mut Vec<u8>) -> Result<EncodeReport, Self::Error> {
        match self {
            BuiltinEncoder::Png(encoder) => Ok(encoder.encode(tile, out)?),
            #[cfg(feature = "webp")]
            BuiltinEncoder::Webp(encoder) => Ok(encoder.encode(tile, out)?),
        }
    }
}

/// The largest width or height libwebp can encode.
#[cfg(feature = "webp")]
const WEBP_MAX_SIDE: u32 = 16383;

/// The input to [`WebpEncoder::run`]: one tile's straight-alpha RGBA pixels and size.
#[cfg(feature = "webp")]
#[derive(Clone, Copy)]
struct WebpInput<'a> {
    rgba: &'a [u8],
    width: u32,
    height: u32,
}

/// An encoded WebP file in a buffer that libwebp allocated. Dropping it frees the buffer.
///
/// libwebp allocated the buffer, so it must be freed with `WebPFree`, not by Rust's allocator.
#[cfg(feature = "webp")]
struct WebpOutput {
    mem: *mut u8,
    size: usize,
}

#[cfg(feature = "webp")]
impl std::ops::Deref for WebpOutput {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: `mem` was checked non-null when this value was built, `size` is the byte count
        // libwebp's writer reported for it, and nothing mutates the buffer for the lifetime of
        // this `WebpOutput`.
        unsafe { std::slice::from_raw_parts(self.mem, self.size) }
    }
}

#[cfg(feature = "webp")]
impl Drop for WebpOutput {
    fn drop(&mut self) {
        // SAFETY: `mem` was allocated by libwebp through a `WebPMemoryWriter` and has not been
        // freed yet. That is what `WebPFree` requires.
        unsafe { libwebp_sys::WebPFree(self.mem.cast()) };
    }
}

#[cfg(feature = "webp")]
impl WebpEncoder {
    /// Encodes `input` once with libwebp. A `quality` of `None` means lossless.
    ///
    /// This calls `libwebp-sys` directly rather than going through the safe `webp` crate. That
    /// crate depends on an older libwebp with slower lossless encoding.
    fn run(&self, input: WebpInput<'_>, quality: Option<f32>) -> Result<WebpOutput, WebpError> {
        use libwebp_sys as ffi;

        // The ABI version is a small positive number. If it does not fit in an `i32`, the
        // bindings are broken, which is what `Config` reports.
        let abi = i32::try_from(ffi::WEBP_ENCODER_ABI_VERSION).map_err(|_| WebpError::Config)?;

        let mut config = {
            let mut uninit = std::mem::MaybeUninit::<ffi::WebPConfig>::uninit();
            // SAFETY: `uninit.as_mut_ptr()` points to valid, properly aligned space for a
            // `WebPConfig`, which is all this call requires before it writes to it.
            let ok = unsafe {
                ffi::WebPConfigInitInternal(
                    uninit.as_mut_ptr(),
                    ffi::WebPPreset::WEBP_PRESET_DEFAULT,
                    75.0,
                    abi,
                )
            };
            if ok == 0 {
                return Err(WebpError::Config);
            }
            // SAFETY: a non-zero return from `WebPConfigInitInternal` means every field of
            // `uninit` was written.
            unsafe { uninit.assume_init() }
        };
        config.method = i32::from(self.method);

        // `thread_level` stays at libwebp's default of 0. The render already keeps every core
        // busy with its own tile, so extra threads per tile would only compete with them.

        match quality {
            None => {
                config.lossless = 1;
                // For lossless, `quality` means search effort. The output is exact either way.
                config.quality = f32::from(self.lossless_quality);
                // Lossless already stores alpha exactly. Compressing it separately would only
                // cost time.
                config.alpha_compression = 0;
            }
            Some(quality) => {
                config.lossless = 0;
                config.quality = quality;
                config.alpha_compression = 1;
                config.alpha_quality = i32::from(self.alpha_quality);
                config.use_sharp_yuv = i32::from(self.sharp_yuv);
                config.sns_strength = i32::from(self.sns_strength);
                config.filter_strength = i32::from(self.filter_strength);
                // `preprocessing` is a set of flags: 1 is segment smoothing and 2 is dithering.
                // Only dithering is offered, because smoothing blurs the fine detail in map
                // sprites.
                config.preprocessing = i32::from(self.dither) * 2;
            }
        }

        let mut picture = {
            let mut uninit = std::mem::MaybeUninit::<ffi::WebPPicture>::uninit();
            // SAFETY: `uninit.as_mut_ptr()` points to valid, properly aligned space for a
            // `WebPPicture`, which is all this call requires before it writes to it.
            let ok = unsafe { ffi::WebPPictureInitInternal(uninit.as_mut_ptr(), abi) };
            if ok == 0 {
                return Err(WebpError::Config);
            }
            // SAFETY: a non-zero return from `WebPPictureInitInternal` means every field of
            // `uninit` was written.
            unsafe { uninit.assume_init() }
        };

        // The caller already limits both sides to `WEBP_MAX_SIDE`, so these conversions cannot
        // fail. They are checked anyway in case another caller does not.
        let too_large = || WebpError::TooLarge {
            width: input.width,
            height: input.height,
            max: WEBP_MAX_SIDE,
        };
        picture.use_argb = 1;
        picture.width = i32::try_from(input.width).map_err(|_| too_large())?;
        picture.height = i32::try_from(input.height).map_err(|_| too_large())?;
        let stride = (input.width as usize)
            .checked_mul(BPP)
            .and_then(|s| i32::try_from(s).ok())
            .ok_or_else(too_large)?;

        // The import reads `height` rows of `stride` bytes, so the buffer must be exactly that
        // size. A canvas always is, so a mismatch here is a bug.
        assert_eq!(
            input.rgba.len(),
            input.width as usize * input.height as usize * BPP,
            "WebP input buffer does not match its dimensions"
        );
        // SAFETY: `picture` was initialized above. `input.rgba` points to
        // `input.height as usize * stride as usize` readable bytes, as the assertion above
        // checks. `stride` matches `picture.width` and the RGBA pixel format that `use_argb`
        // implies.
        let imported =
            unsafe { ffi::WebPPictureImportRGBA(&raw mut picture, input.rgba.as_ptr(), stride) };
        if imported == 0 {
            // SAFETY: `picture` is initialized. Even after a failed import, it owns nothing that
            // `WebPPictureFree` cannot release.
            unsafe { ffi::WebPPictureFree(&raw mut picture) };
            return Err(WebpError::Import {
                width: input.width,
                height: input.height,
            });
        }

        let mut writer = {
            let mut uninit = std::mem::MaybeUninit::<ffi::WebPMemoryWriter>::uninit();
            // SAFETY: `uninit.as_mut_ptr()` points to valid, properly aligned space for a
            // `WebPMemoryWriter`, which `WebPMemoryWriterInit` always fully initializes.
            unsafe { ffi::WebPMemoryWriterInit(uninit.as_mut_ptr()) };
            // SAFETY: the call above already initialized every field.
            unsafe { uninit.assume_init() }
        };
        picture.writer = Some(ffi::WebPMemoryWrite);
        picture.custom_ptr = (&raw mut writer).cast();

        // SAFETY: `config` and `picture` are both initialized, and `picture.custom_ptr` points to
        // `writer`, which outlives this call.
        let ok = unsafe { ffi::WebPEncode(&raw const config, &raw mut picture) };
        let code = picture.error_code;
        // SAFETY: `picture` is initialized. It owns the imported copy of the pixels. The
        // writer's buffer is separate and is not freed here.
        unsafe { ffi::WebPPictureFree(&raw mut picture) };

        if ok == 0 || writer.mem.is_null() {
            // SAFETY: `writer` is initialized. This frees whatever it wrote before the failure.
            unsafe { ffi::WebPMemoryWriterClear(&raw mut writer) };
            return Err(WebpError::Encode {
                reason: WebpEncodeFailure::from_code(code),
            });
        }

        Ok(WebpOutput {
            mem: writer.mem,
            size: writer.size,
        })
    }
}

/// Stores tiles as files in the Deep Zoom layout, `root/{level}/{col}_{row}.{extension}`.
///
/// It is both a [`TileSink`] and a [`TileSource`], so the render can read tiles back to build
/// the upper levels. After the render, call [`Self::write_descriptor`] to write the `.dzi`
/// metadata file. This file is required to read the directory tree as a DZI pyramid.
///
/// Each tile is written to a temporary file and then renamed into place. A crash can leave a
/// stray `.tmp` file, but never a half-written tile.
#[derive(Clone, Debug)]
pub struct DziStore {
    root: PathBuf,
    extension: String,
    /// The path of the `.dzi` descriptor, next to `root`.
    descriptor: PathBuf,
}

/// An error from [`DziStore`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DziStoreError {
    /// A file or directory operation failed. `op` names the operation, such as `"writing"`.
    #[error("{op} {}", path.display())]
    Io {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A stored tile could not be decoded.
    #[error("decoding {}", path.display())]
    Decode {
        path: PathBuf,
        #[source]
        source: DecodeError,
    },

    /// The disk is full, or a disk quota was reached.
    ///
    /// This is separate from [`Self::Io`] because it is the one failure that retrying on another
    /// disk can fix.
    #[error("{op} {} failed: out of space", path.display())]
    OutOfSpace {
        op: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Why encoded tile bytes could not be turned back into a [`Canvas`].
///
/// This has no file path, so that stores which do not use files can return it from
/// [`decode_tile`] too.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The `image` crate could not decode the bytes.
    #[error("not a decodable image")]
    Image(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// libwebp could not decode the bytes. libwebp gives no reason, so there is no detail.
    #[cfg(feature = "webp")]
    #[error("not a decodable WebP")]
    Webp,

    /// The decoder returned the wrong number of bytes for the image size.
    #[error("decoded to {len} bytes, not the {expected} a {width}x{height} tile needs")]
    Mismatched {
        width: u32,
        height: u32,
        len: usize,
        expected: usize,
    },
}

/// Wraps an IO error in the right [`DziStoreError`] variant.
///
/// A full disk (`ENOSPC`) has its own `ErrorKind::StorageFull`. A full quota (`EDQUOT`) does not,
/// so it is checked by its raw error number.
fn classify(op: &'static str, path: PathBuf, source: std::io::Error) -> DziStoreError {
    #[cfg(unix)]
    const EDQUOT: i32 = 122;

    #[cfg(unix)]
    let quota = source.raw_os_error() == Some(EDQUOT);
    #[cfg(not(unix))]
    let quota = false;

    if source.kind() == std::io::ErrorKind::StorageFull || quota {
        DziStoreError::OutOfSpace { op, path, source }
    } else {
        DziStoreError::Io { op, path, source }
    }
}

impl DziStore {
    /// A store with tiles under `root`, saved with the file extension `extension`.
    ///
    /// Name `root` `{name}_files`. The descriptor then goes next to it as `{name}.dzi`. In the
    /// Deep Zoom layout, the tile directory has the descriptor's name plus `_files`.
    /// `extension` should be the encoder's [`TileEncoder::format`].
    pub fn new(root: impl Into<PathBuf>, extension: impl Into<String>) -> Self {
        let root: PathBuf = root.into();
        let descriptor = descriptor_path(&root);
        DziStore {
            root,
            extension: extension.into(),
            descriptor,
        }
    }

    /// Writes one tile by writing a temporary file next to it and renaming it into place.
    ///
    /// A crash leaves the tile either missing or complete, never cut short. Errors stay plain
    /// `io::Error`s so the caller can check the kind and decide whether to retry.
    fn write_once(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        // Next to the tile, not in a temp directory, because a rename is only atomic within one
        // filesystem. The name is based on the tile's, so a leftover from a crash is overwritten
        // by the next attempt instead of piling up.
        let mut temp = path.to_path_buf();
        temp.as_mut_os_string().push(".tmp");

        if let Err(source) = fs::write(&temp, bytes) {
            // Don't leave a partial temporary file behind.
            drop(fs::remove_file(&temp));
            return Err(source);
        }

        fs::rename(&temp, path).inspect_err(|_| {
            drop(fs::remove_file(&temp));
        })
    }

    /// The tile directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file path of the tile at `address`.
    pub fn tile_path(&self, address: TileAddress) -> PathBuf {
        self.root.join(address.level.to_string()).join(format!(
            "{}_{}.{}",
            address.col, address.row, self.extension
        ))
    }

    /// Writes the `.dzi` descriptor for `pyramid` next to the tile directory.
    ///
    /// The descriptor gives the image size, the tile size, and this store's extension as the
    /// tile format. Unlike the tiles, it is flushed to disk before this returns. It is a single
    /// small file, and the pyramid can't be read without it.
    ///
    /// # Errors
    ///
    /// Returns [`DziStoreError`] if the file cannot be written.
    pub fn write_descriptor(&self, pyramid: &Pyramid) -> Result<(), DziStoreError> {
        if let Some(parent) = self.descriptor.parent() {
            fs::create_dir_all(parent)
                .map_err(|source| classify("creating", parent.to_path_buf(), source))?;
        }

        let body = pyramid.descriptor(&self.extension);
        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create(&self.descriptor)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()
        };

        write().map_err(|source| classify("writing", self.descriptor.clone(), source))
    }
}

impl TileSink for DziStore {
    type Error = DziStoreError;

    /// Writes one tile, all at once or not at all.
    ///
    /// This expects [`Self::prepare`] to have created the level directory. If the directory is
    /// missing, it is created and the write is tried once more. So a store that was never
    /// prepared still works.
    fn write_tile(&self, address: TileAddress, bytes: &[u8]) -> Result<(), Self::Error> {
        let path = self.tile_path(address);

        match Self::write_once(&path, bytes) {
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|source| classify("creating", parent.to_path_buf(), source))?;
                }

                Self::write_once(&path, bytes).map_err(|source| classify("writing", path, source))
            }
            Err(source) => Err(classify("writing", path, source)),
            Ok(()) => Ok(()),
        }
    }

    /// Creates the directory for each level up front, so tile writes don't have to.
    fn prepare(&self, levels: u32) -> Result<(), Self::Error> {
        for level in 0..levels {
            let dir = self.root.join(level.to_string());
            fs::create_dir_all(&dir).map_err(|source| classify("creating", dir, source))?;
        }

        Ok(())
    }
}

/// Decodes the bytes of a stored tile back into a premultiplied [`Canvas`].
///
/// `format` is the encoder's [`TileEncoder::format`], such as `"webp"` or `"png"`. A custom
/// [`TileSource`] can use this to read back tiles from the built-in encoders, the same way
/// [`DziStore`] does. With the `webp` feature, WebP is decoded with libwebp, which is faster
/// than the `image` crate's decoder.
///
/// # Errors
///
/// Returns [`DecodeError`] if the bytes are not a valid image in a supported format.
///
/// # Examples
///
/// ```
/// use pzmap2img::render::{PngEncoder, TileEncoder, blend::Canvas, decode_tile};
///
/// let tile = Canvas::new(64, 64);
/// let mut bytes = Vec::new();
/// PngEncoder::default().encode(&tile, &mut bytes)?;
///
/// let decoded = decode_tile(&bytes, "png")?;
/// assert_eq!(decoded.pixels(), tile.pixels());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn decode_tile(bytes: &[u8], format: &str) -> Result<Canvas, DecodeError> {
    let (width, height, mut pixels) = decode_straight(bytes, format)?;
    premultiply(&mut pixels);

    let expected = width as usize * height as usize * BPP;
    let len = pixels.len();

    Canvas::from_pixels(width, height, pixels.into_boxed_slice()).ok_or(DecodeError::Mismatched {
        width,
        height,
        len,
        expected,
    })
}

/// Decodes to straight-alpha RGBA, choosing the decoder by format.
fn decode_straight(bytes: &[u8], format: &str) -> Result<(u32, u32, Vec<u8>), DecodeError> {
    #[cfg(feature = "webp")]
    if format.eq_ignore_ascii_case("webp") {
        return decode_webp(bytes);
    }
    #[cfg(not(feature = "webp"))]
    let _ = format;

    let image = image::load_from_memory(bytes)
        .map_err(|err| DecodeError::Image(Box::new(err)))?
        .into_rgba8();

    Ok((image.width(), image.height(), image.into_raw()))
}

/// Decodes a WebP file to straight-alpha RGBA with libwebp.
///
/// The output always has an alpha channel. A fully opaque lossy tile is stored without one, and
/// libwebp fills in 255 for it.
#[cfg(feature = "webp")]
fn decode_webp(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), DecodeError> {
    use libwebp_sys as ffi;

    let (mut width, mut height) = (0_i32, 0_i32);
    // SAFETY: `bytes` is a valid slice, so `bytes.as_ptr()` and `bytes.len()` describe it
    // correctly. `width` and `height` are valid places to write the two dimensions.
    let ok =
        unsafe { ffi::WebPGetInfo(bytes.as_ptr(), bytes.len(), &raw mut width, &raw mut height) };
    if ok == 0 {
        return Err(DecodeError::Webp);
    }
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(DecodeError::Webp);
    };

    // libwebp limits each side to 16383, so this cannot overflow. It is checked anyway, because
    // the sizes come from the file, not from a canvas this crate made.
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(BPP))
        .ok_or(DecodeError::Webp)?;
    let stride = i32::try_from(width as usize * BPP).map_err(|_| DecodeError::Webp)?;

    let mut rgba = vec![0_u8; len];
    // SAFETY: `bytes` is a valid slice, passed with its own pointer and length. `rgba` is exactly
    // `height` rows of `stride` bytes, the layout this call fills. `rgba.len()` is passed as the
    // output buffer's size.
    let out = unsafe {
        ffi::WebPDecodeRGBAInto(
            bytes.as_ptr(),
            bytes.len(),
            rgba.as_mut_ptr(),
            rgba.len(),
            stride,
        )
    };
    if out.is_null() {
        return Err(DecodeError::Webp);
    }

    Ok((width, height, rgba))
}

impl TileSource for DziStore {
    type Error = DziStoreError;

    fn read_tile(&self, address: TileAddress) -> Result<Option<Canvas>, Self::Error> {
        let path = self.tile_path(address);

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            // A missing tile is transparent, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(DziStoreError::Io {
                    op: "reading",
                    path,
                    source,
                });
            }
        };

        decode_tile(&bytes, &self.extension)
            .map(Some)
            .map_err(|source| DziStoreError::Decode { path, source })
    }
}

/// The descriptor path for a tile directory: `{parent}/{name}.dzi` for `{parent}/{name}_files`.
///
/// If the directory name does not end in `_files`, the descriptor is the whole name plus `.dzi`.
fn descriptor_path(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = name.strip_suffix("_files").unwrap_or(name);
    root.with_file_name(format!("{stem}.dzi"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "webp")]
    use crate::testing::noise;

    /// An opaque 64x64 canvas of random noise. Lossy WebP is always much smaller on noise.
    #[cfg(feature = "webp")]
    fn opaque_noise(seed: u64) -> Canvas {
        let mut next = noise(seed);
        let mut canvas = Canvas::new(64, 64);
        for p in canvas.pixels_mut().as_chunks_mut::<4>().0 {
            *p = [next(), next(), next(), 255];
        }
        canvas
    }

    #[test]
    fn tile_paths_follow_the_deep_zoom_layout() {
        let store = DziStore::new("/out/layer0_files", "png");
        assert_eq!(
            store.tile_path(TileAddress::new(11, 3, 7)),
            PathBuf::from("/out/layer0_files/11/3_7.png")
        );
        assert_eq!(store.descriptor, PathBuf::from("/out/layer0.dzi"));
    }

    #[test]
    fn a_png_round_trips_through_the_encoder_and_store() -> Result<(), Box<dyn std::error::Error>> {
        // A RAII temp dir, so the directory is removed even if the test panics.
        let temp = tempfile::tempdir()?;
        let dir = temp.path();
        let store = DziStore::new(dir.join("layer0_files"), "png");

        let mut canvas = Canvas::new(4, 4);
        for (i, p) in canvas
            .pixels_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            // Premultiplied, so no color channel is larger than alpha.
            let alpha = 255 - u8::try_from(i * 4)?;
            *p = [alpha / 2, alpha / 3, alpha / 4, alpha];
        }

        let mut bytes = Vec::new();
        PngEncoder::default().encode(&canvas, &mut bytes)?;

        let address = TileAddress::new(5, 1, 2);
        store.write_tile(address, &bytes)?;

        let read = store.read_tile(address)?.expect("just written");
        assert_eq!((read.width(), read.height()), (4, 4));

        // Converting to straight alpha and back rounds each channel, so it can move by one.
        // Anything more means the encoder and decoder disagree.
        for (a, b) in read.pixels().iter().zip(canvas.pixels()) {
            assert!(a.abs_diff(*b) <= 1, "{a} vs {b}");
        }

        assert!(store.read_tile(TileAddress::new(5, 99, 99))?.is_none());

        // `temp` removes the directory itself when dropped.
        Ok(())
    }

    /// libwebp and the `image` crate must decode a tile the same way, because which one runs
    /// depends on build features.
    ///
    /// The opaque case matters most. A lossy WebP of an opaque tile has no alpha channel, and
    /// the two decoders must still both return four bytes per pixel.
    #[cfg(feature = "webp")]
    #[test]
    fn libwebp_and_image_decode_a_tile_the_same_way() -> Result<(), Box<dyn std::error::Error>> {
        let mut noise = noise(0x243F_6A88_85A3_08D3);

        for opaque in [true, false] {
            let mut canvas = Canvas::new(32, 32);
            for pixel in canvas.pixels_mut().as_chunks_mut::<4>().0 {
                let alpha = if opaque { 255 } else { noise() };
                // Premultiplied, so no channel may exceed alpha.
                *pixel = [
                    noise().min(alpha),
                    noise().min(alpha),
                    noise().min(alpha),
                    alpha,
                ];
            }

            for encoder in [WebpEncoder::LOSSLESS, WebpEncoder::lossy(90.0)] {
                let mut bytes = Vec::new();
                encoder.encode(&canvas, &mut bytes)?;

                let (lw, lh, libwebp) = decode_webp(&bytes)?;
                let through_image = image::load_from_memory(&bytes)?.into_rgba8();

                assert_eq!((lw, lh), (through_image.width(), through_image.height()));
                assert_eq!(
                    libwebp,
                    through_image.into_raw(),
                    "opaque {opaque}, {} encoder",
                    if matches!(encoder.mode(), WebpMode::Lossless) {
                        "lossless"
                    } else {
                        "lossy"
                    }
                );
            }
        }

        Ok(())
    }

    /// A fully opaque lossy tile, stored without an alpha channel, reads back as opaque.
    #[cfg(feature = "webp")]
    #[test]
    fn an_opaque_lossy_tile_reads_back_opaque() -> Result<(), Box<dyn std::error::Error>> {
        // A RAII temp dir, so the directory is removed even if the test panics.
        let temp = tempfile::tempdir()?;
        let dir = temp.path();
        let store = DziStore::new(dir.join("layer0_files"), "webp");

        let mut canvas = Canvas::new(32, 32);
        for (i, pixel) in canvas
            .pixels_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            let i = u8::try_from(i % 256)?;
            *pixel = [
                i.wrapping_mul(7),
                i.wrapping_mul(29),
                i.wrapping_mul(3),
                255,
            ];
        }

        let mut bytes = Vec::new();
        WebpEncoder::lossy(90.0).encode(&canvas, &mut bytes)?;

        let address = TileAddress::new(2, 1, 1);
        store.write_tile(address, &bytes)?;
        let read = store.read_tile(address)?.expect("just written");

        assert_eq!((read.width(), read.height()), (32, 32));
        for pixel in read.pixels().as_chunks::<4>().0 {
            assert_eq!(pixel.get(3), Some(&255), "alpha survived the round trip");
        }

        // `temp` removes the directory itself when dropped.
        Ok(())
    }

    /// A lossless WebP tile must read back exactly as written. Any error would grow with each
    /// level of the pyramid.
    #[cfg(feature = "webp")]
    #[test]
    fn a_lossless_webp_round_trips_bit_exact() -> Result<(), Box<dyn std::error::Error>> {
        // A RAII temp dir, so the directory is removed even if the test panics.
        let temp = tempfile::tempdir()?;
        let dir = temp.path();
        let store = DziStore::new(dir.join("layer0_files"), "webp");

        // Fully opaque, so the alpha conversions change nothing. Any difference comes from the
        // codec.
        let mut canvas = Canvas::new(16, 16);
        for (i, p) in canvas
            .pixels_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            let i = u8::try_from(i)?;
            *p = [i.wrapping_mul(7), i.wrapping_mul(13), i, 255];
        }

        let mut bytes = Vec::new();
        WebpEncoder::LOSSLESS.encode(&canvas, &mut bytes)?;

        let address = TileAddress::new(3, 4, 5);
        store.write_tile(address, &bytes)?;

        let read = store.read_tile(address)?.expect("just written");
        assert_eq!((read.width(), read.height()), (16, 16));
        assert_eq!(read.pixels(), canvas.pixels());

        // `temp` removes the directory itself when dropped.
        Ok(())
    }

    /// Adaptive must give the smaller of the two modes' outputs, on both kinds of content a map
    /// contains.
    #[cfg(feature = "webp")]
    #[test]
    fn adaptive_is_never_worse_than_either_mode() -> Result<(), Box<dyn std::error::Error>> {
        let encode = |encoder: &WebpEncoder, canvas: &Canvas| -> Result<usize, WebpError> {
            let mut out = Vec::new();
            encoder.encode(canvas, &mut out)?;
            Ok(out.len())
        };

        // Noise, which lossy encoding compresses far better than lossless.
        let noisy = opaque_noise(0x9E37_79B9_7F4A_7C15);

        // A repeating 4x4 pattern. Most of a Project Zomboid map repeats like this, and lossless
        // encoding compresses it well.
        let mut tiled = Canvas::new(64, 64);
        for (i, p) in tiled
            .pixels_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .enumerate()
        {
            let cell = u8::try_from((i % 4) * 16 + (i / 64 % 4) * 4).unwrap_or(0);
            *p = [cell, 128 - cell, 60, 255];
        }

        let megapixels = 64.0 * 64.0 / 1_000_000.0;
        let worth_trying =
            f64::from(WebpEncoder::LOSSY_WORTH_TRYING_BYTES_PER_MEGAPIXEL) * megapixels;

        // On noise, lossy wins by far more than any reasonable margin, so adaptive picks it.
        let lossless = encode(&WebpEncoder::LOSSLESS, &noisy)?;
        let lossy = encode(&WebpEncoder::lossy(92.0), &noisy)?;
        let adaptive = encode(&WebpEncoder::adaptive(92.0), &noisy)?;
        assert!(
            lossless as f64 >= worth_trying,
            "noise should not compress under the skip gate; got {lossless} vs {worth_trying}",
        );
        assert_eq!(
            adaptive,
            lossless.min(lossy),
            "adaptive {adaptive} should be min(lossless {lossless}, lossy {lossy})",
        );

        // The repeating pattern compresses below the worth-trying threshold. So the lossy pass
        // never runs, and the output is exactly the lossless bytes.
        let lossless = encode(&WebpEncoder::LOSSLESS, &tiled)?;
        let adaptive = encode(&WebpEncoder::adaptive(92.0), &tiled)?;
        assert!(
            (lossless as f64) < worth_trying,
            "a repeating pattern should compress under the skip gate; got {lossless}",
        );
        assert_eq!(
            adaptive, lossless,
            "under the skip gate adaptive must return the lossless bytes untouched",
        );

        Ok(())
    }

    /// The margin, not just which file is smaller, must decide the mode.
    #[cfg(feature = "webp")]
    #[test]
    fn the_margin_decides_the_mode_not_the_byte_count() -> Result<(), Box<dyn std::error::Error>> {
        let encode = |encoder: &WebpEncoder, canvas: &Canvas| -> Result<Vec<u8>, WebpError> {
            let mut out = Vec::new();
            encoder.encode(canvas, &mut out)?;
            Ok(out)
        };

        let canvas = opaque_noise(0x243F_6A88_85A3_08D3);

        let lossless = encode(&WebpEncoder::LOSSLESS, &canvas)?;
        let lossy = encode(&WebpEncoder::lossy(92.0), &canvas)?;
        assert!(
            lossy.len() < lossless.len(),
            "the premise: lossy {} should beat lossless {} on noise",
            lossy.len(),
            lossless.len(),
        );

        // With a zero margin, adaptive keeps the smaller file.
        let none = encode(&WebpEncoder::adaptive(92.0).with_lossy_margin(0), &canvas)?;
        assert_eq!(none, lossy, "a zero margin should take the smaller file");

        // Lossy can never save more than the whole lossless file. So with a margin that large,
        // adaptive keeps lossless even though lossy is smaller.
        let huge = encode(
            &WebpEncoder::adaptive(92.0).with_lossy_margin(u32::MAX / 2),
            &canvas,
        )?;
        assert_eq!(
            huge, lossless,
            "an unreachable margin should keep lossless whatever the sizes",
        );

        Ok(())
    }
}

#[cfg(test)]
mod fs_store_tests {
    use super::*;
    fn address(level: u32, col: u32, row: u32) -> TileAddress {
        TileAddress { level, col, row }
    }

    #[test]
    fn prepare_creates_every_level_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("out_files"), "png");

        store.prepare(4).expect("preparing four levels");

        for level in 0..4 {
            assert!(
                dir.path()
                    .join("out_files")
                    .join(level.to_string())
                    .is_dir(),
                "level {level} directory missing"
            );
        }
        assert!(!dir.path().join("out_files").join("4").exists());
    }

    /// A store can be used without `render_pyramid`, so writing to a level that was never
    /// prepared must still work.
    #[test]
    fn a_write_without_prepare_still_lands() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("out_files"), "png");

        store
            .write_tile(address(2, 1, 1), b"tile bytes")
            .expect("the write creates its own directory");

        assert_eq!(
            fs::read(store.tile_path(address(2, 1, 1))).expect("the tile exists"),
            b"tile bytes"
        );
    }

    /// A crash during a write leaves a temporary file, which must not be read as a tile.
    #[test]
    fn a_leftover_temp_file_is_not_read_as_a_tile() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("out_files"), "png");
        store.prepare(1).expect("preparing");

        let mut temp = store.tile_path(address(0, 0, 0));
        temp.as_mut_os_string().push(".tmp");
        fs::write(&temp, b"half a tile").expect("simulating a crashed write");

        assert!(
            store
                .read_tile(address(0, 0, 0))
                .expect("a missing tile is not an error")
                .is_none(),
            "the temp file was mistaken for a tile"
        );
    }

    #[test]
    fn an_atomic_write_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("out_files"), "png");
        store.prepare(1).expect("preparing");
        store
            .write_tile(address(0, 3, 4), b"tile")
            .expect("writing");

        let level = dir.path().join("out_files").join("0");
        let names: Vec<_> = fs::read_dir(&level)
            .expect("listing the level")
            .filter_map(|e| Some(e.ok()?.file_name().to_string_lossy().into_owned()))
            .collect();

        assert_eq!(names, vec!["3_4.png".to_owned()], "stray files: {names:?}");
    }

    /// Tests `classify` directly, rather than by filling a real disk.
    #[test]
    fn a_full_disk_is_reported_as_out_of_space() {
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert!(matches!(
            classify("writing", PathBuf::from("/x"), full),
            DziStoreError::OutOfSpace { .. }
        ));

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(
            classify("writing", PathBuf::from("/x"), denied),
            DziStoreError::Io { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_quota_is_reported_as_out_of_space() {
        // EDQUOT has no `ErrorKind` of its own, so only the raw error number identifies it.
        let quota = std::io::Error::from_raw_os_error(122);
        assert!(matches!(
            classify("writing", PathBuf::from("/x"), quota),
            DziStoreError::OutOfSpace { .. }
        ));
    }
}
