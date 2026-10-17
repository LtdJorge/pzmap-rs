#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
//! Build phase tests against a real Project Zomboid install.
//!
//! The tests are ignored by default. To run them:
//!
//! ```text
//! cargo test -p pzmap2img --release --test build_phase -- --ignored --nocapture
//! ```
//!
//! They need `PZ_INSTALL_DIR` set to a Build 42 install, and panic with that instruction when it
//! isn't set.

mod common;

use pzmap2img::{
    builder,
    runtime::{
        BuildTargets,
        size::HeapSize as _,
    },
};

#[test]
#[ignore = "requires an installed copy of the game"]
fn full_build_matches_measured_shape() -> anyhow::Result<()> {
    let config = common::vanilla_muldraugh_2x().with_targets(BuildTargets::BOTH);
    let output = builder::build(&config)?;

    assert_eq!(output.cells_discovered, 4065);

    let map = output.raster.expect("raster requested");
    let stats = output.texture_stats.expect("textures requested");
    let annotations = output.annotations.expect("vector requested");

    // Name counts, found by a separate scan of the lotheaders.
    assert_eq!(map.tile_names().len(), 20_777, "distinct tile names");
    assert_eq!(annotations.names().len(), 527, "distinct room names");
    assert_eq!(annotations.rooms().len(), 85_003, "room instances");
    assert_eq!(annotations.buildings().len(), 8_724, "buildings");

    assert_eq!(stats.composited, 48, "vegetation composites");

    // Tiles with no sprite in the vanilla packs. Fails if the count grows.
    assert!(
        stats.missing <= 46,
        "missing sprites regressed to {}",
        stats.missing
    );

    let tiles: usize = map.cells().iter().map(|c| c.tile_count()).sum();
    let squares: usize = map.cells().iter().map(|c| c.occupied_squares()).sum();

    // Estimated from a sample of 60 cells: about 474 million tiles on about 256 million
    // squares.
    assert!(
        (450_000_000..500_000_000).contains(&tiles),
        "tile refs {tiles}"
    );
    assert!(
        (245_000_000..270_000_000).contains(&squares),
        "non-empty squares {squares}"
    );

    let layer0: usize = map.cells().iter().map(|c| c.occupied_squares_on(0)).sum();
    assert!(
        layer0 as f64 / squares as f64 > 0.98,
        "layer 0 share {:.3}",
        layer0 as f64 / squares as f64
    );

    println!(
        "raster {:.2} GB, vector {:.1} MB, blob {:.0} MB",
        map.heap_size() as f64 / 1e9,
        annotations.heap_size() as f64 / 1e6,
        stats.blob_bytes as f64 / 1e6,
    );

    Ok(())
}

/// Room contents come from the lotpacks and the tile definitions, without loading textures.
#[test]
#[ignore = "requires an installed copy of the game"]
fn room_contents_are_counted() -> anyhow::Result<()> {
    let config = common::vanilla_muldraugh_2x().with_targets(BuildTargets::VECTOR.with_contents());
    let output = builder::build(&config)?;
    assert!(output.raster.is_none(), "no raster was requested");

    let annotations = output.annotations.expect("vector requested");
    let kinds = annotations.item_kinds();
    let total = annotations.item_total();
    let rooms_with_items = annotations
        .rooms()
        .iter()
        .filter(|room| !room.items.is_empty())
        .count();
    println!(
        "{total} container tiles in {rooms_with_items} of {} rooms, {} kinds, cells {:.2?}",
        annotations.rooms().len(),
        kinds.len(),
        output.timings.cells
    );

    let fridge = kinds
        .iter()
        .position(|kind| kind.label.as_ref() == "Fridge")
        .expect("Muldraugh has fridges") as u32;
    let kitchens: Vec<_> = annotations
        .rooms()
        .iter()
        .filter(|room| annotations.name(room.name) == Some("kitchen"))
        .collect();
    let with_fridge = kitchens
        .iter()
        .filter(|room| annotations.items_of(room).iter().any(|i| i.kind == fridge))
        .count();
    println!("{with_fridge} of {} kitchens have a fridge", kitchens.len());

    // The counts for the Build 42 install. Fails if any of them changes.
    assert_eq!(kinds.len(), 91, "distinct (container, label) kinds");
    assert_eq!(total, 202_240, "container tiles");
    assert_eq!(
        rooms_with_items, 56_920,
        "rooms with at least one container"
    );
    assert_eq!(
        (with_fridge, kitchens.len()),
        (6_723, 7_401),
        "kitchens with a fridge"
    );

    Ok(())
}
