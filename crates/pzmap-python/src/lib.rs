//! Python bindings for [`libpzmap`], built with pyo3.
//!
//! This crate compiles to `pzmap._pzmap`, the native module inside the `pzmap` Python package.
//! The package's `__init__.py` re-exports its classes and defines `pzmap.ParseError`.
//!
//! Most of libpzmap's parsed types borrow from the buffer they were parsed from. A Python class
//! cannot hold a borrow, so the classes here wrap the owning variants instead: [`AnyLotHeader`],
//! [`AnyCell`] and [`OwnedTexturePack`]. Each keeps its input buffer next to the parsed data.
//!
//! Parsing runs with the GIL released. The `from_path` constructors read the file in Rust, so its
//! contents never become a Python object.
//!
//! The `///` comments on Python-visible items become Python docstrings, so they are written for
//! Python users. Keep them in step with `python/pzmap/_pzmap.pyi`.

use libpzmap::{
    Build as NativeBuild,
    lotheader::AnyLotHeader,
    lotpack::AnyCell,
    texturepack::{
        OwnedTexturePack,
        Page as NativePage,
    },
};
use pyo3::{
    exceptions::{
        PyIndexError,
        PyOSError,
        PyValueError,
    },
    prelude::*,
    sync::PyOnceLock,
    types::{
        PyBytes,
        PyTuple,
        PyType,
    },
};
use std::{
    path::PathBuf,
    sync::Arc,
};
use yoke::Yoke;

/// The `pzmap.ParseError` class, looked up on first use.
///
/// It is defined in Python because, under the abi3 ABI, a Rust class can only subclass an
/// exception from Python 3.12 on.
static PARSE_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The ways a constructor can fail. Converts into the matching Python exception.
enum Error {
    /// The file could not be read.
    Io(PathBuf, std::io::Error),
    /// The data did not parse.
    Parse(libpzmap::ParseError),
}

impl From<libpzmap::ParseError> for Error {
    fn from(error: libpzmap::ParseError) -> Error {
        Error::Parse(error)
    }
}

impl From<Error> for PyErr {
    fn from(error: Error) -> PyErr {
        match error {
            // Called with `(errno, message, filename)`, `OSError` creates the matching subclass,
            // such as `FileNotFoundError`, and sets its `filename` attribute.
            Error::Io(path, error) => match error.raw_os_error() {
                Some(code) => PyOSError::new_err((code, error.to_string(), path)),
                None => PyOSError::new_err(format!("{}: {error}", path.display())),
            },
            Error::Parse(error) => {
                Python::attach(|py| match PARSE_ERROR.import(py, "pzmap", "ParseError") {
                    Ok(class) => PyErr::from_type(
                        class.clone(),
                        (error.to_string(), error.at, error.format.name()),
                    ),
                    Err(import_error) => import_error,
                })
            }
        }
    }
}

/// Reads a whole file, keeping the path for the error.
fn read(path: PathBuf) -> Result<Vec<u8>, Error> {
    std::fs::read(&path).map_err(|error| Error::Io(path, error))
}

/// What `__reduce__` returns: a callable and the arguments that rebuild the object.
type Reduce<'py, Args> = PyResult<(Bound<'py, PyAny>, Args)>;

/// Returns the tuple in `cache`, building it from `items` on first use.
fn cached<'py, T: IntoPyObject<'py>>(
    py: Python<'py>,
    cache: &PyOnceLock<Py<PyTuple>>,
    items: impl FnOnce() -> Vec<T>,
) -> PyResult<Bound<'py, PyTuple>> {
    let tuple = cache.get_or_try_init(py, || PyTuple::new(py, items()).map(Bound::unbind))?;
    Ok(tuple.bind(py).clone())
}

/// Which game build a map file was written for.
///
/// Build 41 and Build 42 use different cell sizes, block sizes and layer ranges, so every parser
/// needs to know the build. Each file records the version it was written for. Parsing it with
/// the wrong build raises `ParseError`.
///
/// The two values are `Build.B41` and `Build.B42`. This is not an `enum.Enum`: `int(build)` gives
/// 0 or 1, and a `Build` compares equal to that number.
#[pyclass(eq, eq_int, frozen, from_py_object, module = "pzmap")]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Build {
    /// Project Zomboid Build 41.
    B41,
    /// Project Zomboid Build 42.
    B42,
}

impl From<Build> for NativeBuild {
    fn from(build: Build) -> NativeBuild {
        match build {
            Build::B41 => NativeBuild::B41,
            Build::B42 => NativeBuild::B42,
        }
    }
}

#[pymethods]
impl Build {
    /// The attribute name, `"B41"` or `"B42"`.
    #[getter]
    fn name(&self) -> &'static str {
        match self {
            Build::B41 => "B41",
            Build::B42 => "B42",
        }
    }

    /// The readable name, such as `"Build 42"`. `str(build)` returns the same.
    #[getter]
    fn display_name(&self) -> &'static str {
        NativeBuild::from(*self).name()
    }

    /// The version number this build writes into `.lotheader` and `.lotpack` files.
    #[getter]
    fn version_number(&self) -> u32 {
        NativeBuild::from(*self).version_number()
    }

    /// Blocks along one edge of a cell: 30 in Build 41, 32 in Build 42.
    #[getter]
    fn cell_size_in_blocks(&self) -> usize {
        NativeBuild::from(*self).cell_size_in_blocks()
    }

    /// Squares along one edge of a block: 10 in Build 41, 8 in Build 42.
    #[getter]
    fn block_size_in_squares(&self) -> usize {
        NativeBuild::from(*self).block_size_in_squares()
    }

    /// Squares along one edge of a cell: 300 in Build 41, 256 in Build 42.
    #[getter]
    fn cell_size_in_squares(&self) -> i32 {
        NativeBuild::from(*self).cell_size_in_squares()
    }

    /// The lowest layer a cell can use in this build.
    ///
    /// A layer is a level of the map. Level 0 is the ground and negative levels are basements.
    #[getter]
    fn min_layer(&self) -> i32 {
        NativeBuild::from(*self).min_layer()
    }

    /// The upper limit of a cell's layer range in this build.
    ///
    /// `LotHeader.max_layer` is never above it.
    #[getter]
    fn max_layer(&self) -> i32 {
        NativeBuild::from(*self).max_layer()
    }

    fn __str__(&self) -> &'static str {
        self.display_name()
    }

    /// Support pickling. Unpickling returns the same `Build.B41` or `Build.B42` object.
    fn __reduce__<'py>(&self, py: Python<'py>) -> Reduce<'py, (Bound<'py, PyAny>, &'static str)> {
        let getattr = py.import("builtins")?.getattr("getattr")?;
        Ok((getattr, (py.get_type::<Build>().into_any(), self.name())))
    }
}

/// A rectangle of squares, part of a room's floor plan, in world coordinates.
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct Rectangle {
    /// World X of the rectangle's origin corner.
    pub x: i32,
    /// World Y of the rectangle's origin corner.
    pub y: i32,
    /// Width in squares.
    pub width: i32,
    /// Height in squares.
    pub height: i32,
}

#[pymethods]
impl Rectangle {
    fn __repr__(&self) -> String {
        format!(
            "Rectangle(x={}, y={}, width={}, height={})",
            self.x, self.y, self.width, self.height
        )
    }
}

/// An object placed in a room, in world coordinates.
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct RoomObject {
    /// The game's numeric type for the object. This library passes it through unchanged.
    #[pyo3(name = "type")]
    pub object_type: i32,
    /// World X of the object.
    pub x: i32,
    /// World Y of the object.
    pub y: i32,
}

#[pymethods]
impl RoomObject {
    fn __repr__(&self) -> String {
        format!(
            "RoomObject(type={}, x={}, y={})",
            self.object_type, self.x, self.y
        )
    }
}

/// One room of a cell, as listed in its `.lotheader`.
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct Room {
    /// The room's name, such as `"kitchen"`.
    pub name: String,
    /// The level the room is on.
    pub layer: i32,
    /// The room's floor plan, as a tuple of `Rectangle`.
    pub rectangles: Py<PyTuple>,
    /// Objects placed in the room, as a tuple of `RoomObject`.
    pub objects: Py<PyTuple>,
}

#[pymethods]
impl Room {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "Room(name={:?}, layer={}, rectangles={}, objects={})",
            self.name,
            self.layer,
            self.rectangles.bind(py).len(),
            self.objects.bind(py).len()
        )
    }
}

/// A building, given as the rooms it contains.
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct Building {
    /// The positions of this building's rooms in `LotHeader.rooms`.
    pub room_indices: Py<PyTuple>,
}

#[pymethods]
impl Building {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!("Building(rooms={})", self.room_indices.bind(py).len())
    }
}

/// A parsed `.lotheader` file: the header of one cell.
///
/// A cell is a square part of the map, 256 by 256 squares in Build 42 and 300 by 300 in Build 41.
/// Its `.lotheader` lists every tile name the cell uses, its rooms and buildings, and the base
/// zombie population of each block. The matching `world_*.lotpack` places the tiles, and is read
/// with `Cell`.
///
/// Create one with `LotHeader.from_path` or `LotHeader.parse`. The `strings`, `rooms` and
/// `buildings` tuples are built on first access and then cached, so later reads return the same
/// object.
///
/// A `LotHeader` can be pickled, for example to send it to a `multiprocessing` worker. It pickles
/// as the original file bytes, which are parsed again when it is unpickled.
#[pyclass(frozen, module = "pzmap")]
pub struct LotHeader {
    inner: AnyLotHeader,
    strings: PyOnceLock<Py<PyTuple>>,
    rooms: PyOnceLock<Py<PyTuple>>,
    buildings: PyOnceLock<Py<PyTuple>>,
}

impl From<AnyLotHeader> for LotHeader {
    fn from(inner: AnyLotHeader) -> LotHeader {
        LotHeader {
            inner,
            strings: PyOnceLock::new(),
            rooms: PyOnceLock::new(),
            buildings: PyOnceLock::new(),
        }
    }
}

#[pymethods]
impl LotHeader {
    /// Parse a `.lotheader` from the file's contents.
    ///
    /// `x` and `y` are the cell coordinates from the file name: `0_18.lotheader` is cell (0, 18).
    /// The parser uses them to convert room and object positions to world coordinates. Wrong
    /// values give a header that parses without error but puts every room in the wrong place.
    ///
    /// `data` is copied. When the file is on disk, `from_path` avoids that copy and the Python
    /// `bytes` object.
    ///
    /// Raises `ParseError` if `data` is not a valid `.lotheader` for `build`.
    #[staticmethod]
    fn parse(
        py: Python<'_>,
        data: Vec<u8>,
        x: i32,
        y: i32,
        build: Build,
    ) -> Result<LotHeader, Error> {
        py.detach(|| Ok(AnyLotHeader::parse(build.into(), x, y, data)?.into()))
    }

    /// Read and parse a `.lotheader` file.
    ///
    /// `path` is a `str` or `os.PathLike`. `x`, `y` and `build` mean the same as in `parse`.
    ///
    /// Raises `OSError` if the file cannot be read, such as `FileNotFoundError` with `filename`
    /// set. Raises `ParseError` if its contents are not a valid `.lotheader` for `build`.
    #[staticmethod]
    fn from_path(
        py: Python<'_>,
        path: PathBuf,
        x: i32,
        y: i32,
        build: Build,
    ) -> Result<LotHeader, Error> {
        py.detach(|| Ok(AnyLotHeader::parse(build.into(), x, y, read(path)?)?.into()))
    }

    /// The cell's X coordinate, as passed to the constructor.
    #[getter]
    fn x(&self) -> i32 {
        self.inner.x()
    }

    /// The cell's Y coordinate, as passed to the constructor.
    #[getter]
    fn y(&self) -> i32 {
        self.inner.y()
    }

    /// The version number stored in the file. It always equals `build.version_number`.
    #[getter]
    fn version(&self) -> u32 {
        self.inner.version()
    }

    /// Every distinct tile name the cell uses, as a tuple of `str`.
    ///
    /// The `.lotpack` stores each tile as a position in this tuple, not as a name.
    #[getter]
    fn strings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        cached(py, &self.strings, || self.inner.strings().to_vec())
    }

    /// The lowest layer the cell uses.
    ///
    /// Pass it to `Cell.parse` or `Cell.from_path` along with `max_layer`.
    #[getter]
    fn min_layer(&self) -> i32 {
        self.inner.min_layer()
    }

    /// One more than the highest layer the cell uses.
    ///
    /// Pass it to `Cell.parse` or `Cell.from_path` along with `min_layer`.
    #[getter]
    fn max_layer(&self) -> i32 {
        self.inner.max_layer()
    }

    /// The cell's rooms, as a tuple of `Room`, in world coordinates.
    #[getter]
    fn rooms<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let rooms = self.inner.rooms();
        let tuple = self.rooms.get_or_try_init(py, || -> PyResult<_> {
            let mut out = Vec::with_capacity(rooms.len());
            for room in rooms {
                let rectangles = room.rectangles.iter().map(|r| Rectangle {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                });
                let objects = room.objects.iter().map(|o| RoomObject {
                    object_type: o.kind,
                    x: o.x,
                    y: o.y,
                });
                out.push(Room {
                    name: room.name.to_owned(),
                    layer: room.layer,
                    rectangles: PyTuple::new(py, rectangles)?.unbind(),
                    objects: PyTuple::new(py, objects)?.unbind(),
                });
            }
            Ok(PyTuple::new(py, out)?.unbind())
        })?;
        Ok(tuple.bind(py).clone())
    }

    /// The cell's buildings, as a tuple of `Building`.
    #[getter]
    fn buildings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        let buildings = self.inner.buildings();
        let tuple = self.buildings.get_or_try_init(py, || -> PyResult<_> {
            let mut out = Vec::with_capacity(buildings.len());
            for building in buildings {
                out.push(Building {
                    room_indices: PyTuple::new(py, &building.room_indices)?.unbind(),
                });
            }
            Ok(PyTuple::new(py, out)?.unbind())
        })?;
        Ok(tuple.bind(py).clone())
    }

    /// Blocks along one edge of the zombie population grid.
    ///
    /// The same as `build.cell_size_in_blocks`.
    #[getter]
    fn zpop_side(&self) -> usize {
        self.inner.zpop_side()
    }

    /// The base zombie population of every block, one byte per block.
    ///
    /// The byte for block (`block_x`, `block_y`) is at `block_x * zpop_side + block_y`. Each read
    /// returns a new `bytes` object.
    #[getter]
    fn zpop<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.zpop())
    }

    /// The base zombie population of one block.
    ///
    /// Raises `IndexError` if either coordinate is outside the grid.
    fn zpop_at(&self, block_x: usize, block_y: usize) -> PyResult<u8> {
        self.inner.zpop_at(block_x, block_y).ok_or_else(|| {
            let side = self.inner.zpop_side();
            PyIndexError::new_err(format!(
                "block {block_x},{block_y} is outside a {side}x{side} grid"
            ))
        })
    }

    /// Bytes left over after the last field the parser reads.
    ///
    /// Expected to be 0. Any other value suggests a file layout this library does not fully
    /// understand.
    #[getter]
    fn trailing_bytes(&self) -> usize {
        self.inner.trailing_bytes()
    }

    /// Support pickling. The header pickles as its file bytes and is parsed again when unpickled.
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> Reduce<'py, (Bound<'py, PyBytes>, i32, i32, Build)> {
        let py = slf.py();
        let header = &slf.get().inner;
        let build = match header.build() {
            NativeBuild::B41 => Build::B41,
            NativeBuild::B42 => Build::B42,
            other => {
                return Err(PyValueError::new_err(format!(
                    "{other} has no pzmap.Build constant"
                )));
            }
        };
        let bytes = header.bytes();
        let parse = slf.get_type().getattr("parse")?;
        Ok((
            parse,
            (PyBytes::new(py, bytes), header.x(), header.y(), build),
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "LotHeader(x={}, y={}, strings={}, rooms={}, buildings={})",
            self.inner.x(),
            self.inner.y(),
            self.inner.strings().len(),
            self.inner.rooms().len(),
            self.inner.buildings().len()
        )
    }
}

/// One non-empty square of a cell, and the tiles on it.
///
/// A square's position is given as a block number plus a position inside that block. To get
/// world coordinates:
///
/// ```python
/// bx, by = divmod(square.block, build.cell_size_in_blocks)
/// world_x = cell.x * build.cell_size_in_squares + bx * build.block_size_in_squares + square.x
/// world_y = cell.y * build.cell_size_in_squares + by * build.block_size_in_squares + square.y
/// ```
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct Square {
    /// The number of the block the square is in.
    ///
    /// Blocks are numbered `block_x * build.cell_size_in_blocks + block_y`.
    pub block: usize,
    /// The level the square is on.
    pub layer: i32,
    /// The square's X position inside its block.
    pub x: usize,
    /// The square's Y position inside its block.
    pub y: usize,
    /// The room id stored by the map editor, or a negative value for none.
    ///
    /// In Build 42 almost every square has no room id, even inside rooms. The ids that do appear
    /// are not positions in `LotHeader.rooms`. To find a square's room, test its world position
    /// against the room rectangles instead.
    pub room: i32,
    /// The square's tiles, as positions in `LotHeader.strings`.
    ///
    /// They are in draw order, from bottom to top.
    pub tiles: Py<PyTuple>,
}

#[pymethods]
impl Square {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "Square(block={}, layer={}, x={}, y={}, room={}, tiles={})",
            self.block,
            self.layer,
            self.x,
            self.y,
            self.room,
            self.tiles.bind(py).len()
        )
    }
}

/// A parsed `world_*.lotpack` file: the tiles on every non-empty square of one cell.
///
/// A cell is split into blocks, and each block into squares. Each square holds a stack of tiles,
/// stored as positions in the matching `LotHeader.strings`. Read the `.lotheader` first, because
/// it also holds the layer range of this file.
///
/// A cell can hold hundreds of thousands of squares and millions of tile indices. `squares`
/// creates one Python object per square as you read it, which is slow for a whole map.
/// `tiles_bytes`, `offsets_bytes` and `squares_bytes` return the whole cell as packed buffers for
/// `numpy.frombuffer` instead.
///
/// `len(cell)` is the number of non-empty squares. A `Cell` cannot be pickled. To parse one in
/// another process, send that process the file path.
#[pyclass(frozen, module = "pzmap")]
pub struct Cell {
    inner: AnyCell,
}

#[pymethods]
impl Cell {
    /// Parse a `world_*.lotpack` from the file's contents.
    ///
    /// `x` and `y` are the cell coordinates from the file name. The cell only records them.
    /// Square positions stay relative to their block.
    ///
    /// `min_layer` and `max_layer` must be the values from the matching `LotHeader`, because a
    /// `.lotpack` does not store its own layer range.
    ///
    /// `data` is copied. When the file is on disk, `from_path` avoids that copy.
    ///
    /// Raises `ParseError` if `data` is not a valid `.lotpack` for `build` and the layer range.
    #[staticmethod]
    fn parse(
        py: Python<'_>,
        data: Vec<u8>,
        x: i32,
        y: i32,
        build: Build,
        min_layer: i32,
        max_layer: i32,
    ) -> Result<Cell, Error> {
        py.detach(|| {
            let inner = AnyCell::parse(build.into(), x, y, &data, min_layer, max_layer)?;
            Ok(Cell { inner })
        })
    }

    /// Read and parse a `world_*.lotpack` file.
    ///
    /// `path` is a `str` or `os.PathLike`. The other arguments mean the same as in `parse`.
    ///
    /// Raises `OSError` if the file cannot be read, such as `FileNotFoundError` with `filename`
    /// set. Raises `ParseError` if its contents are not a valid `.lotpack`.
    #[staticmethod]
    fn from_path(
        py: Python<'_>,
        path: PathBuf,
        x: i32,
        y: i32,
        build: Build,
        min_layer: i32,
        max_layer: i32,
    ) -> Result<Cell, Error> {
        py.detach(|| {
            let data = read(path)?;
            let inner = AnyCell::parse(build.into(), x, y, &data, min_layer, max_layer)?;
            Ok(Cell { inner })
        })
    }

    /// The cell's X coordinate, as passed to the constructor.
    #[getter]
    fn x(&self) -> i32 {
        self.inner.x()
    }

    /// The cell's Y coordinate, as passed to the constructor.
    #[getter]
    fn y(&self) -> i32 {
        self.inner.y()
    }

    /// The number of non-empty squares.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// The total number of tile indices across all squares.
    #[getter]
    fn tile_count(&self) -> usize {
        self.inner.tiles().len()
    }

    /// Every non-empty square, as a sequence of `Square`.
    ///
    /// Squares are ordered by block, then layer, then X, then Y. The sequence creates each
    /// `Square` when you index or iterate it, and does not keep it. For a whole map,
    /// `squares_bytes` and `tiles_bytes` are much faster.
    #[getter]
    fn squares(slf: &Bound<'_, Self>) -> Squares {
        Squares {
            cell: slf.clone().unbind(),
        }
    }

    /// The tiles of square number `index`, as a tuple of `int`.
    ///
    /// Raises `IndexError` if there is no such square.
    fn tiles_of<'py>(&self, py: Python<'py>, index: usize) -> PyResult<Bound<'py, PyTuple>> {
        let tiles = self
            .inner
            .tiles_of(index)
            .ok_or_else(|| PyIndexError::new_err("square index out of range"))?;
        PyTuple::new(py, tiles)
    }

    /// Every square's tiles, concatenated, as little-endian `int32` values.
    ///
    /// Read it with `numpy.frombuffer(cell.tiles_bytes(), dtype="<i4")`. `offsets_bytes` gives
    /// where each square's tiles start. Each call builds a new `bytes` object.
    fn tiles_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let mut out = Vec::with_capacity(self.inner.tiles().len() * 4);
        for tile in self.inner.tiles() {
            out.extend_from_slice(&tile.to_le_bytes());
        }
        PyBytes::new(py, &out)
    }

    /// Where each square's tiles start in `tiles_bytes`, as little-endian `uint64` values.
    ///
    /// There is one more entry than there are squares, and the last entry is the total tile
    /// count. So the tiles of square `i` are `tiles[offsets[i]:offsets[i + 1]]` for every square,
    /// including the last one.
    ///
    /// Read it with `numpy.frombuffer(cell.offsets_bytes(), dtype="<u8")`.
    fn offsets_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let mut out = Vec::with_capacity(self.inner.offsets().len() * 8);
        for offset in self.inner.offsets() {
            // A `usize` fits in a `u64` on every supported platform, so the fallback never runs.
            out.extend_from_slice(&u64::try_from(*offset).unwrap_or(u64::MAX).to_le_bytes());
        }
        PyBytes::new(py, &out)
    }

    /// The position and room of every square, packed into one buffer of 20 bytes per square.
    ///
    /// Each entry holds the same fields as `Square`, without `tiles`. Read it with:
    ///
    /// ```python
    /// dtype = numpy.dtype([("block", "<u4"), ("layer", "<i4"),
    ///                      ("x", "<u4"), ("y", "<u4"), ("room", "<i4")])
    /// squares = numpy.frombuffer(cell.squares_bytes(), dtype=dtype)
    /// ```
    fn squares_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let mut out = Vec::with_capacity(self.inner.squares().len() * 20);
        for at in self.inner.squares() {
            // libpzmap has already checked these against the cell size: at most 1024 blocks and
            // 10 squares along a block edge. They always fit in a `u32`, so the fallback never
            // runs.
            let narrow = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
            out.extend_from_slice(&narrow(at.block).to_le_bytes());
            out.extend_from_slice(&at.layer.to_le_bytes());
            out.extend_from_slice(&narrow(at.x).to_le_bytes());
            out.extend_from_slice(&narrow(at.y).to_le_bytes());
            out.extend_from_slice(&at.room.to_le_bytes());
        }
        PyBytes::new(py, &out)
    }

    fn __repr__(&self) -> String {
        format!(
            "Cell(x={}, y={}, squares={}, tiles={})",
            self.inner.x(),
            self.inner.y(),
            self.inner.len(),
            self.inner.tiles().len()
        )
    }
}

/// The squares of a `Cell`, as a read-only sequence.
///
/// Indexing or iterating it creates each `Square` on demand. It supports `len()` and negative
/// indices.
#[pyclass(frozen, sequence, module = "pzmap")]
pub struct Squares {
    cell: Py<Cell>,
}

#[pymethods]
impl Squares {
    fn __len__(&self) -> usize {
        self.cell.get().inner.len()
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Square> {
        let cell = &self.cell.get().inner;
        let resolved = if index < 0 {
            index.checked_add_unsigned(cell.len())
        } else {
            Some(index)
        };
        let found = resolved
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| Some((cell.squares().get(index)?, cell.tiles_of(index)?)));
        let (at, tiles) =
            found.ok_or_else(|| PyIndexError::new_err("square index out of range"))?;
        Ok(Square {
            block: at.block,
            layer: at.layer,
            x: at.x,
            y: at.y,
            room: at.room,
            tiles: PyTuple::new(py, tiles)?.unbind(),
        })
    }
}

/// One texture (sprite) on a texture pack page.
///
/// Textures are stored with their transparent margins trimmed off. `x`, `y`, `w` and `h` give the
/// trimmed rectangle inside the page image. `ox` and `oy` give where that rectangle sits inside
/// the original, untrimmed sprite, which is `ow` by `oh` pixels. All values are in pixels.
#[pyclass(frozen, get_all, module = "pzmap")]
pub struct Texture {
    /// The texture's name.
    pub name: String,
    /// X of the trimmed rectangle in the page image.
    pub x: i32,
    /// Y of the trimmed rectangle in the page image.
    pub y: i32,
    /// Width of the trimmed rectangle.
    pub w: i32,
    /// Height of the trimmed rectangle.
    pub h: i32,
    /// X offset of the trimmed rectangle inside the original sprite.
    pub ox: i32,
    /// Y offset of the trimmed rectangle inside the original sprite.
    pub oy: i32,
    /// Width of the original, untrimmed sprite.
    pub ow: i32,
    /// Height of the original, untrimmed sprite.
    pub oh: i32,
}

#[pymethods]
impl Texture {
    fn __repr__(&self) -> String {
        format!(
            "Texture(name={:?}, x={}, y={}, w={}, h={})",
            self.name, self.x, self.y, self.w, self.h
        )
    }
}

/// One page of a texture pack: a PNG image holding many textures.
///
/// A page shares its pack's data instead of copying it. It stays valid after the `TexturePack` is
/// gone.
#[pyclass(frozen, module = "pzmap")]
pub struct Page {
    page: Yoke<NativePage<'static>, Arc<OwnedTexturePack>>,
    textures: PyOnceLock<Py<PyTuple>>,
}

#[pymethods]
impl Page {
    /// The page's name.
    #[getter]
    fn name(&self) -> &str {
        self.page.get().name
    }

    /// Whether the pack says the page's PNG has an alpha channel.
    #[getter]
    fn has_alpha(&self) -> bool {
        self.page.get().has_alpha
    }

    /// The page image, as the bytes of a PNG file. This library does not decode it.
    ///
    /// Each read copies the image into a new `bytes` object, so keep the result if you need it
    /// more than once.
    #[getter]
    fn png<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.page.get().png)
    }

    /// The textures on this page, as a tuple of `Texture`.
    #[getter]
    fn textures<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        cached(py, &self.textures, || {
            self.page
                .get()
                .textures
                .iter()
                .map(|t| Texture {
                    name: t.name.to_owned(),
                    x: t.x,
                    y: t.y,
                    w: t.width,
                    h: t.height,
                    ox: t.offset_x,
                    oy: t.offset_y,
                    ow: t.original_width,
                    oh: t.original_height,
                })
                .collect()
        })
    }

    fn __repr__(&self) -> String {
        let page = self.page.get();
        format!(
            "Page(name={:?}, textures={}, png={} bytes)",
            page.name,
            page.textures.len(),
            page.png.len()
        )
    }
}

/// A parsed `.pack` file: a texture atlas made of PNG pages and the named textures on each.
///
/// The game draws map tiles from these textures. `len(pack)` is the number of pages. The `pages`
/// tuple, and each page's `textures` tuple, are built on first access and then cached.
///
/// A `TexturePack` can be pickled. It pickles as the original file bytes, which are parsed again
/// when it is unpickled. Packs can be large, so sending a worker the file path may be cheaper.
/// Pages cannot be pickled on their own.
#[pyclass(frozen, module = "pzmap")]
pub struct TexturePack {
    inner: Arc<OwnedTexturePack>,
    pages: PyOnceLock<Py<PyTuple>>,
}

impl From<OwnedTexturePack> for TexturePack {
    fn from(inner: OwnedTexturePack) -> TexturePack {
        TexturePack {
            inner: Arc::new(inner),
            pages: PyOnceLock::new(),
        }
    }
}

#[pymethods]
impl TexturePack {
    /// Parse a `.pack` from the file's contents.
    ///
    /// `data` is copied. When the file is on disk, `from_path` avoids that copy.
    ///
    /// Raises `ParseError` if `data` is not a valid `.pack`.
    #[staticmethod]
    fn parse(py: Python<'_>, data: Vec<u8>) -> Result<TexturePack, Error> {
        py.detach(|| Ok(OwnedTexturePack::parse(data)?.into()))
    }

    /// Read and parse a `.pack` file.
    ///
    /// `path` is a `str` or `os.PathLike`.
    ///
    /// Raises `OSError` if the file cannot be read, such as `FileNotFoundError` with `filename`
    /// set. Raises `ParseError` if its contents are not a valid `.pack`.
    #[staticmethod]
    fn from_path(py: Python<'_>, path: PathBuf) -> Result<TexturePack, Error> {
        py.detach(|| Ok(OwnedTexturePack::parse(read(path)?)?.into()))
    }

    /// Every page in the pack, in file order, as a tuple of `Page`.
    #[getter]
    fn pages<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        cached(py, &self.pages, || {
            let count = self.inner.get().pages.len();
            (0..count)
                .filter_map(|index| {
                    let page = Yoke::try_attach_to_cart(Arc::clone(&self.inner), |pack| {
                        pack.get().pages.get(index).cloned().ok_or(())
                    });
                    page.ok().map(|page| Page {
                        page,
                        textures: PyOnceLock::new(),
                    })
                })
                .collect()
        })
    }

    /// The total number of textures across all pages.
    #[getter]
    fn texture_count(&self) -> usize {
        self.inner.get().texture_count()
    }

    fn __len__(&self) -> usize {
        self.inner.get().pages.len()
    }

    /// Support pickling. The pack pickles as its file bytes and is parsed again when unpickled.
    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> Reduce<'py, (Bound<'py, PyBytes>,)> {
        let parse = slf.get_type().getattr("parse")?;
        let bytes = PyBytes::new(slf.py(), slf.get().inner.bytes());
        Ok((parse, (bytes,)))
    }

    fn __repr__(&self) -> String {
        let pack = self.inner.get();
        format!(
            "TexturePack(pages={}, textures={})",
            pack.pages.len(),
            pack.texture_count()
        )
    }
}

/// The `pzmap._pzmap` native module. The `pzmap` package re-exports everything in it.
#[pymodule]
fn _pzmap(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add(
        "__doc__",
        "Parsers for Project Zomboid's on-disk map formats.",
    )?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Build>()?;
    m.add_class::<LotHeader>()?;
    m.add_class::<Room>()?;
    m.add_class::<Rectangle>()?;
    m.add_class::<RoomObject>()?;
    m.add_class::<Building>()?;
    m.add_class::<Cell>()?;
    m.add_class::<Squares>()?;
    m.add_class::<Square>()?;
    m.add_class::<TexturePack>()?;
    m.add_class::<Page>()?;
    m.add_class::<Texture>()?;
    Ok(())
}
