"""Type stubs for the compiled ``pzmap._pzmap`` extension.

The docstrings here match the ones compiled into the extension. Change both together.
"""

from os import PathLike
from typing import ClassVar, Iterator, final

__all__ = [
    "Build",
    "Building",
    "Cell",
    "LotHeader",
    "Page",
    "Rectangle",
    "Room",
    "RoomObject",
    "Square",
    "Squares",
    "TexturePack",
    "Texture",
    "__version__",
]

__version__: str

StrPath = str | PathLike[str]

@final
class Build:
    """Which game build a map file was written for.

    Build 41 and Build 42 use different cell sizes, block sizes and layer ranges, so every parser
    needs to know the build. Each file records the version it was written for. Parsing it with
    the wrong build raises ``ParseError``.

    The two values are ``Build.B41`` and ``Build.B42``. This is not an ``enum.Enum``:
    ``int(build)`` gives 0 or 1, and a ``Build`` compares equal to that number.
    """

    B41: ClassVar[Build]
    B42: ClassVar[Build]

    @property
    def name(self) -> str:
        """The attribute name, ``"B41"`` or ``"B42"``."""

    @property
    def display_name(self) -> str:
        """The readable name, such as ``"Build 42"``. ``str(build)`` returns the same."""

    @property
    def version_number(self) -> int:
        """The version number this build writes into ``.lotheader`` and ``.lotpack`` files."""

    @property
    def cell_size_in_blocks(self) -> int:
        """Blocks along one edge of a cell: 30 in Build 41, 32 in Build 42."""

    @property
    def block_size_in_squares(self) -> int:
        """Squares along one edge of a block: 10 in Build 41, 8 in Build 42."""

    @property
    def cell_size_in_squares(self) -> int:
        """Squares along one edge of a cell: 300 in Build 41, 256 in Build 42."""

    @property
    def min_layer(self) -> int:
        """The lowest layer a cell can use in this build.

        A layer is a level of the map. Level 0 is the ground and negative levels are basements.
        """

    @property
    def max_layer(self) -> int:
        """The upper limit of a cell's layer range in this build.

        ``LotHeader.max_layer`` is never above it.
        """

    def __int__(self) -> int: ...

@final
class Rectangle:
    """A rectangle of squares, part of a room's floor plan, in world coordinates."""

    @property
    def x(self) -> int:
        """World X of the rectangle's origin corner."""

    @property
    def y(self) -> int:
        """World Y of the rectangle's origin corner."""

    @property
    def width(self) -> int:
        """Width in squares."""

    @property
    def height(self) -> int:
        """Height in squares."""

@final
class RoomObject:
    """An object placed in a room, in world coordinates."""

    @property
    def type(self) -> int:
        """The game's numeric type for the object. This library passes it through unchanged."""

    @property
    def x(self) -> int:
        """World X of the object."""

    @property
    def y(self) -> int:
        """World Y of the object."""

@final
class Room:
    """One room of a cell, as listed in its ``.lotheader``."""

    @property
    def name(self) -> str:
        """The room's name, such as ``"kitchen"``."""

    @property
    def layer(self) -> int:
        """The level the room is on."""

    @property
    def rectangles(self) -> tuple[Rectangle, ...]:
        """The room's floor plan, as a tuple of ``Rectangle``."""

    @property
    def objects(self) -> tuple[RoomObject, ...]:
        """Objects placed in the room, as a tuple of ``RoomObject``."""

@final
class Building:
    """A building, given as the rooms it contains."""

    @property
    def room_indices(self) -> tuple[int, ...]:
        """The positions of this building's rooms in ``LotHeader.rooms``."""

@final
class LotHeader:
    """A parsed ``.lotheader`` file: the header of one cell.

    A cell is a square part of the map, 256 by 256 squares in Build 42 and 300 by 300 in Build 41.
    Its ``.lotheader`` lists every tile name the cell uses, its rooms and buildings, and the base
    zombie population of each block. The matching ``world_*.lotpack`` places the tiles, and is read
    with ``Cell``.

    Create one with ``LotHeader.from_path`` or ``LotHeader.parse``. The ``strings``, ``rooms`` and
    ``buildings`` tuples are built on first access and then cached, so later reads return the same
    object.

    A ``LotHeader`` can be pickled, for example to send it to a ``multiprocessing`` worker. It
    pickles as the original file bytes, which are parsed again when it is unpickled.
    """

    @staticmethod
    def parse(data: bytes, x: int, y: int, build: Build) -> LotHeader:
        """Parse a ``.lotheader`` from the file's contents.

        ``x`` and ``y`` are the cell coordinates from the file name: ``0_18.lotheader`` is cell
        (0, 18). The parser uses them to convert room and object positions to world coordinates.
        Wrong values give a header that parses without error but puts every room in the wrong
        place.

        ``data`` is copied. When the file is on disk, ``from_path`` avoids that copy and the Python
        ``bytes`` object.

        Raises ``ParseError`` if ``data`` is not a valid ``.lotheader`` for ``build``.
        """

    @staticmethod
    def from_path(path: StrPath, x: int, y: int, build: Build) -> LotHeader:
        """Read and parse a ``.lotheader`` file.

        ``path`` is a ``str`` or ``os.PathLike``. ``x``, ``y`` and ``build`` mean the same as in
        ``parse``.

        Raises ``OSError`` if the file cannot be read, such as ``FileNotFoundError`` with
        ``filename`` set. Raises ``ParseError`` if its contents are not a valid ``.lotheader`` for
        ``build``.
        """

    @property
    def x(self) -> int:
        """The cell's X coordinate, as passed to the constructor."""

    @property
    def y(self) -> int:
        """The cell's Y coordinate, as passed to the constructor."""

    @property
    def version(self) -> int:
        """The version number stored in the file. It always equals ``build.version_number``."""

    @property
    def strings(self) -> tuple[str, ...]:
        """Every distinct tile name the cell uses, as a tuple of ``str``.

        The ``.lotpack`` stores each tile as a position in this tuple, not as a name.
        """

    @property
    def min_layer(self) -> int:
        """The lowest layer the cell uses.

        Pass it to ``Cell.parse`` or ``Cell.from_path`` along with ``max_layer``.
        """

    @property
    def max_layer(self) -> int:
        """One more than the highest layer the cell uses.

        Pass it to ``Cell.parse`` or ``Cell.from_path`` along with ``min_layer``.
        """

    @property
    def rooms(self) -> tuple[Room, ...]:
        """The cell's rooms, as a tuple of ``Room``, in world coordinates."""

    @property
    def buildings(self) -> tuple[Building, ...]:
        """The cell's buildings, as a tuple of ``Building``."""

    @property
    def zpop(self) -> bytes:
        """The base zombie population of every block, one byte per block.

        The byte for block (``block_x``, ``block_y``) is at ``block_x * zpop_side + block_y``. Each
        read returns a new ``bytes`` object.
        """

    @property
    def zpop_side(self) -> int:
        """Blocks along one edge of the zombie population grid.

        The same as ``build.cell_size_in_blocks``.
        """

    @property
    def trailing_bytes(self) -> int:
        """Bytes left over after the last field the parser reads.

        Expected to be 0. Any other value suggests a file layout this library does not fully
        understand.
        """

    def zpop_at(self, block_x: int, block_y: int) -> int:
        """The base zombie population of one block.

        Raises ``IndexError`` if either coordinate is outside the grid.
        """

@final
class Square:
    """One non-empty square of a cell, and the tiles on it.

    A square's position is given as a block number plus a position inside that block. To get
    world coordinates::

        bx, by = divmod(square.block, build.cell_size_in_blocks)
        world_x = cell.x * build.cell_size_in_squares + bx * build.block_size_in_squares + square.x
        world_y = cell.y * build.cell_size_in_squares + by * build.block_size_in_squares + square.y
    """

    @property
    def block(self) -> int:
        """The number of the block the square is in.

        Blocks are numbered ``block_x * build.cell_size_in_blocks + block_y``.
        """

    @property
    def layer(self) -> int:
        """The level the square is on."""

    @property
    def x(self) -> int:
        """The square's X position inside its block."""

    @property
    def y(self) -> int:
        """The square's Y position inside its block."""

    @property
    def room(self) -> int:
        """The room id stored by the map editor, or a negative value for none.

        In Build 42 almost every square has no room id, even inside rooms. The ids that do appear
        are not positions in ``LotHeader.rooms``. To find a square's room, test its world position
        against the room rectangles instead.
        """

    @property
    def tiles(self) -> tuple[int, ...]:
        """The square's tiles, as positions in ``LotHeader.strings``.

        They are in draw order, from bottom to top.
        """

@final
class Squares:
    """The squares of a ``Cell``, as a read-only sequence.

    Indexing or iterating it creates each ``Square`` on demand. It supports ``len()`` and negative
    indices.
    """

    def __len__(self) -> int: ...
    def __getitem__(self, index: int) -> Square: ...
    def __iter__(self) -> Iterator[Square]: ...

@final
class Cell:
    """A parsed ``world_*.lotpack`` file: the tiles on every non-empty square of one cell.

    A cell is split into blocks, and each block into squares. Each square holds a stack of tiles,
    stored as positions in the matching ``LotHeader.strings``. Read the ``.lotheader`` first,
    because it also holds the layer range of this file.

    A cell can hold hundreds of thousands of squares and millions of tile indices. ``squares``
    creates one Python object per square as you read it, which is slow for a whole map.
    ``tiles_bytes``, ``offsets_bytes`` and ``squares_bytes`` return the whole cell as packed
    buffers for ``numpy.frombuffer`` instead.

    ``len(cell)`` is the number of non-empty squares. A ``Cell`` cannot be pickled. To parse one in
    another process, send that process the file path.
    """

    @staticmethod
    def parse(
        data: bytes, x: int, y: int, build: Build, min_layer: int, max_layer: int
    ) -> Cell:
        """Parse a ``world_*.lotpack`` from the file's contents.

        ``x`` and ``y`` are the cell coordinates from the file name. The cell only records them.
        Square positions stay relative to their block.

        ``min_layer`` and ``max_layer`` must be the values from the matching ``LotHeader``,
        because a ``.lotpack`` does not store its own layer range.

        ``data`` is copied. When the file is on disk, ``from_path`` avoids that copy.

        Raises ``ParseError`` if ``data`` is not a valid ``.lotpack`` for ``build`` and the layer
        range.
        """

    @staticmethod
    def from_path(
        path: StrPath, x: int, y: int, build: Build, min_layer: int, max_layer: int
    ) -> Cell:
        """Read and parse a ``world_*.lotpack`` file.

        ``path`` is a ``str`` or ``os.PathLike``. The other arguments mean the same as in
        ``parse``.

        Raises ``OSError`` if the file cannot be read, such as ``FileNotFoundError`` with
        ``filename`` set. Raises ``ParseError`` if its contents are not a valid ``.lotpack``.
        """

    @property
    def x(self) -> int:
        """The cell's X coordinate, as passed to the constructor."""

    @property
    def y(self) -> int:
        """The cell's Y coordinate, as passed to the constructor."""

    def __len__(self) -> int: ...

    @property
    def tile_count(self) -> int:
        """The total number of tile indices across all squares."""

    @property
    def squares(self) -> Squares:
        """Every non-empty square, as a sequence of ``Square``.

        Squares are ordered by block, then layer, then X, then Y. The sequence creates each
        ``Square`` when you index or iterate it, and does not keep it. For a whole map,
        ``squares_bytes`` and ``tiles_bytes`` are much faster.
        """

    def tiles_of(self, index: int) -> tuple[int, ...]:
        """The tiles of square number ``index``, as a tuple of ``int``.

        Raises ``IndexError`` if there is no such square.
        """

    def tiles_bytes(self) -> bytes:
        """Every square's tiles, concatenated, as little-endian ``int32`` values.

        Read it with ``numpy.frombuffer(cell.tiles_bytes(), dtype="<i4")``. ``offsets_bytes`` gives
        where each square's tiles start. Each call builds a new ``bytes`` object.
        """

    def offsets_bytes(self) -> bytes:
        """Where each square's tiles start in ``tiles_bytes``, as little-endian ``uint64`` values.

        There is one more entry than there are squares, and the last entry is the total tile
        count. So the tiles of square ``i`` are ``tiles[offsets[i]:offsets[i + 1]]`` for every
        square, including the last one.

        Read it with ``numpy.frombuffer(cell.offsets_bytes(), dtype="<u8")``.
        """

    def squares_bytes(self) -> bytes:
        """The position and room of every square, packed into one buffer of 20 bytes per square.

        Each entry holds the same fields as ``Square``, without ``tiles``. Read it with::

            dtype = numpy.dtype([("block", "<u4"), ("layer", "<i4"),
                                 ("x", "<u4"), ("y", "<u4"), ("room", "<i4")])
            squares = numpy.frombuffer(cell.squares_bytes(), dtype=dtype)
        """

@final
class Texture:
    """One texture (sprite) on a texture pack page.

    Textures are stored with their transparent margins trimmed off. ``x``, ``y``, ``w`` and ``h``
    give the trimmed rectangle inside the page image. ``ox`` and ``oy`` give where that rectangle
    sits inside the original, untrimmed sprite, which is ``ow`` by ``oh`` pixels. All values are
    in pixels.
    """

    @property
    def name(self) -> str:
        """The texture's name."""

    @property
    def x(self) -> int:
        """X of the trimmed rectangle in the page image."""

    @property
    def y(self) -> int:
        """Y of the trimmed rectangle in the page image."""

    @property
    def w(self) -> int:
        """Width of the trimmed rectangle."""

    @property
    def h(self) -> int:
        """Height of the trimmed rectangle."""

    @property
    def ox(self) -> int:
        """X offset of the trimmed rectangle inside the original sprite."""

    @property
    def oy(self) -> int:
        """Y offset of the trimmed rectangle inside the original sprite."""

    @property
    def ow(self) -> int:
        """Width of the original, untrimmed sprite."""

    @property
    def oh(self) -> int:
        """Height of the original, untrimmed sprite."""

@final
class Page:
    """One page of a texture pack: a PNG image holding many textures.

    A page shares its pack's data instead of copying it. It stays valid after the ``TexturePack``
    is gone.
    """

    @property
    def name(self) -> str:
        """The page's name."""

    @property
    def has_alpha(self) -> bool:
        """Whether the pack says the page's PNG has an alpha channel."""

    @property
    def png(self) -> bytes:
        """The page image, as the bytes of a PNG file. This library does not decode it.

        Each read copies the image into a new ``bytes`` object, so keep the result if you need it
        more than once.
        """

    @property
    def textures(self) -> tuple[Texture, ...]:
        """The textures on this page, as a tuple of ``Texture``."""

@final
class TexturePack:
    """A parsed ``.pack`` file: a texture atlas made of PNG pages and the named textures on each.

    The game draws map tiles from these textures. ``len(pack)`` is the number of pages. The
    ``pages`` tuple, and each page's ``textures`` tuple, are built on first access and then cached.

    A ``TexturePack`` can be pickled. It pickles as the original file bytes, which are parsed again
    when it is unpickled. Packs can be large, so sending a worker the file path may be cheaper.
    Pages cannot be pickled on their own.
    """

    @staticmethod
    def parse(data: bytes) -> TexturePack:
        """Parse a ``.pack`` from the file's contents.

        ``data`` is copied. When the file is on disk, ``from_path`` avoids that copy.

        Raises ``ParseError`` if ``data`` is not a valid ``.pack``.
        """

    @staticmethod
    def from_path(path: StrPath) -> TexturePack:
        """Read and parse a ``.pack`` file.

        ``path`` is a ``str`` or ``os.PathLike``.

        Raises ``OSError`` if the file cannot be read, such as ``FileNotFoundError`` with
        ``filename`` set. Raises ``ParseError`` if its contents are not a valid ``.pack``.
        """

    @property
    def pages(self) -> tuple[Page, ...]:
        """Every page in the pack, in file order, as a tuple of ``Page``."""

    @property
    def texture_count(self) -> int:
        """The total number of textures across all pages."""

    def __len__(self) -> int: ...
