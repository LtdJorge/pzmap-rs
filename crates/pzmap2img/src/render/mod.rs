//! Turns a built [`Map`] into Deep Zoom image tiles.
//!
//! A [`Map`] comes out of [`build`](crate::build). It holds every cell of the map, what is on
//! each square (floors, walls, furniture and so on, which the game calls tiles), and the sprites
//! that draw them. Rendering draws one height layer of that map as a single, very large
//! isometric image. Each square is a diamond on screen, and each layer above the ground is
//! drawn higher up the image. [`geometry`] has the projection from squares to pixels.
//!
//! The image is far too large to hold in memory, so it is cut into square image tiles and stored
//! as a Deep Zoom pyramid. The bottom of the pyramid, called the native level, holds the image
//! at full resolution and has the highest level number. Each level above it is the level below
//! scaled down by half, up to level 0, which is a single pixel. An image tile is named by its
//! [`TileAddress`]: a level, plus a column and row within that level. [`pyramid`] has the
//! details. From here on, "tile" means an image tile.
//!
//! To render, describe the work with a [`RenderJob`]: which layer, the tile size, the sprite
//! scale, and optionally a smaller region. Then call [`render_pyramid`]. It draws each native
//! tile by compositing the sprites of every square that reaches into it, back to front, with
//! premultiplied alpha (see [`blend`]). It builds each upper tile by averaging its four children
//! from the level below. Every finished tile goes through a [`TileEncoder`], which turns pixels
//! into PNG or WebP bytes, and then to a [`TileSink`], which stores them. Tiles that come out
//! fully transparent are never written. On most maps, that is most of them.
//!
//! The levels just above the native one are built in memory as the native tiles finish. Higher
//! levels read their children back through a [`TileSource`], usually the same store that wrote
//! them. [`RenderJob::fuse_depth`] sets where that switch happens.
//!
//! [`RunContext`] lets the caller watch and control a run. Its `on_tile` hook is called for every
//! written tile, and its `stop` hook ends the run early with [`RenderError::Stopped`].
//!
//! ```no_run
//! use pzmap2img::render::{
//!     DziStore, PngEncoder, RenderJob, RenderPlan, RunContext, TileEncoders, render_pyramid,
//! };
//! # fn demo(map: &pzmap2img::Map) -> Result<(), Box<dyn std::error::Error>> {
//! let job = RenderJob::for_layer(0);
//! let store = DziStore::new("/tmp/out/layer0_files", "png");
//! let encoder = PngEncoder::default();
//! // No progress reports and no way to stop early.
//! let ctx = RunContext::none();
//! let stats = render_pyramid(map, &job, TileEncoders::uniform(&encoder), &store, ctx)?;
//! store.write_descriptor(RenderPlan::new(map, &job)?.pyramid())?;
//! println!("{} tiles, {} skipped as empty", stats.written, stats.empty);
//! # Ok(()) }
//! ```
//!
//! The built-in encoders are [`PngEncoder`] and `WebpEncoder` (with the `webp` feature). The
//! built-in store is [`DziStore`], which writes tiles as files in the Deep Zoom layout and reads
//! them back. To write another format or send tiles somewhere else, implement the traits.
//!
//! [`RenderPlan`] works out the image size and pyramid shape for a job. It is `Copy`, so a server
//! can compute one plan and give each worker a different [`TileRect`] of native tiles. All the
//! workers then agree on where every tile starts and ends.

pub mod blend;
pub mod geometry;
pub mod margins;
pub(crate) mod pool;
pub mod pyramid;
pub mod store;

use crate::{
    math::{
        div_ceil_i32,
        div_floor_i32,
    },
    render::{
        blend::{
            BPP,
            Canvas,
        },
        geometry::{
            GRID_ROWS_PER_LAYER,
            IsoLayout,
            IsoScale,
            cell_grid_bounds,
            map_grid_bounds,
        },
        margins::{
            SpriteExtent,
            output_margin,
            render_margin,
        },
        pyramid::{
            Pyramid,
            TileAddress,
            crop_to_level,
            halve_into,
            merge_children,
        },
    },
    runtime::raster::Map,
};
use pool::{
    CanvasPool,
    PooledCanvas,
};
use rayon::prelude::*;
use std::{
    error::Error,
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        Instant,
    },
};
use thiserror::Error;
use tracing::{
    debug,
    info,
    instrument,
    warn,
};

pub use pool::PoolError;
pub use store::{
    BuiltinEncoder,
    DecodeError,
    DziStore,
    PngEncoder,
    decode_tile,
};
#[cfg(feature = "webp")]
pub use store::{
    WebpEncoder,
    WebpMode,
};

/// Typical encoded size, in bytes per megapixel, of a non-empty tile from the default lossless
/// WebP encoder at 2x.
///
/// `pzmap2img plan` uses it to estimate disk usage before rendering.
pub const TYPICAL_BYTES_PER_MEGAPIXEL: u64 = 810_000;

/// An error from planning or running a render.
///
/// Errors from encoders and sinks are boxed, so every render returns this one type. The original
/// error is still available through [`Error::source`] and `downcast_ref`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The tile size is not a whole number of isometric grid steps.
    ///
    /// Such tiles would end partway through a grid step, and neighboring tiles would disagree
    /// about which squares they cover.
    #[error("tile size {size} is not a multiple of the {scale} isometric grid step")]
    TileSize { size: u32, scale: IsoScale },

    /// The map has no cells.
    #[error("the map has no cells to render")]
    EmptyMap,

    /// The native and upper levels use encoders with different formats.
    ///
    /// A Deep Zoom descriptor names one format for the whole pyramid, so they must match.
    #[error(
        "the native level encodes as {native} but the upper levels as {upper}; one pyramid has one format"
    )]
    FormatMismatch { native: String, upper: String },

    /// The requested layer is outside the range of layers the map uses.
    #[error("layer {layer} is outside the map's populated range {min}..={max}")]
    LayerOutOfRange { layer: i32, min: i32, max: i32 },

    /// The [`TileEncoder`] failed on a tile.
    #[error("encoding tile {address}")]
    Encode {
        address: TileAddress,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },

    /// The [`TileSink`] failed to store a tile.
    #[error("writing tile {address}")]
    Write {
        address: TileAddress,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },

    /// The [`TileSource`] failed to read back a child tile.
    #[error("reading back tile {address}")]
    Read {
        address: TileAddress,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },

    /// Rendering stopped because [`RunContext::stop`] returned true.
    #[error("the render was stopped")]
    Stopped,

    /// The disk does not have room for the estimated output plus a safety margin.
    ///
    /// Returned by [`check_free_space`] before any tile is drawn.
    #[error(
        "{} has {available} bytes free; this render needs about {needed}",
        path.display()
    )]
    NotEnoughSpace {
        path: PathBuf,
        needed: u64,
        available: u64,
    },

    /// [`TileSink::prepare`] failed. For [`DziStore`], a level directory could not be created.
    #[error("preparing the tile store for {levels} levels")]
    Prepare {
        levels: u32,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },

    /// The in-memory levels used up the memory budget for tile buffers.
    #[error("out of tile memory building {address}")]
    Memory {
        address: TileAddress,
        #[source]
        source: PoolError,
    },

    /// [`RenderJob::memory_budget`] is too small to build the in-memory levels at all.
    #[error("the tile memory budget is too small")]
    MemoryBudget(#[source] PoolError),
}

/// Free bytes on the filesystem holding `path` that a non-root user can use.
///
/// Uses `f_bavail` rather than `f_bfree`, because blocks reserved for root are not usable by
/// other users. Returns `None` on targets other than unix, where this is not measured.
#[must_use]
pub(crate) fn available_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let stat = rustix::fs::statvfs(path).ok()?;
        Some(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Checks that the disk holding `dir` has room for a render of about `needed` bytes.
///
/// Asks for ten percent more than `needed`, because size estimates are typical values, not
/// upper bounds. If free space cannot be measured, as on targets other than unix, the check logs
/// a warning and passes.
///
/// # Errors
///
/// Returns [`RenderError::NotEnoughSpace`] when the filesystem holding `dir` does not have room.
pub fn check_free_space(dir: &Path, needed: u64) -> Result<(), RenderError> {
    let Some(available) = available_bytes(dir) else {
        warn!(
            path = %dir.display(),
            "cannot measure free space on this platform; skipping the preflight"
        );
        return Ok(());
    };

    let required = needed.saturating_add(needed / 10);

    if available < required {
        return Err(RenderError::NotEnoughSpace {
            path: dir.to_path_buf(),
            needed: required,
            available,
        });
    }

    debug!(needed = required, available, "free space preflight passed");
    Ok(())
}

/// Optional hooks for a render run: one to report progress and one to stop early.
///
/// The hooks are called from the render's worker threads, so they must be `Sync`.
///
/// # Examples
///
/// Count written tiles and give up after a deadline:
///
/// ```
/// use pzmap2img::render::{RunContext, TileReport};
/// use std::{
///     sync::atomic::{AtomicU64, Ordering},
///     time::{Duration, Instant},
/// };
///
/// let written = AtomicU64::new(0);
/// let on_tile = |_: &TileReport| {
///     written.fetch_add(1, Ordering::Relaxed);
/// };
/// let deadline = Instant::now() + Duration::from_secs(60);
/// let stop = || Instant::now() > deadline;
///
/// let ctx = RunContext {
///     on_tile: Some(&on_tile),
///     stop: Some(&stop),
/// };
/// // Pass `ctx` to `render_pyramid`.
/// # let _ = ctx;
/// ```
#[derive(Copy, Clone, Default)]
pub struct RunContext<'a> {
    /// Called after each tile is written, on the thread that wrote it.
    pub on_tile: Option<&'a (dyn Fn(&TileReport) + Sync)>,

    /// Called before each tile. Once it returns true, the render stops with
    /// [`RenderError::Stopped`]. Tiles already written stay written.
    pub stop: Option<&'a (dyn Fn() -> bool + Sync)>,
}

impl RunContext<'_> {
    /// Creates a context with no hooks.
    #[must_use]
    pub fn none() -> Self {
        RunContext::default()
    }

    /// Returns [`RenderError::Stopped`] if the stop hook says so.
    fn check(&self) -> Result<(), RenderError> {
        match self.stop {
            Some(stop) if stop() => Err(RenderError::Stopped),
            _ => Ok(()),
        }
    }
}

impl std::fmt::Debug for RunContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunContext")
            .field("on_tile", &self.on_tile.is_some())
            .field("stop", &self.stop.is_some())
            .finish()
    }
}

/// Details of one written tile, passed to [`RunContext::on_tile`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileReport {
    /// Where the tile sits in the pyramid.
    pub address: TileAddress,
    /// Width in pixels. Tiles on the right edge of a level can be narrower than the tile size.
    pub width: u32,
    /// Height in pixels. Tiles on the bottom edge of a level can be shorter than the tile size.
    pub height: u32,
    /// Encoded size in bytes.
    pub bytes: u64,
    /// Time spent in [`TileEncoder::encode`].
    pub encode_time: Duration,
    /// What the encoder reported doing.
    pub encoded: EncodeReport,
}

/// Turns a finished tile into encoded bytes, such as a PNG or WebP file.
///
/// The built-in encoders are [`PngEncoder`] and `WebpEncoder`. [`BuiltinEncoder`] holds either
/// one, for when the format is chosen at run time.
///
/// One encoder is shared by all the render threads, so `encode` must be safe to call from several
/// threads at once.
pub trait TileEncoder: Sync {
    /// The error returned when a tile cannot be encoded.
    type Error: Error + Send + Sync + 'static;

    /// The format name, used both as the file extension and as the Deep Zoom `Format`
    /// attribute. For example, `"png"`.
    fn format(&self) -> &str;

    /// Encodes `tile`, appends the result to `out`, and reports what it did.
    ///
    /// `tile` holds RGBA pixels, four bytes each, with premultiplied alpha. Most image formats
    /// store straight alpha, so an encoder usually has to convert first; see
    /// [`blend::unpremultiply`].
    ///
    /// The render clears `out` before each call and reuses it for the next tile, so append to it
    /// rather than replacing it. An encoder with only one mode returns [`EncodeReport::single`].
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the tile cannot be encoded.
    fn encode(&self, tile: &Canvas, out: &mut Vec<u8>) -> Result<EncodeReport, Self::Error>;
}

/// Which encoding mode an encoder used for a tile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EncodedAs {
    /// The encoder has only one mode, so there was no choice to make.
    Single,
    /// Exact encoding.
    Lossless,
    /// Approximate encoding that trades some detail for a smaller file.
    Lossy,
}

impl EncodedAs {
    /// A short name for metrics output. It does not change between releases.
    pub const fn name(self) -> &'static str {
        match self {
            EncodedAs::Single => "single",
            EncodedAs::Lossless => "lossless",
            EncodedAs::Lossy => "lossy",
        }
    }
}

/// What an encoder did with one tile: which mode it used and how many times it ran the codec.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeReport {
    /// The mode used for the bytes that were kept.
    pub encoded_as: EncodedAs,
    /// How many times the codec ran. An adaptive encoder that tries both modes reports 2, or 1
    /// when it could tell from the first pass that the second was not worth running.
    pub passes: u8,
}

impl EncodeReport {
    /// The report for an encoder with only one mode: one pass, no choice made.
    #[must_use]
    pub const fn single() -> Self {
        EncodeReport {
            encoded_as: EncodedAs::Single,
            passes: 1,
        }
    }

    /// A report for the given mode and number of codec passes.
    #[must_use]
    pub const fn new(encoded_as: EncodedAs, passes: u8) -> Self {
        EncodeReport { encoded_as, passes }
    }
}

/// Stores encoded tiles: on disk, in object storage, or anywhere else.
///
/// One sink is shared by all the render threads, so `write_tile` must be safe to call from
/// several threads at once. Each write must also be all-or-nothing. The store must never hold
/// part of a tile.
///
/// Fully transparent tiles are never written. A sink that is also a [`TileSource`] should report
/// them as missing.
pub trait TileSink: Sync {
    /// The error returned when a tile cannot be stored.
    type Error: Error + Send + Sync + 'static;

    /// Stores the encoded `bytes` of the tile at `address`.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the tile cannot be written.
    fn write_tile(&self, address: TileAddress, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Gets ready for a run that writes levels `0..levels`.
    ///
    /// The render calls this once, before the first tile. The default does nothing, which suits
    /// a sink with no directories or other setup to create.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the sink cannot prepare itself.
    fn prepare(&self, _levels: u32) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The encoders for the native level and for the levels above it.
///
/// The native level has the most fine detail, which is where lossy encoding saves the most.
/// Upper levels get smoother each time they are scaled down, so a caller may want a different
/// encoder there. Both encoders must produce the same format. Use [`Self::uniform`] to encode
/// every level the same way.
#[derive(Debug)]
pub struct TileEncoders<'a, E> {
    /// Encodes the native, full-resolution level.
    pub native: &'a E,
    /// Encodes every level above the native one.
    pub upper: &'a E,
}

// A derived `Clone` or `Copy` would require `E: Copy`, which no encoder is. The struct only holds
// two shared references, so it can be `Copy` for any `E`.
impl<E> Clone for TileEncoders<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E> Copy for TileEncoders<'_, E> {}

impl<'a, E> TileEncoders<'a, E> {
    /// One encoder for the whole pyramid.
    pub const fn uniform(encoder: &'a E) -> Self {
        TileEncoders {
            native: encoder,
            upper: encoder,
        }
    }

    /// One encoder for the native level and another for every level above it.
    pub const fn split(native: &'a E, upper: &'a E) -> Self {
        TileEncoders { native, upper }
    }
}

/// Reads stored tiles back, so the upper levels of a pyramid can be built from the levels below.
///
/// One source is shared by all the render threads, so `read_tile` must be safe to call from
/// several threads at once.
pub trait TileSource: Sync {
    /// The error returned when a stored tile cannot be read.
    type Error: Error + Send + Sync + 'static;

    /// Reads back the tile at `address` as premultiplied pixels.
    ///
    /// Returns `Ok(None)` if no tile was stored there. That is not an error: the render never
    /// stores fully transparent tiles, so a missing tile is treated as transparent.
    /// [`decode_tile`] turns the bytes of a built-in encoder back into pixels.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the tile exists but cannot be read.
    fn read_tile(&self, address: TileAddress) -> Result<Option<Canvas>, Self::Error>;
}

/// A rectangle of tiles within one pyramid level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TileRect {
    /// Column of the leftmost tile.
    pub col: u32,
    /// Row of the top tile.
    pub row: u32,
    /// Width in tiles.
    pub cols: u32,
    /// Height in tiles.
    pub rows: u32,
}

impl TileRect {
    /// A rectangle `cols` tiles wide and `rows` tiles high, starting at tile `(col, row)`.
    pub const fn new(col: u32, row: u32, cols: u32, rows: u32) -> Self {
        TileRect {
            col,
            row,
            cols,
            rows,
        }
    }

    /// The number of tiles in the rectangle.
    pub const fn tiles(&self) -> u64 {
        self.cols as u64 * self.rows as u64
    }

    /// The smallest rectangle of tiles on the level above that covers this one.
    ///
    /// Each parent tile covers a 2x2 block of child tiles, so the result can stick out by half a
    /// parent tile on each side. For example, a rectangle starting at an odd column shares its
    /// first parent with the column to its left.
    pub const fn parent(self) -> Self {
        let col = self.col / 2;
        let row = self.row / 2;
        TileRect {
            col,
            row,
            cols: (self.col + self.cols).div_ceil(2) - col,
            rows: (self.row + self.rows).div_ceil(2) - row,
        }
    }

    /// Whether tile `(col, row)` is inside the rectangle.
    pub const fn contains(&self, col: u32, row: u32) -> bool {
        col >= self.col
            && row >= self.row
            && col - self.col < self.cols
            && row - self.row < self.rows
    }

    /// Cuts the rectangle down to fit a level that is `cols` by `rows` tiles.
    pub fn clamp_to(self, cols: u32, rows: u32) -> Self {
        let col = self.col.min(cols);
        let row = self.row.min(rows);
        TileRect {
            col,
            row,
            cols: self.cols.min(cols - col),
            rows: self.rows.min(rows - row),
        }
    }
}

/// What to render: one height layer of the map, and how to cut it into tiles.
///
/// Start from [`RenderJob::for_layer`] and change settings with the `with_*` methods.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RenderJob {
    /// The height layer to draw. Layer 0 is the ground, and basements are negative.
    /// Each pyramid shows one layer.
    pub layer: i32,

    /// Width and height of a tile in pixels. It must be a whole number of grid steps both ways,
    /// which at 2x means a multiple of 64.
    pub tile_size: u32,

    /// How many pyramid levels must have tile edges in the same places. The image's top-left
    /// corner is placed so that this holds.
    pub align_levels: u32,

    /// The lowest and highest layers to leave room for in the image. Rendering every layer with
    /// the same range puts them all in the same image coordinates, so they line up. `None` uses
    /// the layers the map has.
    pub layer_range: Option<(i32, i32)>,

    /// The sprite scale, which must match the texture packs the map was built with.
    /// [`IsoScale::X2`] is for `Tiles2x.pack` and the other 2x packs, [`IsoScale::X1`] for the 1x
    /// packs.
    pub scale: IsoScale,

    /// How far sprites can reach from their square, which sets the image margins. `None`
    /// measures it from the map's sprites.
    pub sprite_extent: Option<SpriteExtent>,

    /// Which native-level tiles to render. `None` renders all of them.
    pub region: Option<TileRect>,

    /// How many levels above the native one are built in memory, instead of from tiles read back
    /// through [`TileSource`]. `0` builds only the native level in memory. See
    /// [`DEFAULT_FUSE_DEPTH`].
    pub fuse_depth: u32,

    /// The most memory, in bytes, the in-memory levels may use for tile buffers.
    ///
    /// They need at least `threads * (fuse_depth + 2)` tiles of `tile_size * tile_size * 4`
    /// bytes, and a smaller budget is an error. `None` gives them that much, plus extra so that a
    /// small map's top levels can still use every thread.
    pub memory_budget: Option<u64>,
}

impl RenderJob {
    /// A job for the whole of one layer, with the same defaults as `pzmap2dzi`: 1024-pixel
    /// tiles at 2x, aligned over 3 levels.
    pub const fn for_layer(layer: i32) -> Self {
        RenderJob {
            layer,
            tile_size: 1024,
            align_levels: 3,
            layer_range: None,
            scale: IsoScale::X2,
            sprite_extent: None,
            region: None,
            fuse_depth: DEFAULT_FUSE_DEPTH,
            memory_budget: None,
        }
    }

    /// Sets how many levels above the native one to build in memory. See [`Self::fuse_depth`].
    pub const fn with_fuse_depth(mut self, depth: u32) -> Self {
        self.fuse_depth = depth;
        self
    }

    /// Sets the tile size in pixels. See [`Self::tile_size`].
    pub const fn with_tile_size(mut self, tile_size: u32) -> Self {
        self.tile_size = tile_size;
        self
    }

    /// Sets how many levels must share tile edges. See [`Self::align_levels`].
    pub const fn with_align_levels(mut self, levels: u32) -> Self {
        self.align_levels = levels;
        self
    }

    /// Sets the sprite scale. See [`Self::scale`].
    pub const fn with_scale(mut self, scale: IsoScale) -> Self {
        self.scale = scale;
        self
    }

    /// Fixes the range of layers the image leaves room for, so that pyramids for different
    /// layers line up. See [`Self::layer_range`].
    pub const fn with_layer_range(mut self, range: (i32, i32)) -> Self {
        self.layer_range = Some(range);
        self
    }

    /// Renders only the native tiles in `region`, plus the upper tiles built from them.
    pub const fn with_region(mut self, region: TileRect) -> Self {
        self.region = Some(region);
        self
    }

    /// Fixes how far sprites reach, instead of measuring it from the map.
    pub const fn with_sprite_extent(mut self, extent: SpriteExtent) -> Self {
        self.sprite_extent = Some(extent);
        self
    }

    /// Sets the memory limit for tile buffers, in bytes. See [`Self::memory_budget`].
    pub const fn with_memory_budget(mut self, bytes: u64) -> Self {
        self.memory_budget = Some(bytes);
        self
    }
}

impl Default for RenderJob {
    fn default() -> Self {
        RenderJob::for_layer(0)
    }
}

/// The image size, tile grid and pyramid shape for one [`RenderJob`] on one map.
///
/// The plan is small and `Copy`. Workers that each get a copy agree on where every tile is.
#[derive(Clone, Copy, Debug)]
pub struct RenderPlan {
    layout: IsoLayout,
    pyramid: Pyramid,
    layer: i32,
}

impl RenderPlan {
    /// Works out the plan for `job` on `map`.
    ///
    /// The image covers every cell of the map plus a margin for sprites that reach past the
    /// map's edge.
    ///
    /// # Examples
    ///
    /// Find out how big a render would be before starting it:
    ///
    /// ```no_run
    /// use pzmap2img::render::{RenderJob, RenderPlan};
    /// # fn demo(map: &pzmap2img::Map) -> Result<(), pzmap2img::RenderError> {
    /// let job = RenderJob::for_layer(0);
    /// let plan = RenderPlan::new(map, &job)?;
    /// let coverage = plan.coverage(map);
    /// println!(
    ///     "{:?} pixels, about {} tiles, about {} bytes",
    ///     plan.pyramid().canonical(),
    ///     plan.estimated_tiles(&job, &coverage),
    ///     plan.estimated_output_bytes(&job, &coverage),
    /// );
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::EmptyMap`] if the map has no cells,
    /// [`RenderError::LayerOutOfRange`] if `job.layer` falls outside the populated range, and
    /// [`RenderError::TileSize`] if `job.tile_size` is not a multiple of the grid step.
    pub fn new(map: &Map, job: &RenderJob) -> Result<Self, RenderError> {
        let (min_layer, max_layer) = match job.layer_range {
            Some(range) => range,
            None => map.layer_range().ok_or(RenderError::EmptyMap)?,
        };

        if job.layer < min_layer || job.layer > max_layer {
            return Err(RenderError::LayerOutOfRange {
                layer: job.layer,
                min: min_layer,
                max: max_layer,
            });
        }

        let extent = job
            .sprite_extent
            .unwrap_or_else(|| SpriteExtent::measured(map.textures()));

        let cell_squares = map.geometry().cell_squares().cast_signed();
        let cells = map.cells().iter().map(|cell| (cell.cell_x, cell.cell_y));
        let bounds = map_grid_bounds(cells, cell_squares).ok_or(RenderError::EmptyMap)?;

        // `layer_range` gives the top layer itself, but `output_margin` wants the layer above
        // it, like the half-open range in a lotheader. The margin is added before the origin is
        // aligned below. Aligning first could put the image's corner inside the margin.
        let bounds = bounds + output_margin(extent, job.scale, min_layer, max_layer + 1);

        let layout = IsoLayout::new(
            bounds,
            job.scale,
            job.tile_size,
            job.align_levels,
            render_margin(extent, job.scale),
        )
        .ok_or(RenderError::TileSize {
            size: job.tile_size,
            scale: job.scale,
        })?;

        let (width, height) = layout.canonical_pixels();

        Ok(RenderPlan {
            layout,
            pyramid: Pyramid::new(width, height, job.tile_size),
            layer: job.layer,
        })
    }

    /// Where the image sits on the isometric grid, and how it is cut into tiles.
    pub const fn layout(&self) -> &IsoLayout {
        &self.layout
    }

    /// The pyramid's size, tile size and number of levels.
    pub const fn pyramid(&self) -> &Pyramid {
        &self.pyramid
    }

    /// The layer being drawn.
    pub const fn layer(&self) -> i32 {
        self.layer
    }

    /// The lowest level whose tiles [`render_pyramid`] reads back through [`TileSource`].
    ///
    /// This level and every level below it, down to the native level, are built in memory. Each
    /// level above it is built from tiles of the level below, read back from the store. So a
    /// store only has to read back tiles from this level and the levels above it.
    pub fn read_back_level(&self, job: &RenderJob) -> u32 {
        let native = self.pyramid.max_level();
        native - job.fuse_depth.min(native)
    }

    /// The rectangle holding every tile of the native level.
    pub fn native_region(&self) -> TileRect {
        let (cols, rows) = self.pyramid.tiles_across(self.pyramid.max_level());
        TileRect::new(0, 0, cols, rows)
    }

    /// The native tiles `job` renders: its region cut down to fit the level, or the whole level.
    pub fn region_of(&self, job: &RenderJob) -> TileRect {
        let full = self.native_region();
        match job.region {
            Some(region) => region.clamp_to(full.cols, full.rows),
            None => full,
        }
    }

    /// The most tiles a render of `job` can write, counting every level.
    ///
    /// `coverage` is this plan's [`Self::coverage`]. The result is whichever is smaller: the
    /// tiles that cover the map, or all the tiles in the job's region.
    #[must_use]
    pub fn estimated_tiles(&self, job: &RenderJob, coverage: &Coverage) -> u64 {
        let mut region = self.region_of(job);
        let mut region_tiles = region.tiles();
        for _ in 0..self.pyramid.max_level() {
            region = region.parent();
            region_tiles = region_tiles.saturating_add(region.tiles());
        }
        coverage.pyramid_tiles().min(region_tiles)
    }

    /// Roughly how many bytes a render of `job` will write.
    ///
    /// This is an estimate, not a limit. The tile count is an upper bound, but the size of each
    /// tile is a typical value from [`TYPICAL_BYTES_PER_MEGAPIXEL`].
    #[must_use]
    pub fn estimated_output_bytes(&self, job: &RenderJob, coverage: &Coverage) -> u64 {
        self.estimated_tiles(job, coverage)
            .saturating_mul(self.estimated_tile_bytes())
    }

    /// Roughly how many bytes one written tile takes. See [`TYPICAL_BYTES_PER_MEGAPIXEL`].
    pub fn estimated_tile_bytes(&self) -> u64 {
        let size = u64::from(self.pyramid.tile_size());
        size * size * TYPICAL_BYTES_PER_MEGAPIXEL / 1_000_000
    }

    /// Counts the native tiles that might show part of the map, without drawing anything.
    ///
    /// A tile is counted if it draws any square of a cell that has something on this layer.
    /// Some counted tiles can still come out empty, so the count is an upper bound.
    pub fn coverage(&self, map: &Map) -> Coverage {
        let (cols, rows) = self.pyramid.tiles_across(self.pyramid.max_level());
        let total = u64::from(cols) * u64::from(rows);

        let (Ok(cols_i), Ok(rows_i)) = (i32::try_from(cols), i32::try_from(rows)) else {
            return Coverage {
                tiles: total,
                total,
                bounds: TileRect::new(0, 0, cols, rows),
            };
        };

        let mut marked = Bitmap::new(cols as usize * rows as usize);
        let mut bounds: Option<(i32, i32, i32, i32)> = None;

        let cell_squares = map.geometry().cell_squares().cast_signed();
        let margin = self.layout.margin();
        let (cpt, rpt) = (self.layout.cols_per_tile(), self.layout.rows_per_tile());
        let origin_col = self.layout.origin_col();
        let origin_row = self.layout.origin_row() + GRID_ROWS_PER_LAYER * self.layer;

        for cell in map.cells() {
            // A cell with nothing on this layer contributes no tiles.
            if !cell.has_layer(self.layer) {
                continue;
            }

            let grid = cell_grid_bounds(cell.cell_x, cell.cell_y, cell_squares);

            // Tile `col` draws grid columns `o + cpt*col + left` to `o + cpt*col + cpt + right`,
            // inclusive. It touches this cell when that range overlaps the cell's columns. Rows
            // work the same way.
            let col_lo =
                div_ceil_i32(grid.left - margin.right - cpt - origin_col, cpt).clamp(0, cols_i - 1);
            let col_hi =
                div_floor_i32(grid.right - margin.left - origin_col, cpt).clamp(0, cols_i - 1);
            let row_lo =
                div_ceil_i32(grid.top - margin.bottom - rpt - origin_row, rpt).clamp(0, rows_i - 1);
            let row_hi =
                div_floor_i32(grid.bottom - margin.top - origin_row, rpt).clamp(0, rows_i - 1);

            bounds = Some(match bounds {
                None => (col_lo, row_lo, col_hi, row_hi),
                Some((l, t, r, b)) => (l.min(col_lo), t.min(row_lo), r.max(col_hi), b.max(row_hi)),
            });

            // The bounds were clamped to the level above, so they are never negative here.
            let stride = cols as usize;
            for row in row_lo.max(0).cast_unsigned()..=row_hi.max(0).cast_unsigned() {
                for col in col_lo.max(0).cast_unsigned()..=col_hi.max(0).cast_unsigned() {
                    marked.set(row as usize * stride + col as usize);
                }
            }
        }

        let bounds = match bounds {
            Some((l, t, r, b)) => TileRect::new(
                l.max(0).cast_unsigned(),
                t.max(0).cast_unsigned(),
                (r - l + 1).max(0).cast_unsigned(),
                (b - t + 1).max(0).cast_unsigned(),
            ),
            None => TileRect::new(0, 0, 0, 0),
        };

        Coverage {
            tiles: marked.count(),
            total,
            bounds,
        }
    }
}

/// How many native tiles might show part of the map. See [`RenderPlan::coverage`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Coverage {
    /// Native tiles that might show part of the map. A render writes at most this many.
    pub tiles: u64,
    /// All native tiles in the level. On most maps, most of them are empty.
    pub total: u64,
    /// The smallest rectangle holding every covered tile. It can be used as a
    /// [`RenderJob::region`].
    pub bounds: TileRect,
}

impl Coverage {
    /// Roughly how many tiles a whole-map pyramid writes, counting every level.
    ///
    /// Each level has a quarter as many tiles as the one below it, so the upper levels add about
    /// a third to the native count.
    #[must_use]
    pub const fn pyramid_tiles(&self) -> u64 {
        self.tiles + self.tiles / 3
    }

    /// The covered tiles as a fraction of the whole level, from 0.0 to 1.0.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.tiles as f64 / self.total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parent_rect_covers_every_child() {
        for (child, expected) in [
            (TileRect::new(0, 0, 1, 1), TileRect::new(0, 0, 1, 1)),
            (TileRect::new(1, 1, 1, 1), TileRect::new(0, 0, 1, 1)),
            (TileRect::new(2, 2, 2, 2), TileRect::new(1, 1, 1, 1)),
            (TileRect::new(1, 1, 2, 2), TileRect::new(0, 0, 2, 2)),
            (TileRect::new(3, 7, 5, 3), TileRect::new(1, 3, 3, 2)),
        ] {
            assert_eq!(child.parent(), expected, "child {child:?}");
        }
    }

    /// Every child tile's parent must be inside the parent rectangle, including at the edges.
    #[test]
    fn no_child_falls_outside_its_parent_rect() {
        for col in 0..8_u32 {
            for row in 0..8_u32 {
                for cols in 1..6_u32 {
                    for rows in 1..6_u32 {
                        let child = TileRect::new(col, row, cols, rows);
                        let parent = child.parent();

                        for c in col..col + cols {
                            for r in row..row + rows {
                                assert!(
                                    c / 2 >= parent.col
                                        && c / 2 < parent.col + parent.cols
                                        && r / 2 >= parent.row
                                        && r / 2 < parent.row + parent.rows,
                                    "child ({c},{r}) of {child:?} escapes {parent:?}",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_region_shrinks_to_a_single_tile_and_stays_there() {
        let mut region = TileRect::new(1700, 312, 2, 2);
        for _ in 0..32 {
            region = region.parent();
        }
        assert_eq!(region, TileRect::new(0, 0, 1, 1));
    }

    #[test]
    fn the_bitmap_counts_distinct_marks() {
        let mut bitmap = Bitmap::new(200);
        assert_eq!(bitmap.count(), 0);

        bitmap.set(0);
        bitmap.set(63);
        bitmap.set(64);
        bitmap.set(199);
        bitmap.set(63);
        assert_eq!(bitmap.count(), 4);

        // A bit past the end is ignored. It neither panics nor sets another bit.
        bitmap.set(100_000);
        assert_eq!(bitmap.count(), 4);
    }
}

/// A fixed-size set of bits, one per tile.
///
/// It marks tiles with far less memory and work than a `HashSet<(u32, u32)>` would.
struct Bitmap(Box<[u64]>);

impl Bitmap {
    fn new(bits: usize) -> Self {
        Bitmap(vec![0_u64; bits.div_ceil(u64::BITS as usize)].into_boxed_slice())
    }

    #[inline]
    fn set(&mut self, bit: usize) {
        let word = bit / u64::BITS as usize;
        if let Some(slot) = self.0.get_mut(word) {
            *slot |= 1_u64 << (bit % u64::BITS as usize);
        }
    }

    fn count(&self) -> u64 {
        self.0.iter().map(|word| u64::from(word.count_ones())).sum()
    }
}

/// Tile counts, output size and time taken for a render run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderStats {
    /// Tiles drawn, including ones that came out empty.
    pub rendered: u64,
    /// Tiles passed to the sink.
    pub written: u64,
    /// Tiles not written because they came out fully transparent.
    pub empty: u64,
    /// Total encoded bytes passed to the sink.
    pub bytes: u64,
    /// Wall-clock time for the run.
    pub elapsed: Duration,
}

impl RenderStats {
    fn merge(mut self, other: Self) -> Self {
        self.rendered += other.rendered;
        self.written += other.written;
        self.empty += other.empty;
        self.bytes += other.bytes;
        self.elapsed = self.elapsed.max(other.elapsed);
        self
    }
}

/// Draws the single native tile `(col, row)` into `canvas`.
///
/// `canvas` is cleared first. It should be `plan.pyramid().tile_size()` pixels square, or the
/// tile is cut off or padded with transparency.
///
/// # Examples
///
/// ```no_run
/// use pzmap2img::render::{RenderJob, RenderPlan, blend::Canvas, render_tile};
/// # fn demo(map: &pzmap2img::Map) -> Result<(), pzmap2img::RenderError> {
/// let plan = RenderPlan::new(map, &RenderJob::for_layer(0))?;
/// let size = plan.pyramid().tile_size();
/// let mut canvas = Canvas::new(size, size);
/// render_tile(map, &plan, 10, 20, &mut canvas);
/// if !canvas.is_transparent() {
///     // Encode and store the tile.
/// }
/// # Ok(()) }
/// ```
pub fn render_tile(map: &Map, plan: &RenderPlan, col: i32, row: i32, canvas: &mut Canvas) {
    canvas.clear();
    render_tile_onto_cleared(map, plan, col, row, canvas);
}

/// Like [`render_tile`], but skips clearing a canvas that is already transparent.
///
/// Canvases from [`CanvasPool::acquire`] are already zeroed, so clearing them again would only
/// waste time.
pub(crate) fn render_tile_onto_cleared(
    map: &Map,
    plan: &RenderPlan,
    col: i32,
    row: i32,
    canvas: &mut Canvas,
) {
    for sample in plan.layout.squares_for_tile(col, row, plan.layer) {
        // Squares come one grid row at a time, from the back of the scene to the front. The
        // game tiles on each square are already stacked bottom to top, as the game draws them.
        // So drawing in this order needs no sorting.
        for id in map.tiles_at(sample.world_x, sample.world_y, plan.layer) {
            if let Some(sprite) = map.sprite(*id) {
                canvas.draw(&sprite, sample.anchor_x, sample.anchor_y);
            }
        }
    }
}

/// Builds the tiles of `level` inside `region`, each from its four children read back from
/// `store`.
///
/// # Errors
///
/// Returns [`RenderError::Read`] if a child tile cannot be read back, [`RenderError::Encode`] or
/// [`RenderError::Write`] if a new tile cannot be encoded or written, and
/// [`RenderError::Stopped`] if `ctx` stops the run.
#[instrument(skip_all, fields(level))]
pub(crate) fn merge_level<E, S>(
    plan: &RenderPlan,
    level: u32,
    region: TileRect,
    encoder: &E,
    store: &S,
    ctx: RunContext<'_>,
) -> Result<RenderStats, RenderError>
where
    E: TileEncoder,
    S: TileSink + TileSource,
{
    let started = Instant::now();
    let (cols, rows) = plan.pyramid.tiles_across(level);
    let region = region.clamp_to(cols, rows);
    let tile_size = plan.pyramid.tile_size();

    let cols = region.col..region.col.saturating_add(region.cols);
    let stats = (region.row..region.row.saturating_add(region.rows))
        .into_par_iter()
        // One task per tile, not per row. Near the top of the pyramid there are too few rows to
        // keep every thread busy. `map_init` gives each batch of tiles one encode buffer to
        // reuse.
        .flat_map_iter(move |row| cols.clone().map(move |col| (col, row)))
        .map_init(Vec::new, |encoded, (col, row)| {
            ctx.check()?;

            let mut stats = RenderStats::default();
            let address = TileAddress::new(level, col, row);
            stats.rendered = 1;

            let mut children: [Option<Canvas>; 4] = [None, None, None, None];
            for (slot, child) in children.iter_mut().zip(address.children()) {
                *slot = store.read_tile(child).map_err(|source| RenderError::Read {
                    address: child,
                    source: Box::new(source),
                })?;
            }

            let borrowed = [
                children.first().and_then(Option::as_ref),
                children.get(1).and_then(Option::as_ref),
                children.get(2).and_then(Option::as_ref),
                children.get(3).and_then(Option::as_ref),
            ];

            let Some(merged) = merge_children(borrowed, tile_size) else {
                stats.empty = 1;
                return Ok(stats);
            };

            let Some(cropped) = crop_to_level(&merged, &plan.pyramid, address) else {
                stats.empty = 1;
                return Ok(stats);
            };

            stats.bytes = emit(&cropped, address, encoder, store, encoded, ctx)?;
            stats.written = 1;
            Ok(stats)
        })
        .try_reduce(RenderStats::default, |a, b| Ok(a.merge(b)))?;

    Ok(RenderStats {
        elapsed: started.elapsed(),
        ..stats
    })
}

/// How many levels above the native one [`render_pyramid`] builds in memory by default.
///
/// Building a level from tiles read back from the store means reading and decoding every tile
/// of the level below. Most tiles are in the levels just above the native one, because each
/// level has a quarter as many tiles as the one below. Building those few levels in memory skips
/// most of the reading back. Each extra level costs more memory. See
/// [`RenderJob::memory_budget`].
pub const DEFAULT_FUSE_DEPTH: u32 = 4;

/// The shared state for building the in-memory levels.
///
/// Each top-level tile and everything below it is built depth first, so only a few tiles per
/// level are in memory at once.
struct FusedWalk<'a, E, S> {
    map: &'a Map,
    plan: &'a RenderPlan,
    encoders: TileEncoders<'a, E>,
    store: &'a S,
    pool: &'a CanvasPool,
    /// The tiles to build on each level, indexed by `level - root_level`.
    regions: &'a [TileRect],
    /// The highest level built in memory.
    root_level: u32,
    native_level: u32,
    /// Tiles on levels above this one build their four children in parallel. Setting it to
    /// `root_level` turns that off. See [`FusedWalk::tile`].
    parallel_below: u32,
    ctx: RunContext<'a>,
}

impl<'a, E, S> FusedWalk<'a, E, S>
where
    E: TileEncoder,
    S: TileSink,
{
    /// Builds and writes one tile and every tile below it, and returns the tile's pixels.
    ///
    /// Returns `None` if the tile is transparent or outside the job. The parent treats that the
    /// same as a missing tile. On levels above [`FusedWalk::parallel_below`], the four children
    /// are built in parallel with `rayon::join`, so several threads can share one subtree. Below
    /// it, they are built one after another on this thread.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Memory`] if the canvas pool runs out of memory,
    /// [`RenderError::Encode`] or [`RenderError::Write`] if a tile cannot be encoded or written,
    /// and [`RenderError::Stopped`] if `ctx` stops the run.
    fn tile(
        &self,
        address: TileAddress,
        scratch: &mut Vec<u8>,
        stats: &mut RenderStats,
    ) -> Result<Option<PooledCanvas<'a>>, RenderError> {
        // Checked for every tile on every level. Stopping inside an encode is not possible.
        self.ctx.check()?;

        let Some(level_index) = address.level.checked_sub(self.root_level) else {
            return Ok(None);
        };
        let Some(region) = self.regions.get(level_index as usize) else {
            return Ok(None);
        };
        // Outside the job. Skipping it, rather than drawing it, gives the same bytes as building
        // this level from read-back tiles, where an unwritten child counts as transparent.
        if !region.contains(address.col, address.row) {
            return Ok(None);
        }

        let size = self.plan.pyramid.tile_size();
        let mut canvas = self
            .pool
            .acquire(size, size)
            .map_err(|source| RenderError::Memory { address, source })?;

        if address.level == self.native_level {
            // Grid coordinates are signed. A tile index that does not fit in an `i32` cannot be
            // drawn, so it is treated as outside the image.
            let (Ok(col), Ok(row)) = (i32::try_from(address.col), i32::try_from(address.row))
            else {
                return Ok(None);
            };
            // The pool already zeroed the canvas.
            render_tile_onto_cleared(self.map, self.plan, col, row, &mut canvas);
        } else if address.level < self.parallel_below {
            let [c0, c1, c2, c3] = address.children();
            let ((r0, r1), (r2, r3)) = rayon::join(
                || rayon::join(|| self.subtree(c0), || self.subtree(c1)),
                || rayon::join(|| self.subtree(c2), || self.subtree(c3)),
            );
            for (quadrant, result) in [r0, r1, r2, r3].into_iter().enumerate() {
                let (child_stats, child) = result?;
                *stats = std::mem::take(stats).merge(child_stats);
                // Each child is merged and dropped before the next, so its buffer goes back to
                // the pool as early as possible. The memory budget still allows for all four
                // being held at once.
                if let Some(child) = child {
                    halve_into(&child, &mut canvas, quadrant);
                }
            }
        } else {
            for (quadrant, child) in address.children().into_iter().enumerate() {
                // The child's buffer goes back to the pool at the end of this iteration. So this
                // thread holds one child at a time, plus one partly built tile per level.
                if let Some(child) = self.tile(child, scratch, stats)? {
                    halve_into(&child, &mut canvas, quadrant);
                }
            }
        }

        stats.rendered += 1;
        if canvas.is_transparent() {
            stats.empty += 1;
            return Ok(None);
        }

        let encoder = if address.level == self.native_level {
            self.encoders.native
        } else {
            self.encoders.upper
        };

        let Some(cropped) = crop_to_level(&canvas, &self.plan.pyramid, address) else {
            stats.empty += 1;
            return Ok(None);
        };
        let (kept_width, kept_height) = (cropped.width(), cropped.height());
        stats.bytes += emit(&cropped, address, encoder, self.store, scratch, self.ctx)?;
        stats.written += 1;
        drop(cropped);

        // The parent must get exactly what it would have read back: the stored tile, padded
        // with transparency. Only tiles on the right and bottom edges were cropped, so for every
        // other tile this does nothing.
        clear_outside(&mut canvas, kept_width, kept_height);

        Ok(Some(canvas))
    }

    /// Builds one child as a separate task, for the parallel case of [`FusedWalk::tile`].
    ///
    /// The child may run on another thread, so it gets its own encode buffer and stats instead
    /// of sharing the caller's.
    fn subtree(
        &self,
        address: TileAddress,
    ) -> Result<(RenderStats, Option<PooledCanvas<'a>>), RenderError> {
        let mut scratch = Vec::new();
        let mut stats = RenderStats::default();
        let canvas = self.tile(address, &mut scratch, &mut stats)?;
        Ok((stats, canvas))
    }
}

/// Zeroes everything outside the top-left `width` x `height` of a canvas.
fn clear_outside(canvas: &mut Canvas, width: u32, height: u32) {
    if width >= canvas.width() && height >= canvas.height() {
        return;
    }

    let stride = canvas.width() as usize * BPP;
    let keep = width as usize * BPP;
    let rows = canvas.height() as usize;
    let kept_rows = height as usize;

    for row in 0..rows {
        let start = row * stride;
        let from = if row < kept_rows { start + keep } else { start };
        if let Some(slice) = canvas.pixels_mut().get_mut(from..start + stride) {
            slice.fill(0);
        }
    }
}

/// Builds the native level and `depth` levels above it in memory, depth first.
///
/// Each parent tile is built from its children's pixels while they are still in memory. That
/// skips reading tiles back, and with a lossy encoder it avoids compressing the same pixels
/// twice. A child's buffer goes back to the pool as soon as it is merged into its parent.
///
/// # Errors
///
/// Returns [`RenderError::Memory`] if the canvas pool runs out of memory,
/// [`RenderError::Encode`] or [`RenderError::Write`] if a tile cannot be encoded or written, and
/// [`RenderError::Stopped`] if `ctx` stops the run.
// Every argument is a separate input, and bundling them would only move the list elsewhere.
#[allow(clippy::too_many_arguments)]
#[instrument(skip_all, fields(depth, budget = pool.budget()))]
pub(crate) fn render_fused<E, S>(
    map: &Map,
    plan: &RenderPlan,
    region: TileRect,
    depth: u32,
    encoders: TileEncoders<'_, E>,
    store: &S,
    pool: &CanvasPool,
    ctx: RunContext<'_>,
) -> Result<RenderStats, RenderError>
where
    E: TileEncoder,
    S: TileSink,
{
    let started = Instant::now();
    let native_level = plan.pyramid.max_level();
    let root_level = native_level.saturating_sub(depth);

    // The tiles to build on each level. Start from the native region and take its parent
    // rectangle one level at a time, then reverse so the list starts at `root_level`.
    let mut regions = Vec::with_capacity(depth as usize + 1);
    regions.push(region);
    let mut current = region;
    for level in (root_level..native_level).rev() {
        let (cols, rows) = plan.pyramid.tiles_across(level);
        current = current.parent().clamp_to(cols, rows);
        regions.push(current);
    }
    regions.reverse();

    let Some(roots) = regions.first().copied() else {
        return Ok(RenderStats::default());
    };

    // A small map may have fewer than four top-level tiles per thread, which leaves threads
    // idle. In that case, the top levels build their children in parallel too, down to the
    // first level with enough tiles. That needs the extra memory from
    // `parallel_descent_surcharge`. Without it, each subtree stays on one thread.
    let threads = rayon::current_num_threads();
    let fan_out_target = u64::try_from(threads).unwrap_or(u64::MAX).saturating_mul(4);
    let affordable = pool.budget()
        >= CanvasPool::budget_for(threads, plan.pyramid.tile_size(), depth).saturating_add(
            CanvasPool::parallel_descent_surcharge(threads, plan.pyramid.tile_size()),
        );
    let parallel_below = if affordable {
        let sparse_levels = regions
            .iter()
            .take_while(|region| region.tiles() < fan_out_target)
            .count();
        root_level.saturating_add(u32::try_from(sparse_levels).unwrap_or(u32::MAX))
    } else {
        root_level
    };
    if parallel_below > root_level {
        debug!(
            parallel_below,
            root_level, "descending the sparse top levels in parallel"
        );
    }

    let walk = FusedWalk {
        map,
        plan,
        encoders,
        store,
        pool,
        regions: &regions,
        root_level,
        native_level,
        parallel_below,
        ctx,
    };

    let cols = roots.col..roots.col.saturating_add(roots.cols);
    let stats = (roots.row..roots.row.saturating_add(roots.rows))
        .into_par_iter()
        // One task per subtree, not per row of subtrees. A subtree is thousands of tiles of
        // work, so the cost of scheduling it is small. Small tasks also spread the work evenly
        // when some parts of the map are much busier than others.
        .flat_map_iter(move |row| cols.clone().map(move |col| (col, row)))
        .map(|(col, row)| {
            let mut scratch = Vec::new();
            let mut stats = RenderStats::default();
            walk.tile(
                TileAddress::new(root_level, col, row),
                &mut scratch,
                &mut stats,
            )?;
            Ok(stats)
        })
        .try_reduce(RenderStats::default, |a, b| Ok(a.merge(b)))?;

    debug!(
        high_water = pool.high_water(),
        resident = pool.resident(),
        allocations = pool.allocations(),
        "fused levels done"
    );

    Ok(RenderStats {
        elapsed: started.elapsed(),
        ..stats
    })
}

/// Renders every level of the pyramid for `job` and writes the tiles to `store`.
///
/// The native level and the [`RenderJob::fuse_depth`] levels above it are built in memory. Each
/// level above those is built from tiles read back through [`TileSource`]. If the job has a
/// [`RenderJob::region`], each upper level builds only the tiles that cover the region.
///
/// Rendering uses rayon's current thread pool. Fully transparent tiles are not written.
///
/// The pyramid also needs a `.dzi` metadata file. Write it with
/// [`DziStore::write_descriptor`] after the render.
///
/// # Examples
///
/// Render layer 0 as WebP tiles, counting them as they are written:
///
/// ```no_run
/// # #[cfg(feature = "webp")]
/// # fn demo(map: &pzmap2img::Map) -> Result<(), Box<dyn std::error::Error>> {
/// use pzmap2img::render::{
///     DziStore, RenderJob, RenderPlan, RunContext, TileEncoders, TileReport, WebpEncoder,
///     render_pyramid,
/// };
/// use std::sync::atomic::{AtomicU64, Ordering};
///
/// let job = RenderJob::for_layer(0);
/// let store = DziStore::new("/tmp/out/layer0_files", "webp");
/// let encoder = WebpEncoder::default();
///
/// let done = AtomicU64::new(0);
/// let on_tile = |_: &TileReport| {
///     done.fetch_add(1, Ordering::Relaxed);
/// };
/// let ctx = RunContext {
///     on_tile: Some(&on_tile),
///     ..RunContext::none()
/// };
///
/// render_pyramid(map, &job, TileEncoders::uniform(&encoder), &store, ctx)?;
/// store.write_descriptor(RenderPlan::new(map, &job)?.pyramid())?;
/// # Ok(()) }
/// ```
///
/// # Errors
///
/// Returns:
///
/// - [`RenderError::FormatMismatch`] if the two encoders produce different formats.
/// - Any error from [`RenderPlan::new`].
/// - [`RenderError::MemoryBudget`] if [`RenderJob::memory_budget`] is too small.
/// - [`RenderError::Prepare`] if [`TileSink::prepare`] fails.
/// - [`RenderError::Memory`] if the in-memory levels run out of tile memory.
/// - [`RenderError::Read`] if a tile cannot be read back.
/// - [`RenderError::Encode`] or [`RenderError::Write`] if a tile cannot be encoded or written.
/// - [`RenderError::Stopped`] if `ctx` stops the run.
#[instrument(skip_all, fields(layer = job.layer))]
pub fn render_pyramid<E, S>(
    map: &Map,
    job: &RenderJob,
    encoders: TileEncoders<'_, E>,
    store: &S,
    ctx: RunContext<'_>,
) -> Result<RenderStats, RenderError>
where
    E: TileEncoder,
    S: TileSink + TileSource,
{
    if encoders.native.format() != encoders.upper.format() {
        return Err(RenderError::FormatMismatch {
            native: encoders.native.format().to_owned(),
            upper: encoders.upper.format().to_owned(),
        });
    }

    let plan = RenderPlan::new(map, job)?;
    let region = plan.region_of(job);
    let (width, height) = plan.pyramid.canonical();

    info!(
        width,
        height,
        max_level = plan.pyramid.max_level(),
        tiles = region.tiles(),
        "rendering"
    );

    // Called once before the first tile. `DziStore` creates its level directories here, so
    // writes don't have to check for them.
    let levels = plan.pyramid.max_level() + 1;
    store
        .prepare(levels)
        .map_err(|source| RenderError::Prepare {
            levels,
            source: Box::new(source),
        })?;

    let native_level = plan.pyramid.max_level();
    let fused_root = plan.read_back_level(job);
    let depth = native_level - fused_root;

    let threads = rayon::current_num_threads();
    // The default budget includes extra room so a small map's top levels can use every thread.
    // The pool only allocates that memory if the walk actually uses it.
    let budget = job.memory_budget.unwrap_or_else(|| {
        CanvasPool::budget_for(threads, plan.pyramid.tile_size(), depth).saturating_add(
            CanvasPool::parallel_descent_surcharge(threads, plan.pyramid.tile_size()),
        )
    });
    let pool = CanvasPool::for_walk(threads, plan.pyramid.tile_size(), depth, budget)
        .map_err(RenderError::MemoryBudget)?;

    info!(
        depth,
        threads, budget, "building the bottom levels in memory"
    );
    let mut stats = render_fused(map, &plan, region, depth, encoders, store, &pool, ctx)?;
    debug!(?stats, "fused levels done");

    // Build only the parents of the region, not each whole level. Otherwise a job for a few
    // native tiles would try millions of parent tiles, each with four reads that find nothing.
    let mut parents = region;
    for _ in fused_root..native_level {
        parents = parents.parent();
    }
    for level in (0..fused_root).rev() {
        parents = parents.parent();
        let level_stats = merge_level(&plan, level, parents, encoders.upper, store, ctx)?;
        // Logged per level because each level is smaller and uses fewer threads than the one
        // below. The numbers show how long the last, small levels take.
        debug!(
            level,
            rendered = level_stats.rendered,
            written = level_stats.written,
            empty = level_stats.empty,
            elapsed = ?level_stats.elapsed,
            "merge level done"
        );
        stats = stats.merge(level_stats);
    }

    Ok(stats)
}

/// Encodes one tile, passes it to the sink, and returns its size in bytes.
fn emit<E, S>(
    tile: &Canvas,
    address: TileAddress,
    encoder: &E,
    sink: &S,
    scratch: &mut Vec<u8>,
    ctx: RunContext<'_>,
) -> Result<u64, RenderError>
where
    E: TileEncoder,
    S: TileSink,
{
    scratch.clear();

    // Only read the clock if there is an `on_tile` hook to report to.
    let started = ctx.on_tile.map(|_| Instant::now());
    let encoded = encoder
        .encode(tile, scratch)
        .map_err(|source| RenderError::Encode {
            address,
            source: Box::new(source),
        })?;
    let encode_time = started.map(|started| started.elapsed());

    sink.write_tile(address, scratch)
        .map_err(|source| RenderError::Write {
            address,
            source: Box::new(source),
        })?;

    let bytes = u64::try_from(scratch.len()).unwrap_or(u64::MAX);
    if let (Some(on_tile), Some(encode_time)) = (ctx.on_tile, encode_time) {
        on_tile(&TileReport {
            address,
            width: tile.width(),
            height: tile.height(),
            bytes,
            encode_time,
            encoded,
        });
    }

    Ok(bytes)
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    #[test]
    fn a_render_that_cannot_fit_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        if available_bytes(dir.path()).is_none() {
            return;
        }

        let err = check_free_space(dir.path(), u64::MAX / 2)
            .expect_err("no filesystem holds eight exabytes");

        assert!(
            matches!(err, RenderError::NotEnoughSpace { .. }),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn a_render_that_fits_is_allowed() {
        let dir = tempfile::tempdir().expect("temp dir");
        check_free_space(dir.path(), 1024).expect("a kilobyte fits anywhere that can hold a test");
    }

    /// Checks the ten percent margin. A run that needs exactly the free space is refused, because
    /// the size estimate is a typical value, not an upper bound.
    #[test]
    fn the_margin_refuses_a_run_sized_exactly_to_the_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let Some(available) = available_bytes(dir.path()) else {
            return;
        };

        assert!(
            check_free_space(dir.path(), available).is_err(),
            "a render needing every free byte should be refused"
        );
    }
}

/// End-to-end checks of the render walk on a synthetic map.
#[cfg(test)]
mod walk_tests {
    use super::*;
    use crate::testing::synthetic_map;
    use std::sync::atomic::{
        AtomicU64,
        Ordering,
    };

    fn small_job() -> RenderJob {
        RenderJob {
            tile_size: 256,
            ..RenderJob::for_layer(0)
        }
    }

    /// A `tiles` x `tiles` region around the left corner of the first cell, so some of its tiles
    /// are covered and some are transparent. It starts on a multiple of `tiles`, so it holds
    /// whole subtrees.
    fn edge_region(map: &Map, job: &RenderJob, tiles: u32) -> TileRect {
        let plan = RenderPlan::new(map, job).expect("the synthetic map plans");
        let squares = map.geometry().cell_squares().cast_signed();
        let (col, row) = plan.layout().tile_for_world_square(0, squares - 1, 0);
        let align = |at: i32| at.max(0).cast_unsigned() / tiles * tiles;
        TileRect::new(align(col), align(row), tiles, tiles)
    }

    fn tiles_on_disk(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut found = Vec::new();
        for level in std::fs::read_dir(root).expect("listing levels").flatten() {
            for entry in std::fs::read_dir(level.path())
                .expect("listing tiles")
                .flatten()
            {
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                found.push((relative, std::fs::read(&path).expect("reading a tile")));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    }

    #[test]
    fn a_stopped_run_stops_early() {
        let map = synthetic_map(&[(0, 0)], 8);
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("layer0_files"), "png");
        let encoder = PngEncoder::default();
        let render = |ctx| {
            render_pyramid(
                &map,
                &small_job(),
                TileEncoders::uniform(&encoder),
                &store,
                ctx,
            )
        };

        let never = || false;
        let stats = render(RunContext {
            stop: Some(&never),
            ..RunContext::none()
        })
        .expect("a hook that never fires changes nothing");
        assert!(stats.written > 0, "the baseline wrote nothing to compare");

        let always = || true;
        let err = render(RunContext {
            stop: Some(&always),
            ..RunContext::none()
        })
        .expect_err("a stopped render stops");
        assert!(
            matches!(err, RenderError::Stopped),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn every_written_tile_is_reported() {
        let map = synthetic_map(&[(0, 0)], 8);
        let dir = tempfile::tempdir().expect("temp dir");
        let store = DziStore::new(dir.path().join("layer0_files"), "png");
        let encoder = PngEncoder::default();

        let (tiles, bytes) = (AtomicU64::new(0), AtomicU64::new(0));
        let on_tile = |report: &TileReport| {
            assert_eq!(report.encoded, EncodeReport::single());
            tiles.fetch_add(1, Ordering::Relaxed);
            bytes.fetch_add(report.bytes, Ordering::Relaxed);
        };
        let stats = render_pyramid(
            &map,
            &small_job(),
            TileEncoders::uniform(&encoder),
            &store,
            RunContext {
                on_tile: Some(&on_tile),
                ..RunContext::none()
            },
        )
        .expect("render");

        assert_eq!(tiles.into_inner(), stats.written);
        assert_eq!(bytes.into_inner(), stats.bytes);
    }

    /// This holds only because the encoder is lossless. Reading back decodes each level before
    /// building the next, while the in-memory walk uses the original pixels. With a lossy
    /// encoder, the read-back path would add compression error at every level.
    #[test]
    fn fusing_and_reading_back_build_the_same_pyramid() {
        let map = synthetic_map(&[(0, 0), (1, 0)], 8);
        let region = edge_region(&map, &small_job(), 8);
        let encoder = PngEncoder::default();
        let root = tempfile::tempdir().expect("temp dir");

        let mut outputs = Vec::new();
        for depth in [0_u32, 4] {
            let store = DziStore::new(
                root.path()
                    .join(format!("depth{depth}"))
                    .join("layer0_files"),
                encoder.format(),
            );
            let job = small_job().with_fuse_depth(depth).with_region(region);
            let stats = render_pyramid(
                &map,
                &job,
                TileEncoders::uniform(&encoder),
                &store,
                RunContext::none(),
            )
            .expect("render");
            outputs.push((stats, tiles_on_disk(store.root())));
        }

        let [(read_back_stats, read_back), (fused_stats, fused)] = outputs.as_slice() else {
            panic!("two renders");
        };

        assert!(!fused.is_empty(), "the region rendered nothing at all");
        assert_eq!(read_back, fused);
        assert_eq!(read_back_stats.written, fused_stats.written);
        assert_eq!(read_back_stats.empty, fused_stats.empty);
        assert_eq!(read_back_stats.bytes, fused_stats.bytes);
    }

    /// The memory the walk actually holds must stay within the budget computed from the thread
    /// count, tile size and fuse depth.
    #[test]
    fn the_fused_walk_stays_inside_its_memory_bound() {
        let map = synthetic_map(&[(0, 0)], 8);
        let job = small_job();
        let plan = RenderPlan::new(&map, &job).expect("the synthetic map plans");
        let region = edge_region(&map, &job, 16);

        let depth = 4;
        let threads = rayon::current_num_threads();
        let tile_size = plan.pyramid().tile_size();
        let budget = CanvasPool::budget_for(threads, tile_size, depth);
        let pool = CanvasPool::for_walk(threads, tile_size, depth, budget).expect("exact budget");

        let dir = tempfile::tempdir().expect("temp dir");
        let encoder = PngEncoder::default();
        let store = DziStore::new(dir.path().join("layer0_files"), encoder.format());

        let stats = render_fused(
            &map,
            &plan,
            region,
            depth,
            TileEncoders::uniform(&encoder),
            &store,
            &pool,
            RunContext::none(),
        )
        .expect("fused render");

        assert!(stats.written > 0, "the region rendered nothing at all");
        assert!(pool.resident() <= budget);
        assert!(
            pool.high_water() <= budget,
            "live set {} exceeded the {budget} byte bound",
            pool.high_water()
        );
        // Buffers must be reused: one allocation per buffer held at once, not one per tile.
        let live = (u64::from(depth) + 2) * threads as u64;
        assert!(
            pool.allocations() <= live,
            "{} allocations for a live set of at most {live}",
            pool.allocations(),
        );
    }
}
