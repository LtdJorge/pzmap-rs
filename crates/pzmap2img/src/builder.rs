//! The build phase: reads a map's files and builds the in-memory map data.
//!
//! [`build`] runs four stages in order:
//!
//! 1. **Discover** finds each cell's `{x}_{y}.lotheader` and `world_{x}_{y}.lotpack` and pairs
//!    them up.
//! 2. **Headers** parses every lotheader. It collects the tile names each cell uses into one
//!    shared table. With [`BuildTargets::vector`] set, it also collects rooms, buildings, room
//!    objects and zombie spawn density (zpop).
//! 3. **Textures** loads the sprites for the tile names found in stage 2.
//! 4. **Cells** parses every lotpack into a compact per-block form. With
//!    [`BuildTargets::contents`] set, it also counts the container sprites in each room.
//!
//! [`BuildTargets::vector`]: crate::runtime::BuildTargets::vector
//! [`BuildTargets::contents`]: crate::runtime::BuildTargets::contents

use crate::{
    budget::{
        BuildBudget,
        BuildCtx,
    },
    config::{
        BuildConfig,
        BuildLimits,
        InputTrust,
    },
    contents::{
        self,
        CellContents,
        ItemIndex,
        RoomLookup,
        SortedContents,
    },
    error::{
        BuildError,
        Result,
    },
    runtime::{
        raster::{
            self,
            CellBuilder,
            CellData,
            CellGeometry,
        },
        size::HeapSize as _,
        vector::{
            self,
            AnnotationParts,
            Annotations,
            CellAnnotations,
            ItemKind,
        },
    },
    source::SourceBytes,
    string_table::{
        RoomNameId,
        RoomNames,
        StringTableBuilder,
        StringTableReader,
        TileId,
        TileNames,
    },
    textures::{
        LoadedPack,
        TextureStats,
        build_texture_library,
    },
};
use ahash::AHashMap;
use libpzmap::{
    B41,
    B42,
    Build,
    VersionMetadata,
    lotheader::LotHeader,
    lotpack::Cell,
    sidecar::tiledefs::TileDefs,
};
use rayon::prelude::*;
use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
    time::{
        Duration,
        Instant,
    },
};
use tracing::{
    info,
    instrument,
    warn,
};

/// A cell's two files, located during discovery.
#[derive(Clone, Debug)]
pub(crate) struct CellPaths {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) header: PathBuf,
    pub(crate) lotpack: PathBuf,
}

/// What stage 2 produces for one cell.
struct HeaderOutput {
    x: i32,
    y: i32,
    /// The cell's lotpack. Kept here because once a cell is skipped, the header list no longer
    /// lines up with the discovered cells by index.
    lotpack: PathBuf,
    min_layer: i32,
    max_layer: i32,
    /// Maps each index into this cell's own tile name table to the shared [`TileId`]. Lotpacks
    /// refer to tiles by the cell-local index.
    remap: Box<[TileId]>,
    annotations: Option<CellAnnotationParts>,
}

/// One cell's annotation data, with indices still local to the cell.
///
/// [`assemble_annotations`] merges these into the shared arrays.
struct CellAnnotationParts {
    rooms: Vec<vector::Room>,
    rectangles: Vec<vector::Rectangle>,
    objects: Vec<vector::Object>,
    buildings: Vec<std::ops::Range<u32>>,
    zpop: Vec<u8>,
}

/// The result of [`build`], with counts and timings for reporting.
#[non_exhaustive]
pub struct BuildOutput {
    /// The tiles and sprites for bitmap rendering. Present when [`BuildTargets::raster`] was set.
    ///
    /// [`BuildTargets::raster`]: crate::runtime::BuildTargets::raster
    pub raster: Option<raster::Map>,
    /// Rooms, buildings and zpop. Present when [`BuildTargets::vector`] or
    /// [`BuildTargets::contents`] was set.
    ///
    /// [`BuildTargets::vector`]: crate::runtime::BuildTargets::vector
    /// [`BuildTargets::contents`]: crate::runtime::BuildTargets::contents
    pub annotations: Option<Annotations>,
    /// Counts from building the texture library. Present when the raster target was built.
    pub texture_stats: Option<TextureStats>,
    /// The number of cells found in the map directory.
    pub cells_discovered: usize,
    /// Cells that failed to parse and were left out, within the limits set in
    /// [`BuildLimits`].
    pub skipped_cells: Vec<SkippedCell>,
    /// The most memory the build held at once, in bytes. It counts the large allocations the
    /// build tracks, but not the memory of the tile and room name tables.
    pub peak_bytes: u64,
    /// The part of `peak_bytes` used by the texture library.
    pub texture_bytes: u64,
    /// How long each stage took.
    pub timings: StageTimings,
}

/// How long each build stage took, in wall-clock time.
#[derive(Copy, Clone, Debug, Default)]
#[non_exhaustive]
pub struct StageTimings {
    /// Finding and pairing the cell files.
    pub discover: Duration,
    /// Parsing the lotheaders.
    pub headers: Duration,
    /// Building the texture library.
    pub textures: Duration,
    /// Parsing the lotpacks.
    pub cells: Duration,
    /// The whole build.
    pub total: Duration,
}

/// Reads the map and texture packs described by `config` and builds the in-memory map data.
///
/// Which outputs are built depends on [`BuildConfig::targets`]. The stages are described in the
/// [module documentation](self).
///
/// # Errors
///
/// Returns a [`BuildError`] if a file can't be read or parsed, if a limit in [`BuildLimits`] is
/// exceeded, or if `config` asks for something that isn't available, such as room contents
/// without tile definitions. A cell that fails to parse is skipped instead when the skip limits
/// allow it, and listed in [`BuildOutput::skipped_cells`].
///
/// # Examples
///
/// ```no_run
/// use pzmap2img::{BuildConfig, BuildTargets, builder};
///
/// let media = std::path::Path::new("/path/to/ProjectZomboid/media");
/// let config = BuildConfig::new(
///     media.join("maps/Muldraugh, KY"),
///     vec![media.join("texturepacks")],
/// )
/// .with_targets(BuildTargets::RASTER);
///
/// let output = builder::build(&config)?;
/// println!("{} cells", output.cells_discovered);
/// # Ok::<(), pzmap2img::BuildError>(())
/// ```
#[instrument(skip_all, fields(map = %config.map_path.display()))]
pub fn build(config: &BuildConfig) -> Result<BuildOutput> {
    // `VersionMetadata` has associated constants, so it can't be a trait object. The match
    // picks one of two versions of `build_with`, compiled once per game build.
    match config.game_build {
        Build::B41 => build_with::<B41>(config),
        Build::B42 => build_with::<B42>(config),
        other => Err(BuildError::UnsupportedBuild {
            build: other.name(),
        }),
    }
}

/// [`build`], with the game build chosen.
fn build_with<V: VersionMetadata>(config: &BuildConfig) -> Result<BuildOutput> {
    let started = Instant::now();
    let mut timings = StageTimings::default();

    let budget = BuildBudget::new(config.limits.max_build_bytes);
    let ctx = BuildCtx::new(&config.limits, &budget);

    let stage = Instant::now();
    let cells = discover_cells(&config.map_path)?;
    timings.discover = stage.elapsed();
    info!(cells = cells.len(), "discovered");

    let tile_names = StringTableBuilder::<TileNames>::new();
    let room_names = StringTableBuilder::<RoomNames>::new();

    let stage = Instant::now();
    let header_outcomes = parse_headers::<V>(
        &cells,
        &tile_names,
        &room_names,
        config.targets.wants_vector(),
        config.input_trust,
        &config.limits,
    )?;
    let (headers, mut skipped_cells) =
        enforce_skip_budget(header_outcomes, cells.len(), &config.limits)?;
    timings.headers = stage.elapsed();

    let tile_reader = tile_names.into_reader();
    info!(
        tile_names = tile_reader.len(),
        room_names = room_names.len(),
        "interned"
    );

    let item_index = config
        .targets
        .contents
        .then(|| load_item_index(config, &tile_reader))
        .transpose()?;

    let (textures, texture_stats) = if config.targets.raster {
        let stage = Instant::now();
        let (textures, stats) = load_textures(config, &tile_reader, ctx)?;
        timings.textures = stage.elapsed();
        info!(
            blob_mb = stats.blob_bytes as f64 / 1e6,
            textures = stats.textures,
            missing = stats.missing,
            pages = format!("{}/{}", stats.pages_decoded, stats.pages_total),
            slack_mb = stats.slack_bytes as f64 / 1e6,
            "texture library built"
        );
        (Some(textures), Some(stats))
    } else {
        (None, None)
    };

    // One pass over the lotpacks serves both the raster target and the room contents.
    let mut cell_data = Vec::new();
    let mut cell_contents: AHashMap<(i32, i32), SortedContents> = AHashMap::new();
    if config.targets.raster || item_index.is_some() {
        let stage = Instant::now();
        let cell_outcomes =
            parse_cells::<V>(&headers, ctx, item_index.as_ref(), config.targets.raster)?;
        let (parsed, skipped_here) =
            enforce_skip_budget(cell_outcomes, cells.len(), &config.limits)?;
        skipped_cells.extend(skipped_here);
        timings.cells = stage.elapsed();

        cell_data.reserve(parsed.len());
        for cell in parsed {
            if let Some(data) = cell.data {
                cell_data.push(data);
            }
            if let Some(contents) = cell.contents {
                cell_contents.insert((cell.x, cell.y), contents.into_sorted());
            }
        }
    }

    let annotations = if config.targets.wants_vector() {
        let kinds = item_index.as_ref().map_or(&[][..], ItemIndex::kinds);
        Some(assemble_annotations::<V>(
            &headers,
            room_names.build(),
            &cell_contents,
            kinds,
        )?)
    } else {
        None
    };

    let raster_map = textures.map(|textures| {
        raster::Map::new(
            cell_data,
            CellGeometry::of::<V>(),
            textures,
            tile_reader.build(),
        )
    });

    timings.total = started.elapsed();
    info!(
        discover_ms = timings.discover.as_millis(),
        headers_ms = timings.headers.as_millis(),
        textures_ms = timings.textures.as_millis(),
        cells_ms = timings.cells.as_millis(),
        total_ms = timings.total.as_millis(),
        "build phase complete"
    );

    Ok(BuildOutput {
        raster: raster_map,
        annotations,
        texture_stats,
        cells_discovered: cells.len(),
        skipped_cells,
        peak_bytes: budget.high_water(),
        texture_bytes: texture_stats.map_or(0, |stats| stats.blob_bytes as u64),
        timings,
    })
}

/// Stage 1. Pairs up `{x}_{y}.lotheader` with `world_{x}_{y}.lotpack`.
#[instrument(skip_all)]
pub(crate) fn discover_cells(map_path: &Path) -> Result<Vec<CellPaths>> {
    let mut headers: BTreeMap<(i32, i32), PathBuf> = BTreeMap::new();
    let mut lotpacks: BTreeMap<(i32, i32), PathBuf> = BTreeMap::new();

    let io = |source| BuildError::Io {
        op: "reading map directory",
        path: map_path.to_path_buf(),
        source,
    };

    for entry in std::fs::read_dir(map_path).map_err(io)? {
        let path = entry.map_err(io)?.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if let Some(stem) = name.strip_suffix(".lotheader") {
            if let Some(coords) = parse_coords(stem) {
                headers.insert(coords, path);
            }
        } else if let Some(stem) = name.strip_suffix(".lotpack")
            && let Some(stem) = stem.strip_prefix("world_")
            && let Some(coords) = parse_coords(stem)
        {
            lotpacks.insert(coords, path);
        }
    }

    let mut cells = Vec::with_capacity(headers.len());
    for ((x, y), header) in headers {
        match lotpacks.remove(&(x, y)) {
            Some(lotpack) => cells.push(CellPaths {
                x,
                y,
                header,
                lotpack,
            }),
            None => warn!(x, y, "lotheader with no matching lotpack, skipping cell"),
        }
    }

    if !lotpacks.is_empty() {
        warn!(
            count = lotpacks.len(),
            "lotpacks with no matching lotheader"
        );
    }

    Ok(cells)
}

/// Parses a file name stem such as `"49_6"` into cell coordinates `(49, 6)`.
///
/// Accepts only ASCII digits on each side. `str::parse::<i32>` alone would also accept a
/// leading `-` and give a negative coordinate.
fn parse_coords(stem: &str) -> Option<(i32, i32)> {
    let (x, y) = stem.split_once('_')?;

    let parse = |s: &str| -> Option<i32> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    };

    Some((parse(x)?, parse(y)?))
}

/// The result of one stage for one cell: the parsed value, or the reason the cell was skipped.
///
/// A skip is a value rather than an `Err` because collecting a parallel iterator into a
/// `Result` stops at the first error and throws away every cell that already succeeded.
enum CellOutcome<T> {
    Built(T),
    Skipped(SkippedCell),
}

/// A cell left out of the build because its lotheader or lotpack could not be parsed.
#[derive(Debug)]
#[non_exhaustive]
pub struct SkippedCell {
    /// The cell's x coordinate, in cells.
    pub x: i32,
    /// The cell's y coordinate, in cells.
    pub y: i32,
    /// The parsing stage that failed.
    pub stage: CellStage,
    /// Why it failed.
    pub reason: BuildError,
}

/// The per-cell parsing stage a [`SkippedCell`] failed in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CellStage {
    /// Parsing the cell's `.lotheader`.
    Header,
    /// Parsing the cell's `.lotpack`.
    Lotpack,
}

impl CellStage {
    /// A short, stable name for logs: `"header"` or `"lotpack"`.
    pub const fn name(self) -> &'static str {
        match self {
            CellStage::Header => "header",
            CellStage::Lotpack => "lotpack",
        }
    }
}

impl std::fmt::Display for CellStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Returns `count * fraction`, rounded down.
///
/// The fraction is clamped to `0.0..=1.0` first, and a NaN or infinite fraction gives 0. The
/// result is therefore always between 0 and `count`, so the cast back to `usize` is safe.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fraction_of(count: usize, fraction: f64) -> usize {
    if !fraction.is_finite() || fraction <= 0.0 {
        return 0;
    }

    let scaled = count as f64 * fraction.min(1.0);

    // A large `count` may not be exact as an `f64`, and rounding can push `scaled` above it.
    if scaled >= count as f64 {
        count
    } else {
        scaled as usize
    }
}

/// Splits the outcomes into built values and skipped cells.
///
/// # Errors
///
/// Returns [`BuildError::TooManyCellsSkipped`] if more cells were skipped than `limits` allows.
/// The default limits allow none, so with them any skipped cell fails the build.
fn enforce_skip_budget<T>(
    outcomes: Vec<CellOutcome<T>>,
    discovered: usize,
    limits: &BuildLimits,
) -> Result<(Vec<T>, Vec<SkippedCell>)> {
    let mut built = Vec::with_capacity(outcomes.len());
    let mut skipped = Vec::new();

    for outcome in outcomes {
        match outcome {
            CellOutcome::Built(value) => built.push(value),
            CellOutcome::Skipped(cell) => skipped.push(cell),
        }
    }

    if skipped.is_empty() {
        return Ok((built, skipped));
    }

    let by_fraction = fraction_of(discovered, limits.max_skipped_fraction);
    let allowed = usize::try_from(limits.max_skipped_cells)
        .unwrap_or(usize::MAX)
        .max(by_fraction);

    if skipped.len() > allowed {
        for cell in skipped.iter().take(5) {
            warn!(x = cell.x, y = cell.y, stage = cell.stage.name(), reason = %cell.reason, "skipped");
        }

        return Err(BuildError::TooManyCellsSkipped {
            skipped: skipped.len(),
            discovered,
            allowed,
        });
    }

    warn!(
        skipped = skipped.len(),
        discovered, allowed, "carrying on without some cells"
    );

    Ok((built, skipped))
}

/// Stage 2. Parses every lotheader in parallel, adding tile and room names to the shared
/// tables.
#[instrument(skip_all, fields(cells = cells.len()))]
fn parse_headers<V: VersionMetadata>(
    cells: &[CellPaths],
    tile_names: &StringTableBuilder<TileNames>,
    room_names: &StringTableBuilder<RoomNames>,
    collect_vector: bool,
    trust: InputTrust,
    limits: &BuildLimits,
) -> Result<Vec<CellOutcome<HeaderOutput>>> {
    cells
        .par_iter()
        .map(|cell| {
            let bytes = match map_file(&cell.header, trust, limits) {
                Ok(bytes) => bytes,
                Err(reason) if reason.is_cell_local() => {
                    return Ok(CellOutcome::Skipped(SkippedCell {
                        x: cell.x,
                        y: cell.y,
                        stage: CellStage::Header,
                        reason,
                    }));
                }
                Err(reason) => return Err(reason),
            };

            let header = match LotHeader::<V>::parse(cell.x, cell.y, &bytes) {
                Ok(header) => header,
                Err(source) => {
                    let reason = BuildError::Parse {
                        path: cell.header.clone(),
                        source,
                    };

                    if reason.is_cell_local() {
                        return Ok(CellOutcome::Skipped(SkippedCell {
                            x: cell.x,
                            y: cell.y,
                            stage: CellStage::Header,
                            reason,
                        }));
                    }

                    return Err(reason);
                }
            };

            // Tile names are needed for every target. They decide which sprites the texture
            // library loads, and the lotpack's tile indices are resolved through `remap`.
            let mut remap = Vec::with_capacity(header.strings.len());
            for name in header.strings.iter() {
                remap.push(tile_names.intern(name)?);
            }

            let annotations = collect_vector
                .then(|| collect_annotations::<V>(&header, room_names))
                .transpose()?;

            Ok(CellOutcome::Built(HeaderOutput {
                x: cell.x,
                y: cell.y,
                lotpack: cell.lotpack.clone(),
                min_layer: header.min_layer,
                max_layer: header.max_layer,
                remap: remap.into_boxed_slice(),
                annotations,
            }))
        })
        .collect()
}

/// Converts a length to `u32`. Annotation ranges are stored as `u32`.
///
/// # Errors
///
/// Returns [`BuildError::TooManyAnnotations`] if `len` does not fit in a `u32`.
#[inline]
fn span(len: usize, what: &'static str) -> Result<u32> {
    u32::try_from(len).map_err(|_| BuildError::TooManyAnnotations { what, count: len })
}

/// Copies one lotheader's rooms, room rectangles, room objects, buildings and zpop into
/// cell-local arrays.
fn collect_annotations<V: VersionMetadata>(
    header: &LotHeader<'_, V>,
    room_names: &StringTableBuilder<RoomNames>,
) -> Result<CellAnnotationParts> {
    let mut rooms = Vec::with_capacity(header.rooms.len());
    let mut rectangles = Vec::new();
    let mut objects = Vec::new();

    for room in header.rooms.iter() {
        let name: RoomNameId = room_names.intern(room.name)?;

        let rect_start = span(rectangles.len(), "rectangles")?;
        rectangles.extend(room.rectangles.iter().map(|r| vector::Rectangle {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }));

        let obj_start = span(objects.len(), "objects")?;
        objects.extend(room.objects.iter().map(|o| vector::Object {
            object_type: o.kind,
            x: o.x,
            y: o.y,
        }));

        rooms.push(vector::Room {
            name,
            layer: room.layer,
            rectangles: rect_start..span(rectangles.len(), "rectangles")?,
            objects: obj_start..span(objects.len(), "objects")?,
            // Filled in by `assemble_annotations` once the cell's contents are counted.
            items: 0..0,
        });
    }

    // A building's room indices may not be contiguous. Each building stores the range from its
    // lowest to its highest room index, and `assemble_annotations` offsets it.
    let buildings = header
        .buildings
        .iter()
        .map(|b| {
            let min = b.room_indices.iter().copied().min().unwrap_or(0);
            // The index comes from the file, so it can be `u32::MAX`.
            let max = b
                .room_indices
                .iter()
                .copied()
                .max()
                .map_or(0, |m| m.saturating_add(1));
            min..max
        })
        .collect();

    Ok(CellAnnotationParts {
        rooms,
        rectangles,
        objects,
        buildings,
        zpop: header.zpop.clone(),
    })
}

/// Merges every cell's annotation parts into shared arrays and offsets their indices.
///
/// `contents` maps cell coordinates to that cell's sorted `((room, kind), count)` entries.
/// Rooms in a cell that has no entry get no items.
fn assemble_annotations<V: VersionMetadata>(
    headers: &[HeaderOutput],
    names: crate::string_table::StringTable<RoomNames>,
    contents: &AHashMap<(i32, i32), SortedContents>,
    kinds: &[ItemKind],
) -> Result<Annotations> {
    let mut cells = Vec::with_capacity(headers.len());
    let mut rooms = Vec::new();
    let mut rectangles = Vec::new();
    let mut objects = Vec::new();
    let mut buildings = Vec::new();
    let mut zpop = Vec::new();
    let mut items = Vec::new();

    for header in headers {
        let Some(parts) = &header.annotations else {
            continue;
        };

        let room_base = span(rooms.len(), "rooms")?;
        let rect_base = span(rectangles.len(), "rectangles")?;
        let obj_base = span(objects.len(), "objects")?;
        let building_base = span(buildings.len(), "buildings")?;

        let counted = contents
            .get(&(header.x, header.y))
            .map_or(&[][..], Vec::as_slice);
        let item_ranges = contents::items_for_rooms(counted, parts.rooms.len(), &mut items)?;

        rooms.extend(
            parts
                .rooms
                .iter()
                .zip(item_ranges)
                .map(|(r, items)| vector::Room {
                    name: r.name,
                    layer: r.layer,
                    rectangles: r.rectangles.start + rect_base..r.rectangles.end + rect_base,
                    objects: r.objects.start + obj_base..r.objects.end + obj_base,
                    items,
                }),
        );
        rectangles.extend_from_slice(&parts.rectangles);
        objects.extend_from_slice(&parts.objects);
        buildings.extend(
            parts
                .buildings
                .iter()
                .map(|b| b.start + room_base..b.end + room_base),
        );

        let zpop_base = span(zpop.len(), "zpop")?;
        zpop.extend_from_slice(&parts.zpop);

        cells.push(CellAnnotations {
            cell_x: header.x,
            cell_y: header.y,
            rooms: room_base..span(rooms.len(), "rooms")?,
            buildings: building_base..span(buildings.len(), "buildings")?,
            zpop: zpop_base..span(zpop.len(), "zpop")?,
        });
    }

    Ok(Annotations::new(
        AnnotationParts {
            cells,
            rooms,
            rectangles,
            objects,
            buildings,
            zpop,
            zpop_side: V::CELL_SIZE_IN_BLOCKS,
            items,
            item_kinds: kinds.to_vec(),
        },
        names,
    ))
}

/// Reads the tile definitions and classifies every interned tile name against them.
#[instrument(skip_all)]
fn load_item_index(
    config: &BuildConfig,
    tiles: &StringTableReader<TileNames>,
) -> Result<ItemIndex> {
    let path = config
        .tile_defs
        .as_ref()
        .ok_or(BuildError::MissingTileDefs)?;
    let bytes = SourceBytes::open(
        path,
        config.input_trust,
        config.limits.max_input_bytes,
        "tile definitions",
    )?;
    let defs = TileDefs::parse(&bytes).map_err(|source| BuildError::Parse {
        path: path.clone(),
        source,
    })?;

    let index = ItemIndex::build(tiles, &defs);
    info!(
        sheets = defs.sheets().len(),
        item_tiles = index.item_tiles(),
        kinds = index.kinds().len(),
        "tile definitions loaded"
    );
    Ok(index)
}

/// Stage 3. Opens the configured texture packs and builds the texture library.
#[instrument(skip_all)]
fn load_textures(
    config: &BuildConfig,
    tiles: &StringTableReader<TileNames>,
    ctx: BuildCtx<'_>,
) -> Result<(crate::textures::TextureLibrary, TextureStats)> {
    let paths = config.resolve_packs();
    if paths.is_empty() {
        return Err(BuildError::NoTexturePacks {
            searched: config.texture_paths.clone(),
        });
    }

    let packs = paths
        .iter()
        .map(|path| LoadedPack::open(path, config.input_trust, &config.limits))
        .collect::<Result<Vec<_>>>()?;

    build_texture_library(tiles, &packs, config.plants.as_ref(), ctx)
}

/// What stage 4 produces for one lotpack.
///
/// `data` is set when the raster target is built, and `contents` when an [`ItemIndex`] was
/// given.
struct CellParsed {
    x: i32,
    y: i32,
    data: Option<CellData>,
    contents: Option<CellContents>,
}

/// Stage 4. Parses each lotpack in parallel into its runtime form.
///
/// Given an [`ItemIndex`], it also counts the containers in each room. With `build_raster` off,
/// only those counts are collected. They are a few entries per room, so they are not charged
/// against the memory budget.
#[instrument(skip_all, fields(cells = headers.len()))]
fn parse_cells<V: VersionMetadata>(
    headers: &[HeaderOutput],
    ctx: BuildCtx<'_>,
    items: Option<&ItemIndex>,
    build_raster: bool,
) -> Result<Vec<CellOutcome<CellParsed>>> {
    headers
        .par_iter()
        .map(|header| {
            let skip = |reason| {
                Ok(CellOutcome::Skipped(SkippedCell {
                    x: header.x,
                    y: header.y,
                    stage: CellStage::Lotpack,
                    reason,
                }))
            };

            let bytes = match map_lotpack(&header.lotpack, ctx.limits) {
                Ok(bytes) => bytes,
                Err(reason) if reason.is_cell_local() => return skip(reason),
                Err(reason) => return Err(reason),
            };

            // Each tile reference in the parsed cell comes from at least one byte of the file, so
            // the file length is an upper bound on the cell's size. Charging it up front keeps
            // budget accounting out of the per-square loop.
            let reserved = if build_raster { bytes.len() as u64 } else { 0 };
            ctx.charge("cell", reserved)?;

            match build_cell::<V>(header, &bytes, items, build_raster) {
                Ok(cell) => {
                    let actual = cell.data.as_ref().map_or(0, |data| data.heap_size() as u64);
                    ctx.release(reserved.saturating_sub(actual));
                    Ok(CellOutcome::Built(cell))
                }
                Err(reason) if reason.is_cell_local() => {
                    ctx.release(reserved);
                    skip(reason)
                }
                Err(reason) => {
                    ctx.release(reserved);
                    Err(reason)
                }
            }
        })
        .collect()
}

/// Parses one lotpack into its runtime form.
///
/// `Cell::visit` passes each non-empty square as a slice of the mapped file, so the only
/// allocation is the final `CellData`. When `items` is given, the container tiles of each
/// square inside one of the header's rooms are counted too.
fn build_cell<V: VersionMetadata>(
    header: &HeaderOutput,
    bytes: &[u8],
    items: Option<&ItemIndex>,
    build_raster: bool,
) -> Result<CellParsed> {
    let mut builder =
        build_raster.then(|| CellBuilder::new(header.x, header.y, CellGeometry::of::<V>()));
    let remap = &header.remap;

    // Which room a square is in comes from the header's room rectangles. `RoomLookup` explains
    // why the lotpack's own room field is not used.
    let rooms = items.and(header.annotations.as_ref()).map(|parts| {
        RoomLookup::new(
            header.x,
            header.y,
            usize::try_from(V::CELL_SIZE_IN_SQUARES).unwrap_or(0),
            &parts.rooms,
            &parts.rectangles,
        )
    });
    let mut contents = rooms
        .as_ref()
        .filter(|r| !r.is_empty())
        .map(|_| CellContents::default());

    // The lotpack's block index is `block_x * CELL_SIZE_IN_BLOCKS + block_y`. libpzmap has
    // already checked the block count against this build's geometry, so both parts are in range.
    let cell_blocks = V::CELL_SIZE_IN_BLOCKS;
    let block_squares = V::BLOCK_SIZE_IN_SQUARES;

    let global = |index: i32| {
        usize::try_from(index)
            .ok()
            .and_then(|i| remap.get(i))
            .copied()
    };

    Cell::<V>::visit(bytes, header.min_layer, header.max_layer, |at, tiles| {
        let sub_x = (at.block / cell_blocks) * block_squares + at.x;
        let sub_y = (at.block % cell_blocks) * block_squares + at.y;

        if let (Some(contents), Some(items), Some(rooms)) =
            (contents.as_mut(), items, rooms.as_ref())
            && let Some(room) = rooms.room_at(at.layer, sub_x, sub_y)
        {
            contents.record(room, tiles.iter().filter_map(global), items);
        }

        if let Some(builder) = builder.as_mut() {
            builder.push_square(at.layer, sub_x, sub_y, tiles.iter().filter_map(global));
        }
    })
    .map_err(|source| BuildError::Parse {
        path: header.lotpack.clone(),
        source,
    })?;

    Ok(CellParsed {
        x: header.x,
        y: header.y,
        data: builder.map(CellBuilder::build).transpose()?,
        contents,
    })
}

/// Opens a lotheader, memory-mapped or read into memory depending on `trust`.
fn map_file(path: &Path, trust: InputTrust, limits: &BuildLimits) -> Result<SourceBytes> {
    SourceBytes::open(path, trust, limits.max_input_bytes, "map file")
}

/// Opens a lotpack. It is always memory-mapped, whatever `trust` says. See
/// [`SourceBytes::open_mapped`] for why.
fn map_lotpack(path: &Path, limits: &BuildLimits) -> Result<SourceBytes> {
    SourceBytes::open_mapped(path, limits.max_input_bytes, "lotpack")
}

#[cfg(test)]
mod skip_budget_tests {
    use super::*;

    fn skipped(x: i32) -> CellOutcome<u32> {
        CellOutcome::Skipped(SkippedCell {
            x,
            y: 0,
            stage: CellStage::Lotpack,
            reason: BuildError::CellTooLarge {
                cell_x: x,
                cell_y: 0,
                tiles: 9_000_000,
                max: 1_000_000,
            },
        })
    }

    fn limits(cells: u32, fraction: f64) -> BuildLimits {
        BuildLimits {
            max_skipped_cells: cells,
            max_skipped_fraction: fraction,
            ..BuildLimits::default()
        }
    }

    /// One skipped cell must not discard the cells that parsed fine.
    #[test]
    fn good_cells_survive_a_skipped_one() {
        let outcomes = vec![
            CellOutcome::Built(1_u32),
            skipped(2),
            CellOutcome::Built(3),
            CellOutcome::Built(4),
        ];

        let (built, skips) =
            enforce_skip_budget(outcomes, 4, &limits(1, 0.0)).expect("one skip is within budget");

        assert_eq!(built, vec![1, 3, 4]);
        assert_eq!(skips.len(), 1);
        assert_eq!((skips[0].x, skips[0].stage), (2, CellStage::Lotpack));
    }

    /// The default allows no skips, so a vanilla build that starts losing cells fails.
    #[test]
    fn the_default_budget_refuses_any_skip() {
        let outcomes = vec![CellOutcome::Built(1_u32), skipped(2)];

        let err = enforce_skip_budget(outcomes, 2, &BuildLimits::default())
            .expect_err("the default budget is zero");

        match err {
            BuildError::TooManyCellsSkipped {
                skipped,
                discovered,
                allowed,
            } => assert_eq!((skipped, discovered, allowed), (1, 2, 0)),
            other => panic!("expected TooManyCellsSkipped, got {other:?}"),
        }
    }

    #[test]
    fn a_build_with_no_skips_reports_none() {
        let outcomes = vec![CellOutcome::Built(1_u32), CellOutcome::Built(2)];
        let (built, skips) =
            enforce_skip_budget(outcomes, 2, &BuildLimits::default()).expect("no skips");

        assert_eq!(built, vec![1, 2]);
        assert!(skips.is_empty());
    }

    /// The fraction lets one setting work for maps of any size.
    #[test]
    fn the_fraction_allows_more_than_the_count() {
        let outcomes: Vec<CellOutcome<u32>> = (0..10).map(skipped).collect();

        // 10% of 100 discovered is 10, which beats the absolute allowance of 2.
        enforce_skip_budget(outcomes, 100, &limits(2, 0.1)).expect("ten skips fit the fraction");

        let outcomes: Vec<CellOutcome<u32>> = (0..10).map(skipped).collect();
        assert!(
            enforce_skip_budget(outcomes, 20, &limits(2, 0.1)).is_err(),
            "10% of 20 is 2, so ten skips should not fit"
        );
    }

    /// Whichever allowance is larger wins, so setting only one of them still works.
    #[test]
    fn the_count_allows_more_than_the_fraction() {
        let outcomes: Vec<CellOutcome<u32>> = (0..5).map(skipped).collect();
        enforce_skip_budget(outcomes, 10, &limits(5, 0.0)).expect("five skips fit the count");
    }
}

#[cfg(test)]
mod fraction_tests {
    use super::fraction_of;

    #[test]
    fn a_fraction_scales_and_rounds_down() {
        assert_eq!(fraction_of(100, 0.1), 10);
        assert_eq!(fraction_of(20, 0.1), 2);
        assert_eq!(fraction_of(7, 0.5), 3);
        assert_eq!(fraction_of(0, 0.5), 0);
    }

    /// A nonsense fraction must not become a huge allowance.
    #[test]
    fn a_fraction_outside_zero_to_one_is_clamped() {
        assert_eq!(fraction_of(100, 0.0), 0);
        assert_eq!(fraction_of(100, -1.0), 0);
        assert_eq!(fraction_of(100, 5.0), 100);
        assert_eq!(fraction_of(100, f64::NAN), 0);
        assert_eq!(fraction_of(100, f64::INFINITY), 0);
    }

    #[test]
    fn a_full_fraction_allows_everything() {
        assert_eq!(fraction_of(4065, 1.0), 4065);
    }
}
