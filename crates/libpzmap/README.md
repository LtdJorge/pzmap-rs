# libpzmap

Parsers for the map files of Project Zomboid: `.lotheader`, `world_*.lotpack`, `.pack` texture
atlases, and sidecar files (`objects.lua`, `streets.xml`, `.tiles`).

The parsers are zero-copy, they take a `&[u8]` and return usable data structures. Strings and PNG data borrow the
input buffer instead of being copied. It doesn't read the strings or decode the images.

```rust,no_run
use libpzmap::{B42, lotheader::LotHeader};

let bytes = std::fs::read("0_18.lotheader")?;
let header = LotHeader::<B42>::parse(0, 18, &bytes)?;
println!("{} distinct tile names", header.strings.len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

libpzmap needs Rust 1.88 or later.

## How the game stores a map

The world is a grid of **cells**. A cell is a square area of 256×256 squares in Build 42, or
300×300 in Build 41. Each cell is stored as two files named after its position in the grid:

- `X_Y.lotheader` lists for the cell at (x,y) the name of every tile used, its rooms and buildings, and the zombie
  population in each block.
- `world_X_Y.lotpack` says which tiles sit on each square. It stores each tile as a number that
  is an index into the lotheader's name list, so both files are needed to read a cell.

A **square** is one floor position on one **layer**. Layers are what the game calls levels:
layer 0 is ground level, and Build 42 adds negative layers for basements. Inside a cell,
squares are grouped into **blocks** of 8×8 (10×10 in Build 41) loaded together by the game.

A **tile** is one sprite drawn on a square, such as a floor, a wall, or a fridge. A square can
hold several tiles, drawn bottom to top. The sprite images live in **texture packs** (`.pack`
files). Each pack holds a few large PNG images (spritesheets), called pages, and a table with each
sprite's name and rectangle within the page.

The `libpzmap::glossary` module defines these terms and a few more in one place.

## Builds

The crate reads files from Build 41 and Build 42 of the game. The two builds use different cell
and block sizes and a different range of layers, so you have to choose the game build for the files to parse.

When you know the build at compile time, pass `B41` or `B42` as a type parameter, as in
`LotHeader::<B42>`. The build becomes part of the type, so a `Cell<B41>` cannot be passed where
a `Cell<B42>` is expected.
When the build is chosen at runtime, for example, from a command-line flag, use the `Build` enum
with `AnyLotHeader` and `AnyCell`.

## Reading a cell

Use `Cell::visit` to go through the squares once, or `Cell::parse` to keep them for random
access:

```rust,no_run
use libpzmap::{B42, lotpack::Cell};

let bytes = std::fs::read("world_0_18.lotpack")?;

// Calls the closure for each non-empty square. Tile data is borrowed from `bytes`.
Cell::<B42>::visit(&bytes, -32, 32, |at, tiles| {
    println!("block {} layer {} has {} tiles", at.block, at.layer, tiles.len());
})?;

// Collects every block, so squares can be looked up afterwards.
let cell = Cell::<B42>::parse(&bytes, -32, 32)?;
println!("{} blocks", cell.blocks().len());
# Ok::<(), Box<dyn std::error::Error>>(())
```

A lotpack does not record which layers it contains. Take `min_layer` and `max_layer` from the
cell's `.lotheader`.

## Sidecar files

A map directory also includes files that describe the map as a whole. `libpzmap::sidecar` reads
these:

- `objects.lua` and `regions.lua` list zones: typed rectangles, polygons, and lines.
- `streets.xml` lists named streets as center lines.
- `media/newtiledefinitions.tiles` lists properties for each sprite, such as whether it is a
  container.

The Lua files are read as plain data and never executed to prevent running code from a map coming from an untrusted
source.
Anything beyond plain table literals is rejected with `ParseErrorKind::Syntax`.

```rust,no_run
use libpzmap::sidecar::{streets::Street, tiledefs::TileDefs, zones::Zone};

let (objects, streets_xml, tiles) = (
    std::fs::read("objects.lua")?,
    std::fs::read("streets.xml")?,
    std::fs::read("newtiledefinitions.tiles")?,
);
let zones = Zone::parse_all(&objects)?; // borrows `objects`
let streets = Street::parse_all(&streets_xml)?;
let defs = TileDefs::parse(&tiles)?;
println!("{} zones, {} streets", zones.len(), streets.len());
println!("{:?}", defs.property("appliances_refrigeration_01_20", "container"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Owned results

Most parsed types borrow the input buffer (zero-copy). When you need an owned result:

- `lotheader::AnyLotHeader` and `texturepack::OwnedTexturePack` keep the parsed value together
  with its buffer without copying.
- `lotpack::AnyCell` copies a cell's squares and tile numbers into a few flat arrays.

The Java and Python bindings are built on these types.

## Errors and panics

Every parser returns `Result<_, ParseError>`. An error points to the file format and the byte offset
at which reading failed. `ParseError` is `'static` and `Send`, so you can pass it between threads
or box it without turning it into a string.

A malformed file should produce an error, not a panic. Before allocating, the parsers check every count
read from the file against the bytes that remain. They also check offsets before following them
and check every read against the end of the buffer. The crate uses assertions only for its own
internal invariants, keeping them enabled in release builds (inspired by the Tiger-style guide).

## Other languages

Java and Python bindings are in the same repository, as `pzmap-java` and `pzmap-python`.

## License

MIT OR Apache-2.0.

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
