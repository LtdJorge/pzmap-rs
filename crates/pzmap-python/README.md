# pzmap

Python bindings for [`libpzmap`](https://github.com/LtdJorge/pzmap-rs), which parses Project
Zomboid's on-disk map formats: `.lotheader`, `world_*.lotpack`, and `.pack` texture atlases.

It does not decode images or draw anything. A texture page comes back as the bytes of a PNG file,
and a square's tiles come back as positions in a list of tile names.

## Install

```sh
pip install pzmap
```

## Use

The map is divided into cells, square areas of 256 by 256 squares in Build 42 (300 by 300 in
Build 41). Each cell is stored as two files that you read together. The `.lotheader` lists every
tile name the cell uses. The matching `world_*.lotpack` places tiles on squares, storing each tile
as a position in that list.

```python
import pzmap

header = pzmap.LotHeader.from_path("0_18.lotheader", 0, 18, pzmap.Build.B42)
cell = pzmap.Cell.from_path(
    "world_0_18.lotpack", 0, 18, pzmap.Build.B42, header.min_layer, header.max_layer
)

for square in cell.squares:
    names = [header.strings[tile] for tile in square.tiles]
    print(square.layer, square.x, square.y, names)
```

`x` and `y` must be the cell coordinates from the file name. The header uses them to convert room
and object positions to world coordinates. Wrong values give a header that parses without error
but puts every room in the wrong place.

`from_path` reads the file itself. Each class also has `parse`, which takes the file's contents as
`bytes` and copies them.

## Bulk access

A cell can hold hundreds of thousands of squares and millions of tile indices, so creating one
Python object per square is slow. The buffer methods return the whole cell at once:

```python
import numpy

tiles = numpy.frombuffer(cell.tiles_bytes(), dtype="<i4")
offsets = numpy.frombuffer(cell.offsets_bytes(), dtype="<u8")

dtype = numpy.dtype([("block", "<u4"), ("layer", "<i4"),
                     ("x", "<u4"), ("y", "<u4"), ("room", "<i4")])
squares = numpy.frombuffer(cell.squares_bytes(), dtype=dtype)

# Tiles of square i are tiles[offsets[i]:offsets[i + 1]].
```

`offsets` has one more entry than there are squares, so the last square needs no special case.

## Builds

Build 41 and Build 42 use different cell sizes, block sizes and layer ranges. Every parser takes
a `Build` argument instead of guessing it from the file. Each file records the version it was
written for, so passing the wrong build raises `ParseError`.

## Errors

A malformed file raises `pzmap.ParseError`, a subclass of `ValueError`. Its `offset` attribute is
the byte offset where the problem was found. Its `format` attribute is the kind of file being read:
`"lotheader"`, `"lotpack"` or `"texture pack"`.

A file that cannot be read raises `OSError` or one of its subclasses, such as `FileNotFoundError`.

## Threads and processes

Parsing releases the GIL, so a thread pool can parse several files at the same time.

`Build`, `LotHeader`, `TexturePack` and `ParseError` can be pickled, so they can be sent to
`multiprocessing` workers. A header or pack pickles as its file bytes and is parsed again when it
is unpickled. `Cell` cannot be pickled, so send the worker the file path instead.

## License

MIT OR Apache-2.0.

The wheel's `.dist-info/licenses/THIRD-PARTY-NOTICES.md` lists the licenses of the Rust crates
compiled into its native module.

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
