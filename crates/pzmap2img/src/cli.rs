//! Command-line arguments for programs built on this crate. Needs the `cli` feature.
//!
//! [`SourceArgs`], [`RenderArgs`] and [`EncodingArgs`] are groups of clap arguments. A program
//! adds them to its own commands with `#[command(flatten)]`, and then converts them into the
//! library's settings: [`BuildConfig`], [`RenderJob`] and an encoder.
//!
//! The library's own enums, such as [`Season`] and [`PngCompression`], don't derive clap's
//! `ValueEnum`. This module has its own copies instead, such as [`SeasonArg`] and
//! [`CompressionArg`], so the rest of the library doesn't depend on clap.

#[cfg(feature = "webp")]
use crate::render::WebpEncoder;
use crate::{
    config::{
        BuildConfig,
        BuildLimits,
        InputTrust,
        VANILLA_1X_PACKS,
        VANILLA_2X_PACKS,
    },
    render::{
        BuiltinEncoder,
        DEFAULT_FUSE_DEPTH,
        PngEncoder,
        RenderJob,
        TileRect,
        geometry::IsoScale,
        margins::SpriteExtent,
        store::PngCompression,
    },
    runtime::BuildTargets,
    textures::{
        PlantsConfig,
        Season,
    },
};
use clap::{
    Args as ClapArgs,
    ValueEnum,
};
use libpzmap::Build;
use std::path::PathBuf;

/// Where the game data is, and how to build from it.
///
/// Parse it with clap, or create it with [`SourceArgs::new`], which sets every other option to
/// its default.
#[derive(Clone, Debug, ClapArgs)]
#[non_exhaustive]
pub struct SourceArgs {
    /// The map directory, which holds the `{x}_{y}.lotheader` and `world_{x}_{y}.lotpack` files.
    #[arg(long, short = 'm')]
    pub map: PathBuf,

    /// A directory to search for texture packs. Repeat it to search several, in order.
    #[arg(long = "texture-dir", short = 't', required = true)]
    pub texture_dirs: Vec<PathBuf>,

    /// Which built-in list of texture packs to load.
    #[arg(long, value_enum, default_value_t = PackSetArg::Vanilla2x)]
    pub packs: PackSetArg,

    /// An extra texture pack to load after the built-in list. Its sprites replace any with the
    /// same name. Repeat it to add several.
    #[arg(long = "pack")]
    pub extra_packs: Vec<String>,

    /// Which game build the map files come from. It is not detected from the files.
    #[arg(long, value_enum, default_value_t = GameBuildArg::B42)]
    pub game_build: GameBuildArg,

    /// The most memory the build may use, in MiB. Defaults to 6 GiB. If the build needs more,
    /// it stops with an error that says what the memory was for.
    #[arg(long)]
    pub build_memory_budget: Option<u64>,

    /// How many cells may fail to parse before the build stops. Cells that fail are left out.
    #[arg(long, default_value_t = 0)]
    pub max_skipped_cells: u32,

    /// Read lotheaders and texture packs into memory instead of memory-mapping them. A mapped
    /// file that shrinks while it is read crashes the program. Lotpacks are always memory-mapped,
    /// so don't let other processes change a map's files during the build.
    #[arg(long)]
    pub untrusted_input: bool,

    /// The game's newtiledefinitions.tiles file, used to list the containers in each room.
    /// Defaults to the one next to the first texture directory.
    #[arg(long)]
    pub tile_defs: Option<PathBuf>,

    #[command(flatten)]
    pub plants: PlantsArgs,
}

/// What to render, and how the tiles are laid out.
#[derive(Clone, Debug, ClapArgs)]
#[non_exhaustive]
pub struct RenderArgs {
    /// A layer to render, or a range of layers including both ends. Repeat it to render more.
    /// Each layer gets its own pyramid.
    ///
    /// Layer 0 is the ground, and negative layers are basements. For example, `--layer -3` is
    /// three layers underground, `--layer -17..-1` is every basement from -17 to -1, and
    /// `--layer 0..29` is the ground and every layer above it up to 29. A layer given twice is
    /// rendered once.
    #[arg(
        long = "layer",
        value_name = "LAYER|FIRST..LAST",
        value_parser = parse_layer_span,
        allow_hyphen_values = true,
        default_value = "0"
    )]
    pub layer_spans: Vec<LayerSpan>,

    /// The width and height of each tile, in pixels. Must be a multiple of 64 at 2x, or 32 at
    /// 1x.
    #[arg(long, default_value_t = 1024)]
    pub tile_size: u32,

    /// How many pyramid levels have their tile edges in the same places.
    #[arg(long, default_value_t = 3)]
    pub align_levels: u32,

    /// Render only part of the map, given as COL,ROW,COLS,ROWS in tiles of the most detailed
    /// level.
    #[arg(long, value_parser = parse_region)]
    pub region: Option<TileRect>,

    /// How many levels above the most detailed one to build in memory. Higher levels are built
    /// from tiles read back from disk. 0 reads every level back from disk.
    #[arg(long, default_value_t = DEFAULT_FUSE_DEPTH)]
    pub fuse_depth: u32,

    /// The most memory the in-memory levels may use for tiles, in MiB. It must hold at least
    /// threads * (fuse-depth + 2) tiles. The default is that plus extra room, so a small map's
    /// top levels can still use every thread.
    #[arg(long)]
    pub memory_budget: Option<u64>,

    /// Skip the free disk space check. Use this when the output location reports its free
    /// space wrongly, as some network mounts do.
    #[arg(long = "no-space-check", action = clap::ArgAction::SetFalse)]
    pub space_check: bool,

    /// Use pzmap2dzi's fixed sprite size (384x512 at 2x, 192x256 at 1x) to work out tile
    /// positions, instead of the size of the largest sprite loaded. Only needed to get exactly
    /// the same tile numbering as pzmap2dzi.
    #[arg(long)]
    pub legacy_margins: bool,

    /// The sprite scale of the loaded texture packs. Defaults to the scale of the --packs list.
    /// Only needed with --packs none, which has no list to take it from.
    #[arg(long, value_enum)]
    pub iso_scale: Option<IsoScaleArg>,
}

/// How tiles are encoded.
#[derive(Clone, Debug, ClapArgs)]
#[non_exhaustive]
pub struct EncodingArgs {
    /// The tile image format. WebP files are smaller and faster to encode than PNG at any PNG
    /// setting.
    #[arg(long, value_enum, default_value_t = FormatArg::default())]
    pub format: FormatArg,

    /// WebP quality, from 0 to 100, for lossy tiles. Leave it out for lossless tiles, the
    /// default. A pyramid takes a long time to render, and quality loss is often noticed only
    /// after it is done.
    #[arg(long, value_parser = clap::value_parser!(f32))]
    pub quality: Option<f32>,

    /// How hard the WebP encoder searches for a smaller file, from 0 (fastest) to 6. Only used
    /// with --format webp. The default, 0, is faster than libwebp's default of 4 and gives files
    /// of similar size.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=6))]
    pub webp_method: u8,

    /// How hard lossless WebP encoding searches for a smaller file, from 0 to 100. Lossless
    /// tiles are exact at any value. Lower values encode faster but give larger files.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub lossless_quality: u8,

    /// WebP transparency quality, from 0 to 100, like cwebp -alpha_q. At 100, transparency is
    /// kept exact, so the map's edges stay sharp. Only affects lossy tiles.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub alpha_quality: u8,

    /// Turn off WebP's sharp YUV conversion, which is on by default. Encoding is then faster
    /// and files are smaller, but colors can bleed past the edges of sprites. Only affects lossy
    /// tiles.
    #[cfg(feature = "webp")]
    #[arg(long = "no-sharp-yuv")]
    pub no_sharp_yuv: bool,

    /// How much smaller a lossy tile must be, in KiB per megapixel, for --adaptive to choose it
    /// over lossless. At 0, --adaptive keeps whichever is smaller.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = WebpEncoder::DEFAULT_LOSSY_MARGIN_PER_MEGAPIXEL / 1024)]
    pub lossy_margin: u32,

    /// Encode every tile both lossless and lossy, and keep the smaller one. Needs --quality.
    /// Encoding takes about twice as long.
    #[cfg(feature = "webp")]
    #[arg(long)]
    pub adaptive: bool,

    /// WebP quality for the levels above the most detailed one. Leave it out for lossless,
    /// the default. These levels never use sharp YUV, because scaling down has already smoothed
    /// the edges it fixes.
    #[cfg(feature = "webp")]
    #[arg(long)]
    pub upper_quality: Option<f32>,

    /// WebP spatial noise shaping, from 0 to 100, like cwebp -sns. Lower it to keep fine grain
    /// in vegetation and asphalt. Only affects lossy tiles.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub sns_strength: u8,

    /// WebP deblocking filter strength, from 0 to 100, like cwebp -f. Lower values keep grain
    /// sharper, but block edges may show. Often lowered together with --sns-strength. Only
    /// affects lossy tiles.
    #[cfg(feature = "webp")]
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub filter_strength: u8,

    /// Add random dithering, like cwebp -preprocessing 2. It breaks up banding in smooth
    /// gradients. Only affects lossy tiles.
    #[cfg(feature = "webp")]
    #[arg(long)]
    pub dither: bool,

    /// How hard to compress PNG tiles. Only used with --format png.
    #[arg(long, value_enum, default_value_t = CompressionArg::Fast)]
    pub compression: CompressionArg,
}

/// How to draw trees, bushes and grass.
#[derive(Clone, Debug, ClapArgs)]
#[non_exhaustive]
pub struct PlantsArgs {
    /// Don't build vegetation sprites. Most trees, bushes and grass are then missing.
    #[arg(long)]
    pub no_plants: bool,

    /// Season the vegetation is drawn in.
    #[arg(long, value_enum, default_value_t = SeasonArg::Summer2)]
    pub season: SeasonArg,

    /// Draw trees and bushes covered in snow instead of with seasonal leaves.
    #[arg(long)]
    pub snow: bool,

    /// Add flowers to bushes and ground cover.
    #[arg(long)]
    pub flower: bool,

    /// Use the large bush sprites.
    #[arg(long)]
    pub large_bush: bool,

    /// The size of ordinary trees, from 0 to 3.
    #[arg(long, default_value_t = 2)]
    pub tree_size: i32,

    /// The size of large trees, from --tree-size to 5. Sizes 4 and 5 use the game's larger
    /// tree sprites.
    #[arg(long, default_value_t = 3)]
    pub jumbo_tree_size: i32,

    /// Leave bushes and grass out.
    #[arg(long)]
    pub no_ground_cover: bool,

    /// Draw every tree as one species, numbered from 0 to 10.
    #[arg(long)]
    pub unify_tree_type: Option<usize>,
}

/// The `--game-build` values. Converts to [`Build`].
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum GameBuildArg {
    B41,
    B42,
}

impl From<GameBuildArg> for Build {
    fn from(arg: GameBuildArg) -> Self {
        match arg {
            GameBuildArg::B41 => Build::B41,
            GameBuildArg::B42 => Build::B42,
        }
    }
}

/// The `--packs` values: which built-in list of texture packs to load.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PackSetArg {
    /// The game's 2x packs.
    #[value(name = "vanilla-2x")]
    Vanilla2x,
    /// The game's 1x packs. Sprites are half the size.
    #[value(name = "vanilla-1x")]
    Vanilla1x,
    /// Every 2x pack found in the texture directories, smallest file first, so the largest
    /// pack wins when sprite names clash.
    #[value(name = "stacked-2x")]
    Stacked2x,
    /// Like stacked-2x, for the 1x packs.
    #[value(name = "stacked-1x")]
    Stacked1x,
    /// Only the packs given with --pack.
    None,
}

/// The `--iso-scale` values: the sprite scale of the texture packs.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum IsoScaleArg {
    /// Each square's diamond is 64x32 pixels, as in the game's 2x packs.
    #[value(name = "2x")]
    X2,
    /// Each square's diamond is 32x16 pixels, as in the 1x packs. Images have a quarter of
    /// the pixels.
    #[value(name = "1x")]
    X1,
}

/// The `--season` values. Converts to [`Season`].
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SeasonArg {
    Spring,
    Summer,
    Summer2,
    Autumn,
    Winter,
}

/// The `--format` values: the tile image format.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum FormatArg {
    /// WebP, encoded with libwebp. Needs the webp feature, which is on by default.
    #[cfg(feature = "webp")]
    Webp,
    Png,
}

impl Default for FormatArg {
    fn default() -> Self {
        #[cfg(feature = "webp")]
        return FormatArg::Webp;
        #[cfg(not(feature = "webp"))]
        return FormatArg::Png;
    }
}

/// The `--compression` values. Converts to [`PngCompression`].
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompressionArg {
    Fast,
    Balanced,
}

impl From<IsoScaleArg> for IsoScale {
    fn from(value: IsoScaleArg) -> Self {
        match value {
            IsoScaleArg::X2 => IsoScale::X2,
            IsoScaleArg::X1 => IsoScale::X1,
        }
    }
}

impl PackSetArg {
    /// Returns the sprite scale of this pack list.
    ///
    /// Returns 2x for `none`, because the scale of a hand-picked list isn't known.
    pub const fn scale(self) -> IsoScale {
        match self {
            PackSetArg::Vanilla2x | PackSetArg::Stacked2x | PackSetArg::None => IsoScale::X2,
            PackSetArg::Vanilla1x | PackSetArg::Stacked1x => IsoScale::X1,
        }
    }
}

impl From<SeasonArg> for Season {
    fn from(value: SeasonArg) -> Self {
        match value {
            SeasonArg::Spring => Season::Spring,
            SeasonArg::Summer => Season::Summer,
            SeasonArg::Summer2 => Season::Summer2,
            SeasonArg::Autumn => Season::Autumn,
            SeasonArg::Winter => Season::Winter,
        }
    }
}

impl From<CompressionArg> for PngCompression {
    fn from(value: CompressionArg) -> Self {
        match value {
            CompressionArg::Fast => PngCompression::Fast,
            CompressionArg::Balanced => PngCompression::Balanced,
        }
    }
}

impl EncodingArgs {
    /// Returns the encoder for the most detailed level.
    ///
    /// # Errors
    ///
    /// Returns an error if `--quality` is set together with `--format png`, or if `--adaptive`
    /// is set without `--quality`.
    pub fn encoder(&self) -> Result<BuiltinEncoder, String> {
        self.encoder_at(self.quality)
    }

    /// Returns the encoder for the levels above the most detailed one.
    ///
    /// # Errors
    ///
    /// The same as [`Self::encoder`], with `--upper-quality` in place of `--quality`.
    pub fn upper_encoder(&self) -> Result<BuiltinEncoder, String> {
        #[cfg(feature = "webp")]
        {
            let encoder = self.encoder_at(self.upper_quality)?;
            // Sharp YUV stops colors bleeding at sprite edges. Scaling down has already
            // smoothed those edges in the upper levels, so it is turned off there. This only
            // matters for lossy encoding.
            Ok(match encoder {
                BuiltinEncoder::Webp(webp) => BuiltinEncoder::Webp(webp.with_sharp_yuv(false)),
                other => other,
            })
        }
        #[cfg(not(feature = "webp"))]
        {
            self.encoder()
        }
    }

    fn encoder_at(&self, quality: Option<f32>) -> Result<BuiltinEncoder, String> {
        #[cfg(feature = "webp")]
        if self.adaptive && self.quality.is_none() {
            return Err(
                "--adaptive needs a --quality to compare lossless against; without one there is \
                 only lossless and nothing to choose between"
                    .to_owned(),
            );
        }

        match self.format {
            #[cfg(feature = "webp")]
            FormatArg::Webp => Ok(BuiltinEncoder::Webp(
                match quality {
                    Some(quality) if self.adaptive => WebpEncoder::adaptive(quality),
                    Some(quality) => WebpEncoder::lossy(quality),
                    None => WebpEncoder::LOSSLESS,
                }
                .with_method(self.webp_method)
                .with_lossless_quality(self.lossless_quality)
                .with_alpha_quality(self.alpha_quality)
                .with_sharp_yuv(!self.no_sharp_yuv)
                .with_lossy_margin(self.lossy_margin.saturating_mul(1024))
                .with_tone(self.sns_strength, self.filter_strength)
                .with_dither(self.dither),
            )),
            FormatArg::Png => {
                if quality.is_some() {
                    return Err(
                        "--quality and --upper-quality apply to --format webp; PNG is lossless, \
                         and its size/time trade is --compression"
                            .to_owned(),
                    );
                }
                Ok(BuiltinEncoder::Png(PngEncoder::new(
                    self.compression.into(),
                )))
            }
        }
    }
}

impl PlantsArgs {
    /// Returns the vegetation settings, or `None` if `--no-plants` was given.
    ///
    /// Without vegetation settings, a vegetation tile is drawn only if some pack happens to
    /// have a sprite with its name. Most don't.
    pub fn config(&self) -> Option<PlantsConfig> {
        if self.no_plants {
            return None;
        }

        Some(PlantsConfig {
            season: self.season.into(),
            snow: self.snow,
            flower: self.flower,
            large_bush: self.large_bush,
            tree_size: self.tree_size,
            jumbo_tree_size: self.jumbo_tree_size,
            no_ground_cover: self.no_ground_cover,
            unify_tree_type: self.unify_tree_type,
            ..PlantsConfig::default()
        })
    }
}

impl Default for PlantsArgs {
    /// The flags' defaults, as clap fills them in.
    fn default() -> Self {
        PlantsArgs {
            no_plants: false,
            season: SeasonArg::Summer2,
            snow: false,
            flower: false,
            large_bush: false,
            tree_size: 2,
            jumbo_tree_size: 3,
            no_ground_cover: false,
            unify_tree_type: None,
        }
    }
}

impl SourceArgs {
    /// Creates arguments for `map`, `texture_dirs` and `game_build`, with every other option at
    /// its default.
    pub fn new(map: PathBuf, texture_dirs: Vec<PathBuf>, game_build: GameBuildArg) -> Self {
        SourceArgs {
            map,
            texture_dirs,
            packs: PackSetArg::Vanilla2x,
            extra_packs: Vec::new(),
            game_build,
            build_memory_budget: None,
            max_skipped_cells: 0,
            untrusted_input: false,
            tile_defs: None,
            plants: PlantsArgs::default(),
        }
    }

    /// Returns `newtiledefinitions.tiles` in the parent of the first texture directory, if it
    /// exists.
    ///
    /// In a game install the texture directory is `media/texturepacks`, so this finds
    /// `media/newtiledefinitions.tiles`.
    fn default_tile_defs(&self) -> Option<PathBuf> {
        self.texture_dirs
            .first()
            .and_then(|dir| dir.parent())
            .map(|media| media.join("newtiledefinitions.tiles"))
            .filter(|path| path.is_file())
    }

    /// Returns the build settings these arguments describe, for the given targets.
    pub fn build_config(&self, targets: BuildTargets) -> BuildConfig {
        let stock: &[&str] = match self.packs {
            PackSetArg::Vanilla2x => VANILLA_2X_PACKS,
            PackSetArg::Vanilla1x => VANILLA_1X_PACKS,
            // The stacked lists are filled in below, by searching the texture directories.
            PackSetArg::Stacked2x | PackSetArg::Stacked1x | PackSetArg::None => &[],
        };

        let pack_load_order = stock
            .iter()
            .map(|name| (*name).to_string())
            .chain(self.extra_packs.iter().cloned())
            .collect();

        let config = BuildConfig {
            map_path: self.map.clone(),
            texture_paths: self.texture_dirs.clone(),
            pack_load_order,
            targets,
            plants: self.plants.config(),
            game_build: self.game_build.into(),
            limits: BuildLimits {
                max_skipped_cells: self.max_skipped_cells,
                max_build_bytes: self
                    .build_memory_budget
                    .map_or(BuildLimits::default().max_build_bytes, |mb| {
                        mb.saturating_mul(1 << 20)
                    }),
                ..BuildLimits::default()
            },
            // The command line memory-maps files unless `--untrusted-input` is given.
            input_trust: if self.untrusted_input {
                InputTrust::Untrusted
            } else {
                InputTrust::Trusted
            },
            tile_defs: self.tile_defs.clone().or_else(|| self.default_tile_defs()),
        };

        // This runs last so the `--pack` extras stay at the end of the load order, and win
        // over any pack found on disk.
        match self.packs {
            PackSetArg::Stacked2x => config.with_stacked_packs("2x"),
            PackSetArg::Stacked1x => config.with_stacked_packs("1x"),
            _ => config,
        }
    }
}

/// One `--layer` value: a range of layers, including both ends. A single layer is a range
/// of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerSpan {
    /// The lowest layer.
    pub first: i32,
    /// The highest layer.
    pub last: i32,
}

impl LayerSpan {
    /// The largest layer number, positive or negative, that `--layer` accepts.
    ///
    /// It is wide enough for any game build. The render rejects layers the map doesn't have.
    pub const LIMIT: i32 = 128;
}

/// Parses `LAYER` or `FIRST..LAST`, including both ends.
fn parse_layer_span(text: &str) -> Result<LayerSpan, String> {
    let layer = |raw: &str| -> Result<i32, String> {
        let value = raw
            .trim()
            .parse::<i32>()
            .map_err(|e| format!("{raw:?} in {text:?}: {e}"))?;
        if value.abs() > LayerSpan::LIMIT {
            return Err(format!(
                "layer {value} is outside -{limit}..{limit}",
                limit = LayerSpan::LIMIT
            ));
        }
        Ok(value)
    };

    let (first, last) = match text.split_once("..") {
        Some((first, last)) => (layer(first)?, layer(last)?),
        None => {
            let only = layer(text)?;
            (only, only)
        }
    };
    if first > last {
        return Err(format!(
            "range {text:?} runs downward; write the lower layer first, as in {last}..{first}"
        ));
    }
    Ok(LayerSpan { first, last })
}

impl RenderArgs {
    /// Returns the layers to render, in the order given, without repeats.
    pub fn layers(&self) -> Vec<i32> {
        let mut layers = Vec::new();
        for span in &self.layer_spans {
            for layer in span.first..=span.last {
                if !layers.contains(&layer) {
                    layers.push(layer);
                }
            }
        }
        layers
    }

    /// Returns the render job for one layer.
    ///
    /// Pass the same `layer_range` for every layer, so all the pyramids share one coordinate
    /// system and line up.
    pub fn job(&self, layer: i32, layer_range: (i32, i32), packs: PackSetArg) -> RenderJob {
        let scale = self.iso_scale.map_or_else(|| packs.scale(), IsoScale::from);

        RenderJob {
            layer,
            tile_size: self.tile_size,
            align_levels: self.align_levels,
            layer_range: Some(layer_range),
            scale,
            sprite_extent: self
                .legacy_margins
                .then(|| SpriteExtent::legacy_large(scale)),
            region: self.region,
            fuse_depth: self.fuse_depth,
            memory_budget: self.memory_budget.map(|mb| mb.saturating_mul(1 << 20)),
        }
    }
}

/// Parses `COL,ROW,COLS,ROWS`.
fn parse_region(text: &str) -> Result<TileRect, String> {
    let parts: Vec<&str> = text.split(',').collect();
    let [col, row, cols, rows] = parts.as_slice() else {
        return Err(format!("expected COL,ROW,COLS,ROWS, got {text:?}"));
    };

    let field = |name: &str, raw: &str| -> Result<u32, String> {
        raw.trim()
            .parse::<u32>()
            .map_err(|e| format!("{name} in {text:?}: {e}"))
    };

    Ok(TileRect::new(
        field("COL", col)?,
        field("ROW", row)?,
        field("COLS", cols)?,
        field("ROWS", rows)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{
        CommandFactory as _,
        Parser,
    };

    /// A program that uses every argument group, as `pzmap2img` does.
    #[derive(Debug, Parser)]
    struct Args {
        #[command(flatten)]
        source: SourceArgs,
        #[command(flatten)]
        render: RenderArgs,
        #[command(flatten)]
        encoding: EncodingArgs,
    }

    #[test]
    fn the_argument_groups_are_well_formed() {
        Args::command().debug_assert();
    }

    fn parse(extra: &[&str]) -> Result<Args, clap::Error> {
        let mut argv = vec!["test", "-m", "/maps/m", "-t", "/packs"];
        argv.extend_from_slice(extra);
        Args::try_parse_from(argv)
    }

    fn plan_layers(extra: &[&str]) -> Result<Vec<i32>, clap::Error> {
        Ok(parse(extra)?.render.layers())
    }

    /// `SourceArgs::new` repeats clap's defaults, so the two must match.
    #[test]
    fn source_args_new_matches_the_parsed_defaults() {
        let parsed = parse(&["--game-build", "b41"])
            .expect("valid arguments")
            .source;
        let built = SourceArgs::new(
            PathBuf::from("/maps/m"),
            vec![PathBuf::from("/packs")],
            GameBuildArg::B41,
        );
        assert_eq!(format!("{parsed:?}"), format!("{built:?}"));
    }

    #[test]
    fn layers_default_to_the_ground_floor() {
        assert_eq!(plan_layers(&[]).ok(), Some(vec![0]));
    }

    #[test]
    fn a_negative_layer_is_a_value_not_a_flag() {
        assert_eq!(plan_layers(&["--layer", "-3"]).ok(), Some(vec![-3]));
        assert_eq!(plan_layers(&["--layer=-3"]).ok(), Some(vec![-3]));
    }

    #[test]
    fn ranges_are_inclusive_and_mix_with_single_layers() {
        assert_eq!(
            plan_layers(&["--layer", "-17..-1"]).ok(),
            Some((-17..=-1).collect::<Vec<_>>())
        );
        assert_eq!(
            plan_layers(&["--layer", "0..29"]).ok(),
            Some((0..=29).collect::<Vec<_>>())
        );
        assert_eq!(
            plan_layers(&["--layer", "2", "--layer", "-1..1", "--layer", "0"]).ok(),
            Some(vec![2, -1, 0, 1])
        );
        assert_eq!(plan_layers(&["--layer", "4..4"]).ok(), Some(vec![4]));
    }

    #[test]
    fn bad_layers_are_rejected() {
        for bad in ["-1..-17", "a", "1..", "..3", "1...3", "0..999", "-200"] {
            assert!(plan_layers(&["--layer", bad]).is_err(), "{bad}");
        }
        // Another flag right after `--layer` is not swallowed as a layer.
        assert!(plan_layers(&["--layer", "--tile-size", "256"]).is_err());
    }

    #[test]
    fn regions_parse_and_reject() {
        assert_eq!(parse_region("1,2,3,4"), Ok(TileRect::new(1, 2, 3, 4)));
        assert_eq!(
            parse_region(" 1 , 2 , 3 , 4 "),
            Ok(TileRect::new(1, 2, 3, 4))
        );
        assert!(parse_region("1,2,3").is_err());
        assert!(parse_region("1,2,3,4,5").is_err());
        assert!(parse_region("1,2,3,-4").is_err());
        assert!(parse_region("a,b,c,d").is_err());
    }

    #[test]
    fn the_stock_pack_list_is_extended_not_replaced() {
        let args = parse(&["--pack", "MyMod.pack"]).expect("valid arguments");

        let config = args.source.build_config(BuildTargets::BOTH);
        assert_eq!(config.pack_load_order.len(), VANILLA_2X_PACKS.len() + 1);
        assert_eq!(
            config.pack_load_order.last().map(String::as_str),
            Some("MyMod.pack")
        );
    }

    #[test]
    fn no_plants_skips_the_compositing_pass() {
        let args = parse(&["--no-plants"]).expect("valid arguments");
        assert!(args.source.plants.config().is_none());
    }

    fn args(iso_scale: Option<IsoScaleArg>) -> RenderArgs {
        RenderArgs {
            layer_spans: vec![LayerSpan { first: 0, last: 0 }],
            tile_size: 1024,
            align_levels: 3,
            region: None,
            fuse_depth: DEFAULT_FUSE_DEPTH,
            memory_budget: None,
            space_check: true,
            legacy_margins: false,
            iso_scale,
        }
    }

    /// The flags' defaults, as clap would fill them in.
    fn encoding(format: FormatArg, quality: Option<f32>) -> EncodingArgs {
        EncodingArgs {
            format,
            quality,
            #[cfg(feature = "webp")]
            webp_method: 0,
            #[cfg(feature = "webp")]
            lossless_quality: 100,
            #[cfg(feature = "webp")]
            alpha_quality: 100,
            #[cfg(feature = "webp")]
            no_sharp_yuv: false,
            #[cfg(feature = "webp")]
            lossy_margin: WebpEncoder::DEFAULT_LOSSY_MARGIN_PER_MEGAPIXEL / 1024,
            #[cfg(feature = "webp")]
            adaptive: false,
            #[cfg(feature = "webp")]
            upper_quality: None,
            #[cfg(feature = "webp")]
            sns_strength: 50,
            #[cfg(feature = "webp")]
            filter_strength: 60,
            #[cfg(feature = "webp")]
            dither: false,
            compression: CompressionArg::Fast,
        }
    }

    #[test]
    fn quality_belongs_to_webp_and_is_refused_on_png() {
        use crate::render::TileEncoder as _;

        let png = encoding(FormatArg::Png, Some(85.0));
        assert!(png.encoder().is_err());

        let ok = EncodingArgs {
            quality: None,
            ..png.clone()
        };
        assert_eq!(
            ok.encoder().map(|e| e.format().to_owned()).as_deref(),
            Ok("png")
        );
    }

    /// Settings given as flags must reach the encoder unchanged.
    #[cfg(feature = "webp")]
    #[test]
    fn the_cwebp_recipe_maps_onto_the_flags() {
        use crate::render::WebpMode;

        let args = EncodingArgs {
            webp_method: 6,
            ..encoding(FormatArg::Webp, Some(92.0))
        };

        let Ok(BuiltinEncoder::Webp(encoder)) = args.encoder() else {
            panic!("expected a webp encoder");
        };
        assert_eq!(encoder.mode(), WebpMode::Lossy { quality: 92.0 });
        assert_eq!(encoder.method(), 6);
        assert_eq!(encoder.alpha_quality(), 100);
        assert!(encoder.sharp_yuv());
    }

    /// The upper levels stay lossless unless asked otherwise, even when the most detailed level
    /// is lossy.
    #[cfg(feature = "webp")]
    #[test]
    fn the_upper_levels_default_to_lossless() {
        use crate::render::WebpMode;

        let mut args = encoding(FormatArg::Webp, Some(92.0));

        let (Ok(BuiltinEncoder::Webp(native)), Ok(BuiltinEncoder::Webp(upper))) =
            (args.encoder(), args.upper_encoder())
        else {
            panic!("expected webp encoders");
        };
        assert_eq!(native.mode(), WebpMode::Lossy { quality: 92.0 });
        assert_eq!(upper.mode(), WebpMode::Lossless);

        // Sharp YUV is the one setting that does not carry over to the upper encoder.
        assert!(native.sharp_yuv());
        assert!(!upper.sharp_yuv());
        assert_eq!(native.to_lossless().with_sharp_yuv(false), upper);

        args.upper_quality = Some(95.0);
        let Ok(BuiltinEncoder::Webp(upper)) = args.upper_encoder() else {
            panic!("expected a webp encoder");
        };
        assert_eq!(upper.mode(), WebpMode::Lossy { quality: 95.0 });
        assert!(!upper.sharp_yuv());
    }

    #[cfg(feature = "webp")]
    #[test]
    fn adaptive_needs_something_to_compare_against() {
        use crate::render::WebpMode;

        let mut args = EncodingArgs {
            adaptive: true,
            ..encoding(FormatArg::Webp, None)
        };
        assert!(args.encoder().is_err(), "--adaptive without --quality");

        args.quality = Some(92.0);
        let Ok(BuiltinEncoder::Webp(native)) = args.encoder() else {
            panic!("expected a webp encoder");
        };
        assert_eq!(native.mode(), WebpMode::Adaptive { quality: 92.0 });

        // Upper levels have no `--upper-quality` here, so they stay lossless.
        let Ok(BuiltinEncoder::Webp(upper)) = args.upper_encoder() else {
            panic!("expected a webp encoder");
        };
        assert_eq!(upper.mode(), WebpMode::Lossless);
    }

    #[test]
    fn the_pack_set_picks_the_scale_unless_it_is_overridden() {
        let inferred = |packs| args(None).job(0, (0, 0), packs).scale;
        assert_eq!(inferred(PackSetArg::Vanilla2x), IsoScale::X2);
        assert_eq!(inferred(PackSetArg::Vanilla1x), IsoScale::X1);
        assert_eq!(inferred(PackSetArg::None), IsoScale::X2);

        // An explicit `--iso-scale` wins, including over a pack set that disagrees.
        let forced = args(Some(IsoScaleArg::X1));
        assert_eq!(
            forced.job(0, (0, 0), PackSetArg::Vanilla2x).scale,
            IsoScale::X1
        );
    }

    /// `--legacy-margins` must use the 1x size at 1x, not the 2x size.
    #[test]
    fn the_legacy_margin_box_follows_the_scale() {
        let mut render = args(Some(IsoScaleArg::X1));
        render.legacy_margins = true;

        assert_eq!(
            render.job(0, (0, 0), PackSetArg::Vanilla1x).sprite_extent,
            Some(SpriteExtent::centered(192, 256)),
        );
    }
}
