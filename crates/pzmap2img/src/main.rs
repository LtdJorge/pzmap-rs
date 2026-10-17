//! The `pzmap2img` command-line tool, which renders Project Zomboid maps to image tiles.
//!
//! It has three commands: `build` reads a map and reports what it found, `plan` also reports
//! how large a render would be, and `render` writes the tiles.

use anyhow::{
    Context as _,
    Result,
};
use clap::{
    Parser,
    Subcommand,
    ValueEnum,
};
use pzmap2img::{
    builder,
    cli::{
        EncodingArgs,
        RenderArgs,
        SourceArgs,
    },
    render::{
        BuiltinEncoder,
        DziStore,
        RenderPlan,
        RenderStats,
        RunContext,
        TYPICAL_BYTES_PER_MEGAPIXEL,
        TileEncoder as _,
        TileEncoders,
        check_free_space,
        render_pyramid,
    },
    runtime::{
        BuildTargets,
        raster::Map,
        size::HeapSize as _,
    },
    trace::TraceLevel,
};
use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    time::Instant,
};
use tracing::Level;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Debug, Parser)]
#[command(
    name = "pzmap2img",
    about = "Render Project Zomboid maps to Deep Zoom tile pyramids",
    version
)]
struct Args {
    #[command(subcommand)]
    command: Command,

    /// How much to log. The RUST_LOG environment variable overrides it.
    #[arg(long, short = 'l', value_enum, default_value_t = TraceLevel::Info, global = true)]
    trace_level: TraceLevel,

    /// How many worker threads to use. Defaults to one per CPU core.
    #[arg(long, short = 'j', global = true)]
    threads: Option<usize>,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    /// Read a map and report what it contains, without rendering.
    Build {
        #[command(flatten)]
        source: SourceArgs,

        /// Which outputs to build.
        #[arg(long, value_enum, default_value_t = TargetsArg::Both)]
        targets: TargetsArg,
    },

    /// Work out the render's layout and report its size, without writing anything.
    Plan {
        #[command(flatten)]
        source: SourceArgs,

        #[command(flatten)]
        render: RenderArgs,
    },

    /// Render one Deep Zoom image pyramid per layer.
    Render {
        #[command(flatten)]
        source: SourceArgs,

        #[command(flatten)]
        render: RenderArgs,

        /// The directory to write each layer's layer{N}.dzi file and layer{N}_files/ tiles to.
        #[arg(long, short = 'o')]
        out: PathBuf,

        #[command(flatten)]
        encoding: EncodingArgs,
    },
}

/// The `--targets` values. Converts to [`BuildTargets`].
#[derive(Clone, Copy, Debug, ValueEnum)]
enum TargetsArg {
    Raster,
    Vector,
    Both,
}

impl From<TargetsArg> for BuildTargets {
    fn from(value: TargetsArg) -> Self {
        match value {
            TargetsArg::Raster => BuildTargets::RASTER,
            TargetsArg::Vector => BuildTargets::VECTOR,
            TargetsArg::Both => BuildTargets::BOTH,
        }
    }
}

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() -> Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();

    let args = Args::parse();
    init_tracing(args.trace_level.into());

    // Without `--threads`, Rayon uses one thread per CPU the process may run on.
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .build_global()
            .context("configuring the worker pool")?;
    }

    match &args.command {
        Command::Build { source, targets } => run_build(source, (*targets).into()).map(|_| ()),
        Command::Plan { source, render } => run_plan(source, render),
        Command::Render {
            source,
            render,
            out,
            encoding,
        } => {
            let native = encoding.encoder().map_err(anyhow::Error::msg)?;
            let upper = encoding.upper_encoder().map_err(anyhow::Error::msg)?;
            run_render(source, render, out, TileEncoders::split(&native, &upper))
        }
    }
}

fn init_tracing(level: Level) {
    // `RUST_LOG` overrides `--trace-level` when set.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level.to_string().to_lowercase()));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Runs the build phase and prints a summary.
fn run_build(source: &SourceArgs, targets: BuildTargets) -> Result<builder::BuildOutput> {
    let config = source.build_config(targets);
    let output = builder::build(&config).context("build phase")?;
    let t = output.timings;

    println!(
        "build phase: {:.2?} for {} cells \
         (discover {:.0?}, headers {:.0?}, textures {:.0?}, cells {:.0?})",
        t.total, output.cells_discovered, t.discover, t.headers, t.textures, t.cells,
    );

    println!(
        "  memory:   {:.2} GB peak ({:.2} GB textures)",
        output.peak_bytes as f64 / 1e9,
        output.texture_bytes as f64 / 1e9,
    );

    if !output.skipped_cells.is_empty() {
        println!(
            "  skipped:  {} cells failed to parse and were skipped",
            output.skipped_cells.len()
        );
        for cell in output.skipped_cells.iter().take(5) {
            println!(
                "            ({}, {}) at the {} stage: {}",
                cell.x, cell.y, cell.stage, cell.reason
            );
        }
    }

    if let Some(stats) = &output.texture_stats {
        println!(
            "  textures: {} sprites ({} vegetation composites), {} missing, \
             {}/{} pages decoded, {:.0} MB blob ({:.1} MB slack)",
            stats.textures,
            stats.composited,
            stats.missing,
            stats.pages_decoded,
            stats.pages_total,
            stats.blob_bytes as f64 / 1e6,
            stats.slack_bytes as f64 / 1e6,
        );
    }

    if let Some(map) = &output.raster {
        let tiles: usize = map.cells().iter().map(|c| c.tile_count()).sum();
        let squares: usize = map.cells().iter().map(|c| c.occupied_squares()).sum();
        let layers: usize = map.cells().iter().map(|c| c.populated_layers()).sum();

        println!(
            "  raster: {tiles} tile refs, {squares} non-empty squares, \
             {layers} populated (cell,layer) pairs, {:.2} GB",
            map.heap_size() as f64 / 1e9,
        );
    }

    if let Some(annotations) = &output.annotations {
        println!(
            "  vector: {} rooms, {} buildings, {} distinct room names, {:.1} MB",
            annotations.rooms().len(),
            annotations.buildings().len(),
            annotations.names().len(),
            annotations.heap_size() as f64 / 1e6,
        );
    }

    Ok(output)
}

/// Builds the bitmap map, or returns an error if there is nothing to render.
fn raster_for_render(source: &SourceArgs) -> Result<Map> {
    run_build(source, BuildTargets::RASTER)?
        .raster
        .context("the build produced no raster map")
}

/// Works out each layer's render layout and prints its size. Writes nothing.
fn run_plan(source: &SourceArgs, render: &RenderArgs) -> Result<()> {
    let map = raster_for_render(source)?;
    let layer_range = map
        .layer_range()
        .context("the map has no populated layers")?;

    println!(
        "\nlayer range {}..={}, cells {}, grid step {}, sprite extent {:?}",
        layer_range.0,
        layer_range.1,
        map.cells().len(),
        render.iso_scale.map_or_else(
            || source.packs.scale(),
            pzmap2img::render::geometry::IsoScale::from
        ),
        pzmap2img::render::margins::SpriteExtent::measured(map.textures()),
    );

    let mut total_tiles = 0_u64;
    let mut total_bytes = 0_u64;

    for &layer in &render.layers() {
        let job = render.job(layer, layer_range, source.packs);
        let plan = RenderPlan::new(&map, &job).context("resolving the render layout")?;
        let coverage = plan.coverage(&map);
        let region = plan.region_of(&job);

        let (width, height) = plan.pyramid().canonical();
        let native = plan.native_region();

        println!(
            "\nlayer {layer}: {width}x{height} px, {} levels, {}x{} native tiles",
            plan.pyramid().max_level() + 1,
            native.cols,
            native.rows,
        );
        println!(
            "  covered: {} of {} native tiles ({:.1}%), bounding rect {},{} {}x{}",
            coverage.tiles,
            coverage.total,
            coverage.fraction() * 100.0,
            coverage.bounds.col,
            coverage.bounds.row,
            coverage.bounds.cols,
            coverage.bounds.rows,
        );

        if region != native {
            println!(
                "  region:  {},{} {}x{} ({} tiles)",
                region.col,
                region.row,
                region.cols,
                region.rows,
                region.tiles(),
            );
        }

        let pyramid_tiles = plan.estimated_tiles(&job, &coverage);
        let bytes = plan.estimated_output_bytes(&job, &coverage);
        total_tiles += pyramid_tiles;
        total_bytes += bytes;

        println!(
            "  at most:  {pyramid_tiles} tiles including upper levels, ~{:.0} GB lossless",
            bytes as f64 / 1e9,
        );
    }

    if render.layers().len() > 1 {
        println!(
            "\ntotal across {} layers: at most {total_tiles} tiles, ~{:.0} GB",
            render.layers().len(),
            total_bytes as f64 / 1e9,
        );
    }

    println!(
        "\nSizes are upper bounds at {:.2} MB per megapixel — lossless WebP, the default encoder, \
         measured on ground-layer tiles of a vanilla 2x map. A covered tile can still render \
         empty, and tiles above layer 0 are far emptier than the ones that average was taken \
         from, so upper layers come in well under. `--quality 85` roughly halves it again; \
         `--format png` is ~3x it.",
        TYPICAL_BYTES_PER_MEGAPIXEL as f64 / 1e6,
    );
    println!("\nNothing was written. Re-run with `render -o <dir>` to produce tiles.");
    Ok(())
}

/// Renders one pyramid for each requested layer.
fn run_render(
    source: &SourceArgs,
    render: &RenderArgs,
    out: &Path,
    encoders: TileEncoders<'_, BuiltinEncoder>,
) -> Result<()> {
    let map = raster_for_render(source)?;
    let layer_range = map
        .layer_range()
        .context("the map has no populated layers")?;

    let started = Instant::now();
    let mut totals = RenderStats::default();

    // Checks the space for all layers together, before any rendering starts.
    if render.space_check {
        let mut needed = 0_u64;
        for &layer in &render.layers() {
            let job = render.job(layer, layer_range, source.packs);
            let plan = RenderPlan::new(&map, &job)?;
            needed = needed.saturating_add(plan.estimated_output_bytes(&job, &plan.coverage(&map)));
        }

        fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
        check_free_space(out, needed)?;
    }

    for &layer in &render.layers() {
        let job = render.job(layer, layer_range, source.packs);
        let plan = RenderPlan::new(&map, &job)?;
        let store = DziStore::new(
            out.join(format!("layer{layer}_files")),
            encoders.native.format(),
        );

        let stats = render_pyramid(&map, &job, encoders, &store, RunContext::none())
            .with_context(|| format!("rendering layer {layer}"))?;
        store
            .write_descriptor(plan.pyramid())
            .with_context(|| format!("writing the descriptor for layer {layer}"))?;

        println!(
            "layer {layer}: {} tiles written, {} empty, {:.1} GB in {:.1?}",
            stats.written,
            stats.empty,
            stats.bytes as f64 / 1e9,
            stats.elapsed,
        );

        totals.rendered += stats.rendered;
        totals.written += stats.written;
        totals.empty += stats.empty;
        totals.bytes += stats.bytes;
    }

    println!(
        "\nrender phase: {:.1?} for {} layers as {}, {} tiles written ({:.1} GB) under {}",
        started.elapsed(),
        render.layers().len(),
        encoders.native.format(),
        totals.written,
        totals.bytes as f64 / 1e9,
        out.display(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_command_tree_is_well_formed() {
        Args::command().debug_assert();
    }
}
