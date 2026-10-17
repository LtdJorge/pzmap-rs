"""Tests for the pzmap Python bindings.

Tests that need real game data run only when ``PZ_INSTALL_DIR`` names a Project Zomboid Build 42
install. Without it they are skipped, and the other tests still run. The game tests check facts
that hold for any Build 42 install, not counts from one version of the game.
"""

import os
import pathlib
import pickle
import struct

import pytest

import pzmap

NO_INSTALL = "set PZ_INSTALL_DIR to a Project Zomboid Build 42 installation"
INSTALL = pathlib.Path(os.environ["PZ_INSTALL_DIR"]) if os.environ.get("PZ_INSTALL_DIR") else None
MAP_DIR = INSTALL / "media" / "maps" / "Muldraugh, KY" if INSTALL else None
PACK = INSTALL / "media" / "texturepacks" / "Tiles2x.pack" if INSTALL else None

# Cell 49,6 of vanilla Muldraugh, a town cell with rooms and buildings.
CELL_X, CELL_Y = 49, 6

if INSTALL is None:
    needs_game = needs_pack = pytest.mark.skip(reason=NO_INSTALL)
else:
    needs_game = pytest.mark.skipif(
        not MAP_DIR.is_dir(), reason=f"no Muldraugh map under {INSTALL}"
    )
    needs_pack = pytest.mark.skipif(not PACK.is_file(), reason=f"no Tiles2x.pack under {INSTALL}")


def lotheader_path():
    return MAP_DIR / f"{CELL_X}_{CELL_Y}.lotheader"


def lotpack_path():
    return MAP_DIR / f"world_{CELL_X}_{CELL_Y}.lotpack"


def load_cell():
    header = pzmap.LotHeader.from_path(lotheader_path(), CELL_X, CELL_Y, pzmap.Build.B42)
    cell = pzmap.Cell.from_path(
        lotpack_path(), CELL_X, CELL_Y, pzmap.Build.B42, header.min_layer, header.max_layer
    )
    return header, cell


def test_build():
    assert pzmap.Build.B42.name == "B42"
    assert pzmap.Build.B42.display_name == "Build 42"
    assert pzmap.Build.B41.name == "B41"
    assert pzmap.Build.B42.cell_size_in_blocks == 32
    assert pzmap.Build.B41.cell_size_in_blocks == 30

    # A cell's size in squares is its size in blocks times the block size.
    for build in (pzmap.Build.B41, pzmap.Build.B42):
        assert build.cell_size_in_blocks * build.block_size_in_squares == (
            build.cell_size_in_squares
        )
        assert pickle.loads(pickle.dumps(build)) is build


def test_parse_error_is_structured():
    """A malformed file raises ParseError with its offset and format set."""
    with pytest.raises(pzmap.ParseError) as excinfo:
        # "PZPK", file version 9, zero pages. This library does not support version 9.
        pzmap.TexturePack.parse(b"PZPK" + (9).to_bytes(4, "little") + bytes(4))

    error = excinfo.value
    assert type(error).__name__ == "ParseError"
    assert isinstance(error, ValueError)
    assert error.format == "texture pack"
    assert error.offset == 4
    assert "9" in str(error)

    copy = pickle.loads(pickle.dumps(error))
    assert (str(copy), copy.offset, copy.format) == (str(error), error.offset, error.format)


def test_missing_file_raises_oserror():
    with pytest.raises(FileNotFoundError):
        pzmap.TexturePack.from_path("/nonexistent/does-not-exist.pack")


def one_page_pack():
    """A pack with one page named "p", no textures and a four-byte image."""
    return (
        b"PZPK"
        + struct.pack("<III", 1, 1, 1)
        + b"p"
        + struct.pack("<III", 0, 1, 4)
        + b"\x89PNG"
    )


def test_texture_pack_pickles_and_pages_outlive_it():
    pack = pzmap.TexturePack.parse(one_page_pack())
    page = pack.pages[0]

    copy = pickle.loads(pickle.dumps(pack))
    assert [p.name for p in copy.pages] == ["p"]

    del pack
    assert page.png == b"\x89PNG"


def test_package_has_a_version():
    assert isinstance(pzmap.__version__, str) and pzmap.__version__


@needs_game
def test_lotheader():
    header = pzmap.LotHeader.from_path(lotheader_path(), CELL_X, CELL_Y, pzmap.Build.B42)

    assert header.x == CELL_X
    assert header.y == CELL_Y
    assert len(header.strings) > 0
    assert len(header.rooms) > 0
    assert len(header.buildings) > 0
    assert pzmap.Build.B42.min_layer <= header.min_layer <= header.max_layer
    assert header.max_layer <= pzmap.Build.B42.max_layer
    assert header.trailing_bytes == 0

    # Tuples are cached, so repeated access returns the same object.
    assert header.strings is header.strings
    assert header.rooms is header.rooms

    # Every building has to name rooms that exist.
    for building in header.buildings:
        assert all(0 <= room < len(header.rooms) for room in building.room_indices)

    # Build 42's zombie population grid is 32 by 32 blocks, one byte per block.
    assert header.zpop_side == 32
    assert len(header.zpop) == 32 * 32
    assert header.zpop_at(0, 0) == header.zpop[0]
    assert header.zpop_at(1, 2) == header.zpop[1 * 32 + 2]
    with pytest.raises(IndexError):
        header.zpop_at(32, 0)

    room = header.rooms[0]
    assert isinstance(room.name, str)
    assert len(room.rectangles) > 0


@needs_game
def test_parse_from_bytes_and_pickle_match_from_path():
    from_path = pzmap.LotHeader.from_path(lotheader_path(), CELL_X, CELL_Y, pzmap.Build.B42)
    from_bytes = pzmap.LotHeader.parse(
        lotheader_path().read_bytes(), CELL_X, CELL_Y, pzmap.Build.B42
    )
    unpickled = pickle.loads(pickle.dumps(from_path))

    for header in (from_bytes, unpickled):
        assert (header.x, header.y) == (from_path.x, from_path.y)
        assert header.strings == from_path.strings
        assert len(header.rooms) == len(from_path.rooms)


@needs_game
def test_cell():
    header, cell = load_cell()

    assert len(cell) > 0

    squares = cell.squares
    assert len(squares) == len(cell)
    assert squares[-1].tiles == squares[len(cell) - 1].tiles
    with pytest.raises(IndexError):
        squares[len(cell)]

    # Every tile index must be a valid position in the header's string table.
    count = 0
    for square in squares:
        assert all(0 <= tile < len(header.strings) for tile in square.tiles)
        count += len(square.tiles)
    assert count == cell.tile_count

    assert cell.tiles_of(0) == squares[0].tiles
    with pytest.raises(IndexError):
        cell.tiles_of(len(cell))


@needs_game
def test_cell_buffers_agree_with_the_objects():
    """The bulk buffers hold the same data as the per-square objects."""
    _, cell = load_cell()
    size = len(cell)

    tiles = cell.tiles_bytes()
    offsets = cell.offsets_bytes()
    squares_buf = cell.squares_bytes()

    assert len(tiles) == cell.tile_count * 4
    assert len(offsets) == (size + 1) * 8
    assert len(squares_buf) == size * 20

    decoded_offsets = struct.unpack(f"<{size + 1}Q", offsets)
    assert decoded_offsets[0] == 0
    assert decoded_offsets[-1] == cell.tile_count

    # Check a few squares against the objects. The last square is included because an off-by-one
    # error in the offsets would show up there.
    for index in (0, 1, size // 2, size - 1):
        start, end = decoded_offsets[index], decoded_offsets[index + 1]
        from_buffer = struct.unpack(f"<{end - start}i", tiles[start * 4 : end * 4])
        assert from_buffer == cell.tiles_of(index)

        block, layer, x, y, room = struct.unpack(
            "<IiIIi", squares_buf[index * 20 : (index + 1) * 20]
        )
        square = cell.squares[index]
        assert (block, layer, x, y, room) == (
            square.block,
            square.layer,
            square.x,
            square.y,
            square.room,
        )


@needs_pack
def test_texture_pack():
    pack = pzmap.TexturePack.from_path(PACK)

    assert len(pack) > 0
    assert pack.pages is pack.pages
    assert len(pack.pages) == len(pack)
    assert pack.texture_count == sum(len(page.textures) for page in pack.pages)

    page = pack.pages[0]
    assert isinstance(page.name, str)
    # Pages are returned as undecoded PNG files, so they start with the PNG signature.
    assert page.png.startswith(b"\x89PNG\r\n\x1a\n")
    assert len(page.textures) > 0
    assert isinstance(page.textures[0].name, str)
