"""Parsers for Project Zomboid's map files.

This package reads three file formats from the game's ``media`` folder:

- ``.lotheader`` files, with ``LotHeader``.
- ``world_*.lotpack`` files, with ``Cell``.
- ``.pack`` texture atlases, with ``TexturePack``.

It does not decode images or draw anything. A texture page comes back as the bytes of a PNG
file, and a square's tiles come back as positions in a list of tile names.

Terms
-----
The map is divided into cells. A cell is 256 by 256 squares in Build 42 and 300 by 300 in
Build 41. Each cell is stored as two files named after its coordinates, such as
``0_18.lotheader`` and ``world_0_18.lotpack``. A cell is split into blocks of 8 by 8 squares
(10 by 10 in Build 41). A square is one tile position on the floor, and it can hold a stack of
tiles. Layers are the map's levels. Level 0 is the ground and negative levels are basements. World
coordinates count squares from the origin of the whole map.

Reading a cell
--------------
The ``.lotheader`` lists every tile name the cell uses. The ``.lotpack`` places tiles on squares,
storing each tile as a position in that list. Read the header first, because the ``.lotpack``
also needs its layer range::

    import pzmap

    header = pzmap.LotHeader.from_path("0_18.lotheader", 0, 18, pzmap.Build.B42)
    cell = pzmap.Cell.from_path(
        "world_0_18.lotpack", 0, 18, pzmap.Build.B42, header.min_layer, header.max_layer
    )

    for square in cell.squares:
        names = [header.strings[tile] for tile in square.tiles]

Paths or bytes
--------------
Each class has two constructors. ``from_path`` takes a ``str`` or ``os.PathLike`` and reads the
file itself. ``parse`` takes the file's contents as ``bytes`` and copies them. When the file is on
disk, ``from_path`` is the better choice because it never creates a Python ``bytes`` object. Both
release the GIL while they read and parse, so a thread pool can parse several files at once.

Large cells
-----------
A cell can hold hundreds of thousands of squares, so creating one Python object per square is
slow. The buffer methods return a whole cell at once, ready for NumPy::

    import numpy

    tiles = numpy.frombuffer(cell.tiles_bytes(), dtype="<i4")
    offsets = numpy.frombuffer(cell.offsets_bytes(), dtype="<u8")
    # The tiles of square i are tiles[offsets[i]:offsets[i + 1]].

Caching
-------
Tuple properties such as ``LotHeader.strings`` and ``TexturePack.pages`` are built on first
access and then cached, so later reads return the same object. Properties and methods that
return ``bytes``, such as ``Page.png`` and ``Cell.tiles_bytes()``, build a new object every time.

Errors
------
A malformed file raises ``ParseError``, a subclass of ``ValueError``. A file that cannot be read
raises ``OSError`` or one of its subclasses, such as ``FileNotFoundError``, with ``filename`` set.
Each file records the game version it was written for, so parsing it with the wrong ``Build``
also raises ``ParseError``.

Pickling
--------
``Build``, ``LotHeader``, ``TexturePack`` and ``ParseError`` can be pickled, so they can be
passed to ``multiprocessing`` workers. A header or pack pickles as its file bytes and is parsed
again when it is unpickled. ``Cell`` and the other classes cannot be pickled. Send the worker the
file path instead.
"""

from ._pzmap import (
    Build,
    Building,
    Cell,
    LotHeader,
    Page,
    Rectangle,
    Room,
    RoomObject,
    Square,
    Squares,
    TexturePack,
    Texture,
    __version__,
)


class ParseError(ValueError):
    """Raised when a map file is malformed or not supported.

    ``offset`` and ``format`` give where the problem was found and in which kind of file, as
    separate values from the message. The exception can be pickled, so it can be raised in a
    ``multiprocessing`` worker and caught in the parent process.
    """

    offset: int
    """The byte offset in the file where the problem was found."""

    format: str
    """The kind of file being read: ``"lotheader"``, ``"lotpack"`` or ``"texture pack"``."""

    def __init__(self, message: str, offset: int, format: str) -> None:
        # Pickling rebuilds the exception from ``args``, so all three must go there.
        super().__init__(message, offset, format)
        self.offset = offset
        self.format = format

    def __str__(self) -> str:
        return str(self.args[0])


__all__ = [
    "Build",
    "Building",
    "Cell",
    "LotHeader",
    "Page",
    "ParseError",
    "Rectangle",
    "Room",
    "RoomObject",
    "Square",
    "Squares",
    "TexturePack",
    "Texture",
    "__version__",
]
