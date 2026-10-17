# pzmap2img

`pzmap2img` renders Project Zomboid maps to Deep Zoom image pyramids. It is both a command-line tool
and a Rust library.

Give it a map folder and the game's texture packs, and it draws the map the same way as Project Zomboid:
an isometric view with every floor, wall, object, and tree. The result is one Deep Zoom (DZI)
pyramid per layer (in-game floor level). A pyramid is a very large image cut into tiles, stored at several zoom levels,
so a map viewer can load only the tiles on screen. OpenSeadragon and most other Deep Zoom viewers
can open it.

It is for anyone who wants a picture of a map: players making a map website, modders checking
their own maps, and pzmap2dzi users who want faster renders.

## Relation to pzmap2dzi

`pzmap2img` draws maps the same way as [pzmap2dzi](https://github.com/cff29546/pzmap2dzi) and writes
the same Deep Zoom format. Parts of it follow pzmap2dzi's code, as listed in [NOTICE](NOTICE).

By default, pzmap2img positions tiles using the size of the largest sprite it loads. Pass
`--legacy-margins` to use pzmap2dzi's fixed sprite size instead, which gives exactly the same
tile numbering as pzmap2dzi.

## Layers and levels

The game calls each height of the map a level. `pzmap2img` calls them **layers**, because "level"
here always means a zoom level of the Deep Zoom pyramid. Layer 0 is the ground, higher layers
are upper floors, and negative layers are basements (Build 42+).

## How it works

A render has two phases:

1. **Build.** pzmap2img reads the map's cell files and the texture packs. It turns that data into a compact in-memory
   format.
2. **Render.** It draws each requested layer from the in-memory format and writes the tiles of its pyramid, as WebP or
   PNG files.

The `build` command runs only the first phase and reports what the map contains. The `plan`
command also works out how large each pyramid would be, without writing anything. The `render`
command runs both phases and writes the tiles.

## Install

```sh
cargo install pzmap2img
```

Or, from a clone of this repository:

```sh
cargo install --path crates/pzmap2img
```

WebP output compiles libwebp from source, so you will need a C compiler. See [Features](#features) to
build without it.

## Command-line quick start

The examples below use `PZ_INSTALL_DIR` for the game's `projectzomboid` folder, the one that
contains `media`. With Steam on Linux, it is usually
`~/.local/share/Steam/steamapps/common/ProjectZomboid/projectzomboid`.

First, check how large the render will be. `plan` reads the map and prints each layer's size in
pixels and tiles, and an upper bound on its size on disk:

```sh
pzmap2img plan \
  --map "$PZ_INSTALL_DIR/media/maps/Muldraugh, KY" \
  --texture-dir "$PZ_INSTALL_DIR/media/texturepacks" \
  --layer 0
```

Then render. This writes `layer0.dzi` and the tiles in `layer0_files/` to the output folder:

```sh
pzmap2img render \
  --map "$PZ_INSTALL_DIR/media/maps/Muldraugh, KY" \
  --texture-dir "$PZ_INSTALL_DIR/media/texturepacks" \
  --layer 0 \
  --out /path/to/output
```

Some useful options:

- `--layer 0..7` renders a range of layers, each to its own pyramid. `--layer` can be repeated.
- `--region COL,ROW,COLS,ROWS` renders only part of the map, which is useful for a quick test.
  The numbers are tiles of the most detailed level.
- `--quality 85` makes lossy WebP tiles, which are much smaller. Tiles are lossless by default.
- `--format png` writes PNG tiles instead of WebP.
- `--tile-size` sets the tile size in pixels. The default is 1024.
- `--game-build b41` reads a Build 41 map. The default is Build 42.
- `--season`, `--snow` and the tree options change how vegetation is drawn.

Run `pzmap2img help` or `pzmap2img <command> --help` to list every option.

## Disk space and memory

A full map is really large. With the default lossless tiles, the ground layer of the base game map
alone can need several terabytes, so always run `plan` first. On Unix systems, pzmap2img checks
that the output folder has enough free space before a render starts, stopping if it doesn't. `--no-space-check` skips
this check for network drives that report their free space wrongly.

`plan` gives upper bounds for _lossless_ WebP. Upper layers are mostly empty, so they come in well
under the estimate. Lossy tiles with `--quality 85` are roughly half the size, and PNG tiles are
about three times the size.

The build phase may use up to 6 GiB of memory by default. `--build-memory-budget` changes the limit, in MiB. If the
build needs more than the limit, it
aborts with an error printing what the memory was for. The render phase keeps its tiles within a
separate limit, set with `--memory-budget`. These two parameters are useful for safety when rendering third-party maps.

## Library quick start

Add the crate without the command-line tool's dependencies:

```toml
[dependencies]
pzmap2img = { version = "0.1", default-features = false, features = ["webp"] }
```

Build a map, then render layer 0 as PNG tiles:

```rust
use pzmap2img::{
    BuildConfig, BuildTargets, RenderJob, builder,
    render::{DziStore, PngEncoder, RenderPlan, RunContext, TileEncoders, render_pyramid},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let media = std::path::Path::new("/path/to/ProjectZomboid/media");
    let config = BuildConfig::new(
        media.join("maps/Muldraugh, KY"),
        vec![media.join("texturepacks")],
    )
        .with_targets(BuildTargets::RASTER);
    let map = builder::build(&config)?.raster.ok_or("no map was built")?;

    let job = RenderJob::for_layer(0);
    let store = DziStore::new("out/layer0_files", "png");
    let encoder = PngEncoder::default();
    render_pyramid(&map, &job, TileEncoders::uniform(&encoder), &store, RunContext::none())?;
    store.write_descriptor(RenderPlan::new(&map, &job)?.pyramid())?;
    Ok(())
}
```

`builder::build` can also return `Annotations`: the map's rooms, buildings, and zombie spawn
density. You can store tiles somewhere other than a folder by implementing `TileSink`, and use
another image format by implementing `TileEncoder`. These are covered by the documentation.

## Features

- `cli` (on by default) builds the `pzmap2img` command-line tool. Turn it off to use the crate only as
  a library without clap and its other dependencies.
- `webp` (on by default) adds WebP output, using libwebp. Without it, tiles are PNG only. Heavily recommended.
- `dhat-heap` uses dhat as the memory allocator, to profile the command-line tool's memory use.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.

Parts of pzmap2img follow pzmap2dzi, which is under the MIT license.
[NOTICE](NOTICE) lists those parts and includes pzmap2dzi's license.

The licenses of the crates and of libwebp compiled into the `pzmap2img` program are in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
