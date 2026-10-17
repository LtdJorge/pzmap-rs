//! Map annotations: rooms, buildings, room contents and zombie spawn density.
//!
//! These come from the lotheaders, plus the lotpacks when room contents are counted. They are
//! meant for vector tiles and overlays drawn on top of the bitmap map.
//!
//! [`Annotations`] stores each kind of record in one flat array for the whole map. Records refer
//! to each other by `Range<u32>` index ranges into those arrays.

use crate::{
    runtime::size::HeapSize,
    string_table::{
        RoomNameId,
        RoomNames,
        StringTable,
    },
};
use std::ops::Range;

/// One rectangle of a room's floor area, in world square coordinates.
#[derive(Copy, Clone, Debug)]
pub struct Rectangle {
    /// The world x coordinate of the rectangle's first square.
    pub x: i32,
    /// The world y coordinate of the rectangle's first square.
    pub y: i32,
    /// Width, in squares.
    pub width: i32,
    /// Height, in squares.
    pub height: i32,
}

/// An object the lotheader lists in a room.
#[derive(Copy, Clone, Debug)]
pub struct Object {
    /// The game's type number for the object. This crate doesn't interpret it.
    pub object_type: i32,
    /// World x coordinate, in squares.
    pub x: i32,
    /// World y coordinate, in squares.
    pub y: i32,
}

/// A room. Its rectangles, objects and contents are ranges into the arrays in
/// [`Annotations`].
#[derive(Clone, Debug)]
pub struct Room {
    /// The room's name, such as `kitchen`. [`Annotations::name`] turns it into text.
    pub name: RoomNameId,
    /// The layer the room is on.
    pub layer: i32,
    /// The room's floor rectangles. Read them with [`Annotations::rectangles_of`].
    pub rectangles: Range<u32>,
    /// The room's objects. Read them with [`Annotations::objects_of`].
    pub objects: Range<u32>,
    /// The room's container counts. Read them with [`Annotations::items_of`]. Empty unless the
    /// build counted room contents.
    pub items: Range<u32>,
}

/// A kind of container, such as a fridge, with its in-game label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemKind {
    /// The tile definition's `container` value, such as `fridge` or `counter`.
    pub container: Box<str>,
    /// The tile definition's `CustomName`, or the container type with a capital first
    /// letter.
    pub label: Box<str>,
}

/// How many containers of one kind are in a room.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RoomItem {
    /// The container kind, as an index into [`Annotations::item_kinds`].
    pub kind: u32,
    /// How many container tiles of that kind the room has.
    pub count: u32,
}

/// One cell's part of the annotation arrays.
#[derive(Clone, Debug)]
pub struct CellAnnotations {
    /// The cell's x coordinate, in cells.
    pub cell_x: i32,
    /// The cell's y coordinate, in cells.
    pub cell_y: i32,
    /// The cell's rooms. Index [`Annotations::rooms`] with it.
    pub rooms: Range<u32>,
    /// The cell's buildings. Index [`Annotations::buildings`] with it.
    pub buildings: Range<u32>,
    /// The cell's zombie spawn density. Read it with [`Annotations::zpop_of`].
    pub zpop: Range<u32>,
}

/// The arrays an [`Annotations`] is built from.
///
/// The ranges index these arrays as follows:
///
/// - [`CellAnnotations::rooms`] and each entry of `buildings` index `rooms`.
/// - [`CellAnnotations::buildings`] indexes `buildings`.
/// - [`CellAnnotations::zpop`] indexes `zpop`, and covers `zpop_side * zpop_side` bytes.
/// - A [`Room`]'s `rectangles`, `objects` and `items` index the arrays with the same names.
/// - [`RoomItem::kind`] indexes `item_kinds`.
///
/// A range outside its array reads back as empty instead of panicking.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct AnnotationParts {
    /// One entry per cell.
    pub cells: Vec<CellAnnotations>,
    /// Every room in the map.
    pub rooms: Vec<Room>,
    /// Every room rectangle.
    pub rectangles: Vec<Rectangle>,
    /// Every room object.
    pub objects: Vec<Object>,
    /// Every building, as a range of rooms.
    pub buildings: Vec<Range<u32>>,
    /// Zombie spawn density, one byte per block, one cell after another.
    pub zpop: Vec<u8>,
    /// Blocks along one side of a cell: 32 in Build 42, 30 in Build 41.
    pub zpop_side: usize,
    /// Container counts, one room after another. Empty unless contents were counted.
    pub items: Vec<RoomItem>,
    /// Every container kind.
    pub item_kinds: Vec<ItemKind>,
}

/// A map's rooms, buildings, room contents and zombie spawn density.
///
/// [`build`](crate::builder::build) produces it when [`BuildTargets::vector`] or
/// [`BuildTargets::contents`] is set.
///
/// [`BuildTargets::vector`]: crate::runtime::BuildTargets::vector
/// [`BuildTargets::contents`]: crate::runtime::BuildTargets::contents
pub struct Annotations {
    cells: Box<[CellAnnotations]>,
    rooms: Box<[Room]>,
    rectangles: Box<[Rectangle]>,
    objects: Box<[Object]>,
    /// Each building is a range of indices into `rooms`.
    buildings: Box<[Range<u32>]>,
    /// Zombie spawn density, one byte per block, one cell after another.
    zpop: Box<[u8]>,
    /// Blocks along one side of a cell: 32 in Build 42, 30 in Build 41.
    zpop_side: usize,
    names: StringTable<RoomNames>,
    /// Container counts, one room after another. Empty unless contents were counted.
    items: Box<[RoomItem]>,
    item_kinds: Box<[ItemKind]>,
}

impl Annotations {
    /// Creates annotations from their arrays and the table that holds every [`Room::name`].
    ///
    /// See [`AnnotationParts`] for how the arrays refer to each other.
    pub fn new(parts: AnnotationParts, names: StringTable<RoomNames>) -> Self {
        Annotations {
            cells: parts.cells.into_boxed_slice(),
            rooms: parts.rooms.into_boxed_slice(),
            rectangles: parts.rectangles.into_boxed_slice(),
            objects: parts.objects.into_boxed_slice(),
            buildings: parts.buildings.into_boxed_slice(),
            zpop: parts.zpop.into_boxed_slice(),
            zpop_side: parts.zpop_side,
            names,
            items: parts.items.into_boxed_slice(),
            item_kinds: parts.item_kinds.into_boxed_slice(),
        }
    }

    /// Returns one entry per cell.
    pub fn cells(&self) -> &[CellAnnotations] {
        &self.cells
    }

    /// Returns every room in the map.
    pub fn rooms(&self) -> &[Room] {
        &self.rooms
    }

    /// Returns every building, as a range of indices into [`Self::rooms`].
    pub fn buildings(&self) -> &[Range<u32>] {
        &self.buildings
    }

    /// Returns a room's floor rectangles.
    ///
    /// A room from a different `Annotations` may give an empty slice. It doesn't panic.
    pub fn rectangles_of(&self, room: &Room) -> &[Rectangle] {
        slice_of(&self.rectangles, &room.rectangles)
    }

    /// Returns a room's objects.
    ///
    /// A room from a different `Annotations` may give an empty slice. It doesn't panic.
    pub fn objects_of(&self, room: &Room) -> &[Object] {
        slice_of(&self.objects, &room.objects)
    }

    /// Returns a building's rooms.
    ///
    /// A building from a different `Annotations` may give an empty slice. It doesn't panic.
    pub fn rooms_of(&self, building: &Range<u32>) -> &[Room] {
        slice_of(&self.rooms, building)
    }

    /// Returns a room's container counts, or an empty slice when contents weren't counted.
    pub fn items_of(&self, room: &Room) -> &[RoomItem] {
        slice_of(&self.items, &room.items)
    }

    /// Returns every container kind, indexed by [`RoomItem::kind`].
    pub fn item_kinds(&self) -> &[ItemKind] {
        &self.item_kinds
    }

    /// Returns the kind of a [`RoomItem`], or `None` if it is out of range for these
    /// annotations.
    pub fn item_kind(&self, item: &RoomItem) -> Option<&ItemKind> {
        self.item_kinds.get(item.kind as usize)
    }

    /// Returns the total number of containers in all rooms.
    pub fn item_total(&self) -> u64 {
        self.items.iter().map(|item| u64::from(item.count)).sum()
    }

    /// Returns one cell's zombie spawn density, one byte per block.
    ///
    /// The grid is [`Annotations::zpop_side`] blocks on each side. The value for block
    /// (`block_x`, `block_y`) is at index `block_x * zpop_side + block_y`.
    pub fn zpop_of(&self, cell: &CellAnnotations) -> &[u8] {
        slice_of(&self.zpop, &cell.zpop)
    }

    /// Returns the number of blocks along one side of a cell.
    pub fn zpop_side(&self) -> usize {
        self.zpop_side
    }

    /// Returns a room name, or `None` if `id` is not from these annotations.
    pub fn name(&self, id: RoomNameId) -> Option<&str> {
        self.names.resolve(id)
    }

    /// Returns the table of room names.
    pub fn names(&self) -> &StringTable<RoomNames> {
        &self.names
    }
}

/// Indexes `items` with `range`, giving an empty slice if the range is out of bounds.
#[inline]
fn slice_of<'a, T>(items: &'a [T], range: &Range<u32>) -> &'a [T] {
    items
        .get(range.start as usize..range.end as usize)
        .unwrap_or_default()
}

impl HeapSize for Annotations {
    fn heap_size(&self) -> usize {
        self.cells.len() * size_of::<CellAnnotations>()
            + self.rooms.len() * size_of::<Room>()
            + self.rectangles.len() * size_of::<Rectangle>()
            + self.objects.len() * size_of::<Object>()
            + self.buildings.len() * size_of::<Range<u32>>()
            + self.zpop.len()
            + self.items.len() * size_of::<RoomItem>()
            + self
                .item_kinds
                .iter()
                .map(|kind| size_of::<ItemKind>() + kind.container.len() + kind.label.len())
                .sum::<usize>()
    }
}
