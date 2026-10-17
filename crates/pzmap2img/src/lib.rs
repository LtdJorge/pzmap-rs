//! Renders Project Zomboid maps to Deep Zoom image tiles.
//!
//! It draws maps the way pzmap2dzi does, and writes the same Deep Zoom format. Work happens in
//! two phases:
//!
//! 1. **Build.** [`build`] reads the game's map files and texture packs, and keeps only what
//!    rendering needs in a compact form. The result is a [`Map`] for the bitmap tiles, and
//!    optionally [`Annotations`] with rooms, buildings and zombie spawn density.
//! 2. **Render.** [`render_pyramid`] draws one layer of the [`Map`] as a large isometric image,
//!    cut into a pyramid of tiles at several zoom levels. A [`TileEncoder`] turns each tile into
//!    PNG or WebP bytes, and a [`TileSink`] stores them.
//!
//! The `pzmap2img` command-line tool runs both phases. It needs the `cli` feature.
//!
//! # Game terms
//!
//! - A **square** is one map position, the size of a floor tile in the game.
//! - A **cell** is a square area of the map: 256x256 squares in Build 42, 300x300 in Build 41.
//!   Each cell is stored in its own pair of files.
//! - A **block** is a smaller square area inside a cell: 8x8 squares in Build 42, 10x10 in
//!   Build 41.
//! - A **layer** is one height of the map, which the game calls a level. Layer 0 is the ground,
//!   higher layers are upper floors, and negative layers are basements. This crate says layer
//!   because "level" here always means a level of the Deep Zoom pyramid.
//! - A cell's **lotheader** (`{x}_{y}.lotheader`) lists the tile names the cell uses, and its
//!   rooms and buildings.
//! - A cell's **lotpack** (`world_{x}_{y}.lotpack`) lists the tiles on every square.
//! - A **texture pack** (`.pack`) holds **pages**: PNG images with many **sprites** packed
//!   into each. Each tile name is the name of the sprite that draws it.
//!
//! # Examples
//!
//! Build a map, then render layer 0 as PNG tiles:
//!
//! ```no_run
//! use pzmap2img::{
//!     BuildConfig, BuildTargets, RenderJob, builder,
//!     render::{DziStore, PngEncoder, RenderPlan, RunContext, TileEncoders, render_pyramid},
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let media = std::path::Path::new("/path/to/ProjectZomboid/media");
//! let config = BuildConfig::new(
//!     media.join("maps/Muldraugh, KY"),
//!     vec![media.join("texturepacks")],
//! )
//! .with_targets(BuildTargets::RASTER);
//! let map = builder::build(&config)?.raster.ok_or("no map was built")?;
//!
//! let job = RenderJob::for_layer(0);
//! let store = DziStore::new("out/layer0_files", "png");
//! let encoder = PngEncoder::default();
//! render_pyramid(&map, &job, TileEncoders::uniform(&encoder), &store, RunContext::none())?;
//! store.write_descriptor(RenderPlan::new(&map, &job)?.pyramid())?;
//! # Ok(())
//! # }
//! ```
//!
//! # Feature flags
//!
//! - `cli` (default): the command-line tool and the `cli` module's argument types. Turn it off
//!   to use this crate as a library without clap.
//! - `webp` (default): WebP output, using libwebp. Without it, tiles are PNG only.
//! - `dhat-heap`: uses dhat as the allocator, to profile the tool's heap use.

// Tests may panic, since a failed assertion or unwrap is how a test reports a problem.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub(crate) mod budget;
pub mod builder;
pub mod config;
pub(crate) mod contents;
pub mod error;
pub(crate) mod math;
pub mod render;
pub mod runtime;
pub(crate) mod source;
pub mod string_table;

#[cfg(test)]
mod testing;
pub mod textures;

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "cli")]
pub mod trace;

pub use builder::{
    BuildOutput,
    StageTimings,
    build,
};
pub use config::BuildConfig;
pub use error::BuildError;
pub use render::{
    RenderError,
    RenderJob,
    RenderPlan,
    RenderStats,
    TileEncoder,
    TileEncoders,
    TileSink,
    TileSource,
    render_pyramid,
    render_tile,
};
pub use runtime::{
    BuildTargets,
    raster::{
        CellGeometry,
        Map,
    },
    size::HeapSize,
    vector::Annotations,
};
pub use string_table::{
    RoomNameId,
    StringTable,
    TileId,
};
pub use textures::{
    Sprite,
    TexMeta,
    TextureLibrary,
};
