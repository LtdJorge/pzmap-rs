#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
//! Render phase tests against a real Project Zomboid install.
//!
//! The tests are ignored by default. To run them:
//!
//! ```text
//! cargo test -p pzmap2img --release --test render_phase -- --ignored --nocapture
//! ```
//!
//! They need `PZ_INSTALL_DIR` set to a Build 42 install, and panic with that instruction when it
//! isn't set.

mod common;

use pzmap2img::{
    Map,
    builder,
    render::{
        DziStore,
        PngEncoder,
        RenderJob,
        RenderPlan,
        RunContext,
        TileEncoders,
        TileRect,
        TileSource,
        blend::Canvas,
        geometry::IsoScale,
        margins::SpriteExtent,
        pyramid::TileAddress,
        render_pyramid,
        render_tile,
    },
    runtime::BuildTargets,
};

fn build_raster() -> Map {
    let config = common::vanilla_muldraugh_2x().with_targets(BuildTargets::RASTER);
    builder::build(&config)
        .expect("build phase")
        .raster
        .expect("raster target was requested")
}

/// Returns the world square at the center of the cell with the most tiles. That cell is in a
/// town, not open water.
fn busiest_centre(map: &Map) -> (i32, i32) {
    let cell_squares = map.geometry().cell_squares() as i32;
    let busiest = map
        .cells()
        .iter()
        .max_by_key(|cell| cell.tile_count())
        .expect("the map has cells");
    (
        busiest.cell_x * cell_squares + cell_squares / 2,
        busiest.cell_y * cell_squares + cell_squares / 2,
    )
}

#[test]
#[ignore = "requires an installed copy of the game"]
fn the_plan_is_self_consistent() {
    let map = build_raster();
    let job = RenderJob::for_layer(0);
    let plan = RenderPlan::new(&map, &job).expect("layer 0 is populated");

    let (width, height) = plan.pyramid().canonical();
    let layout = plan.layout();

    assert_eq!(layout.tile_size(), job.tile_size);
    assert_eq!(layout.scale(), IsoScale::X2);
    assert_eq!(width % layout.scale().step_x() as u32, 0);
    assert_eq!(height % layout.scale().step_y() as u32, 0);

    let native = plan.native_region();
    assert_eq!(native.cols, width.div_ceil(job.tile_size));
    assert_eq!(native.rows, height.div_ceil(job.tile_size));

    println!(
        "layer 0: {width}x{height} px, {} levels, {}x{} native tiles ({} total), origin ({},{})",
        plan.pyramid().max_level() + 1,
        native.cols,
        native.rows,
        native.tiles(),
        layout.origin_col(),
        layout.origin_row(),
    );
    println!(
        "measured sprite extent: {:?}",
        SpriteExtent::measured(map.textures())
    );
    println!("map layer range: {:?}", map.layer_range());
}

#[test]
#[ignore = "requires an installed copy of the game"]
fn the_one_x_pack_set_halves_the_image() {
    let two_x = build_raster();
    let plan_2x = RenderPlan::new(&two_x, &RenderJob::for_layer(0)).expect("layer 0 is populated");

    let config = common::vanilla_muldraugh_2x()
        .with_pack_load_order(
            pzmap2img::config::VANILLA_1X_PACKS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        )
        .with_targets(BuildTargets::RASTER);

    let one_x = builder::build(&config)
        .expect("build phase")
        .raster
        .expect("raster target was requested");

    let job = RenderJob::for_layer(0)
        .with_scale(IsoScale::X1)
        .with_tile_size(512);
    let plan_1x = RenderPlan::new(&one_x, &job).expect("layer 0 is populated");

    let (w2, h2) = plan_2x.pyramid().canonical();
    let (w1, h1) = plan_1x.pyramid().canonical();
    println!("2x {w2}x{h2} px -> 1x {w1}x{h1} px");
    assert_eq!((w2 / 2, h2 / 2), (w1, h1));

    assert_eq!(plan_2x.layout().origin_col(), plan_1x.layout().origin_col());
    assert_eq!(plan_2x.layout().origin_row(), plan_1x.layout().origin_row());
    assert_eq!(plan_2x.native_region(), plan_1x.native_region());

    let (x, y) = busiest_centre(&two_x);
    assert_eq!(
        plan_2x.layout().tile_for_world_square(x, y, 0),
        plan_1x.layout().tile_for_world_square(x, y, 0),
    );

    // Sprites drawn at half size would still pass every check above, so this checks that the
    // tile is mostly covered.
    let (col, row) = plan_1x.layout().tile_for_world_square(x, y, 0);
    let mut canvas = Canvas::new(512, 512);
    render_tile(&one_x, &plan_1x, col, row, &mut canvas);

    let covered = canvas
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count();
    let total = canvas.pixels().len() / 4;
    println!(
        "1x tile ({col},{row}): {:.1}% covered",
        covered as f64 * 100.0 / total as f64
    );
    assert!(
        covered * 2 > total,
        "an interior 1x tile should be more than half covered, got {covered}/{total}",
    );
}

/// Every cell's corners must land inside the image. If not, the margin is too small and the
/// map is cut off.
#[test]
#[ignore = "requires an installed copy of the game"]
fn every_cell_corner_falls_inside_the_image() {
    let map = build_raster();
    let plan = RenderPlan::new(&map, &RenderJob::for_layer(0)).expect("layer 0 is populated");

    let (cols, rows) = (plan.native_region().cols, plan.native_region().rows);
    let cell_squares = map.geometry().cell_squares() as i32;

    for cell in map.cells() {
        let x0 = cell.cell_x * cell_squares;
        let y0 = cell.cell_y * cell_squares;

        for (x, y) in [
            (x0, y0),
            (x0 + cell_squares - 1, y0),
            (x0, y0 + cell_squares - 1),
            (x0 + cell_squares - 1, y0 + cell_squares - 1),
        ] {
            let (col, row) = plan.layout().tile_for_world_square(x, y, 0);
            assert!(
                col >= 0 && row >= 0 && (col as u32) < cols && (row as u32) < rows,
                "cell ({},{}) corner ({x},{y}) maps to tile ({col},{row}), outside {cols}x{rows}",
                cell.cell_x,
                cell.cell_y,
            );
        }
    }
}

/// A render that drew nothing would pass every other check, so this checks for pixels.
#[test]
#[ignore = "requires an installed copy of the game"]
fn tiles_over_populated_cells_are_not_empty() {
    let map = build_raster();
    let plan = RenderPlan::new(&map, &RenderJob::for_layer(0)).expect("layer 0 is populated");

    let mut canvas = Canvas::new(plan.pyramid().tile_size(), plan.pyramid().tile_size());
    let (x, y) = busiest_centre(&map);
    let (col, row) = plan.layout().tile_for_world_square(x, y, 0);

    render_tile(&map, &plan, col, row, &mut canvas);
    assert!(
        !canvas.is_transparent(),
        "tile ({col},{row}) over square ({x},{y}) came out empty",
    );

    let covered = canvas
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count();
    let total = canvas.pixels().len() / 4;
    println!(
        "tile ({col},{row}) over square ({x},{y}): {:.1}% covered",
        covered as f64 * 100.0 / total as f64,
    );
    assert!(
        covered * 2 > total,
        "an interior tile should be more than half covered, got {covered}/{total}",
    );
}

#[test]
#[ignore = "requires an installed copy of the game"]
fn rendering_is_deterministic_and_canvas_reuse_is_safe() {
    let map = build_raster();
    let plan = RenderPlan::new(&map, &RenderJob::for_layer(0)).expect("layer 0 is populated");

    let size = plan.pyramid().tile_size();
    let (x, y) = busiest_centre(&map);
    let (col, row) = plan.layout().tile_for_world_square(x, y, 0);

    let mut fresh = Canvas::new(size, size);
    render_tile(&map, &plan, col, row, &mut fresh);

    let mut reused = Canvas::new(size, size);
    render_tile(&map, &plan, col + 1, row + 1, &mut reused);
    render_tile(&map, &plan, col, row, &mut reused);

    assert_eq!(fresh.pixels(), reused.pixels());
}

/// Tiles outside the computed bounds must be empty. Otherwise the disk space estimate from
/// `pzmap2img plan` would be too low.
#[test]
#[ignore = "requires an installed copy of the game"]
fn uncovered_tiles_really_are_empty() {
    let map = build_raster();
    let job = RenderJob::for_layer(0);
    let plan = RenderPlan::new(&map, &job).expect("layer 0 is populated");
    let coverage = plan.coverage(&map);

    println!(
        "coverage: {} of {} tiles ({:.1}%), bounds {},{} {}x{}",
        coverage.tiles,
        coverage.total,
        coverage.fraction() * 100.0,
        coverage.bounds.col,
        coverage.bounds.row,
        coverage.bounds.cols,
        coverage.bounds.rows,
    );
    assert!(coverage.tiles > 0);
    assert!(
        coverage.tiles < coverage.total,
        "a whole-map level should not be fully covered",
    );

    // Points just outside each edge of the bounds are the most likely to catch an off-by-one
    // error.
    let size = plan.pyramid().tile_size();
    let mut canvas = Canvas::new(size, size);
    let bounds = coverage.bounds;

    let outside = [
        (bounds.col as i32 - 1, bounds.row as i32 - 1),
        (bounds.col as i32 - 1, (bounds.row + bounds.rows / 2) as i32),
        (
            (bounds.col + bounds.cols) as i32,
            (bounds.row + bounds.rows / 2) as i32,
        ),
        ((bounds.col + bounds.cols / 2) as i32, bounds.row as i32 - 1),
        (
            (bounds.col + bounds.cols / 2) as i32,
            (bounds.row + bounds.rows) as i32,
        ),
    ];

    for (col, row) in outside {
        render_tile(&map, &plan, col, row, &mut canvas);
        assert!(
            canvas.is_transparent(),
            "tile ({col},{row}) is outside the coverage bounds but rendered pixels",
        );
    }
}

#[test]
#[ignore = "requires an installed copy of the game"]
fn a_region_renders_to_disk_and_reads_back() {
    let map = build_raster();
    let job = RenderJob::for_layer(0);
    let plan = RenderPlan::new(&map, &job).expect("layer 0 is populated");

    let (x, y) = busiest_centre(&map);
    let (col, row) = plan.layout().tile_for_world_square(x, y, 0);

    let dir = std::env::temp_dir().join(format!("pzmap2img-render-{}", std::process::id()));
    let store = DziStore::new(dir.join("layer0_files"), "png");

    let encoder = PngEncoder::default();
    let job = job.with_region(TileRect::new(col as u32, row as u32, 2, 2));
    let stats = render_pyramid(
        &map,
        &job,
        TileEncoders::uniform(&encoder),
        &store,
        RunContext::none(),
    )
    .expect("rendering a 2x2 region");

    println!(
        "{} tiles in {:.2?} ({} written, {} empty, {:.1} MB)",
        stats.rendered,
        stats.elapsed,
        stats.written,
        stats.empty,
        stats.bytes as f64 / 1e6,
    );
    assert!(stats.written >= 1, "an interior region should write tiles");

    let address = TileAddress::new(plan.pyramid().max_level(), col as u32, row as u32);
    let read = store
        .read_tile(address)
        .expect("reading back")
        .expect("just written");
    assert_eq!(read.width(), plan.pyramid().tile_size());

    let mut expected = Canvas::new(plan.pyramid().tile_size(), plan.pyramid().tile_size());
    render_tile(&map, &plan, col, row, &mut expected);

    // PNG stores alpha that isn't premultiplied, so saving and loading rounds each channel
    // again. A difference of one is that rounding. Anything more is a real mismatch.
    let worst = read
        .pixels()
        .iter()
        .zip(expected.pixels())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 1,
        "worst channel difference after a PNG round trip: {worst}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
