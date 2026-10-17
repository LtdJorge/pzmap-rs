//! Room contents: how many containers of each kind are in each room.
//!
//! The lotheader's room rectangles say which room each square is in. The game's tile
//! definitions (`newtiledefinitions.tiles`) say which sprites are containers. Combining the two
//! while the lotpacks are parsed gives each room a list such as `Fridge ×1, Counter ×4`,
//! without a second pass over the map.
//!
//! The pieces:
//!
//! - [`ItemIndex`] says which tile ids are containers. It is built once per build.
//! - [`RoomLookup`] says which room each square of one cell is in.
//! - [`CellContents`] counts one cell's containers while its lotpack is parsed.
//! - [`items_for_rooms`] turns those counts into the ranges that
//!   [`Annotations`](crate::runtime::vector::Annotations) stores.

use crate::{
    error::{
        BuildError,
        Result,
    },
    runtime::vector::{
        ItemKind,
        Rectangle,
        Room,
        RoomItem,
    },
    string_table::{
        StringTableReader,
        TileId,
        TileNames,
    },
};
use ahash::AHashMap;
use libpzmap::sidecar::tiledefs::TileDefs;
use std::ops::Range;

/// The value in `ItemIndex::by_tile` for a tile that is not a container.
const NO_ITEM: u32 = u32::MAX;

/// Which tiles are containers, and what to call them.
///
/// It is indexed by [`TileId`], so checking a tile is one array read.
#[derive(Clone, Debug, Default)]
pub struct ItemIndex {
    kinds: Vec<ItemKind>,
    by_tile: Box<[u32]>,
}

impl ItemIndex {
    /// Looks up every tile name in the tile definitions and records which are containers.
    ///
    /// A tile is a container when its definition has a `container` property. Its label comes
    /// from [`label_for`]. Tiles with the same container type and label share one kind.
    pub fn build(tiles: &StringTableReader<TileNames>, defs: &TileDefs<'_>) -> Self {
        let mut kinds: Vec<ItemKind> = Vec::new();
        let mut kind_ids: AHashMap<(String, String), u32> = AHashMap::new();
        let mut by_tile = vec![NO_ITEM; tiles.len()].into_boxed_slice();

        for (id, name) in tiles.iter() {
            let Some(props) = defs.find(name) else {
                continue;
            };
            let Some(container) = props
                .iter()
                .find(|(k, _)| *k == "container")
                .map(|(_, v)| *v)
            else {
                continue;
            };
            let custom = props
                .iter()
                .find(|(k, _)| *k == "CustomName")
                .map(|(_, v)| *v);
            let label = label_for(container, custom);

            let key = (container.to_owned(), label);
            let kind = *kind_ids
                .entry(key)
                .or_insert_with_key(|(container, label)| {
                    kinds.push(ItemKind {
                        container: container.as_str().into(),
                        label: label.as_str().into(),
                    });
                    u32::try_from(kinds.len() - 1).unwrap_or(NO_ITEM)
                });

            if let Some(slot) = by_tile.get_mut(id.index()) {
                *slot = kind;
            }
        }

        ItemIndex { kinds, by_tile }
    }

    /// Returns the kind of a tile, as an index into [`Self::kinds`], or `None` if it is not a
    /// container.
    #[inline]
    pub fn kind_of(&self, tile: TileId) -> Option<u32> {
        self.by_tile
            .get(tile.index())
            .copied()
            .filter(|&kind| kind != NO_ITEM)
    }

    /// Returns every container kind found, indexed by what [`ItemIndex::kind_of`] returns.
    pub fn kinds(&self) -> &[ItemKind] {
        &self.kinds
    }

    /// Returns the number of tile names that are containers.
    pub fn item_tiles(&self) -> usize {
        self.by_tile.iter().filter(|&&k| k != NO_ITEM).count()
    }
}

/// Returns the display label for a container tile.
///
/// This is the definition's `CustomName` when it has one, which is the container's in-game
/// name. Some definitions have the placeholder text `CustomName`, which counts as no name.
/// Without a name, the label is the container type with a capital first letter: `fridge`
/// becomes `Fridge`.
pub fn label_for(container: &str, custom_name: Option<&str>) -> String {
    match custom_name.map(str::trim) {
        Some(name) if !name.is_empty() && name != "CustomName" => name.to_owned(),
        _ => {
            let mut chars = container.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// The value in a `RoomLookup` grid for a square that is in no room.
const NO_ROOM: u32 = u32::MAX;

/// Which room each square of one cell is in, worked out from the lotheader's room rectangles.
///
/// The lotpack also stores a room id for each square, but it can't be used for this. In Build
/// 42 it is set on only a few squares per cell, and it doesn't match the lotheader's room
/// numbers. So the lookup fills one grid per layer from the room rectangles instead.
#[derive(Clone, Debug)]
pub struct RoomLookup {
    cell_squares: usize,
    layers: AHashMap<i32, Box<[u32]>>,
}

impl RoomLookup {
    /// Fills the grids for the cell at (`cell_x`, `cell_y`) from its rooms' rectangles.
    ///
    /// `rooms` and `rectangles` are the cell's own lists, and the rectangles are in world square
    /// coordinates. Parts of rectangles outside the cell are ignored. Where rooms overlap, the
    /// later room wins.
    pub fn new(
        cell_x: i32,
        cell_y: i32,
        cell_squares: usize,
        rooms: &[Room],
        rectangles: &[Rectangle],
    ) -> Self {
        let size = i32::try_from(cell_squares).unwrap_or(i32::MAX);
        let origin_x = cell_x.saturating_mul(size);
        let origin_y = cell_y.saturating_mul(size);
        let mut layers: AHashMap<i32, Box<[u32]>> = AHashMap::new();

        for (index, room) in rooms.iter().enumerate() {
            let Ok(index) = u32::try_from(index) else {
                break;
            };
            let rects = rectangles
                .get(room.rectangles.start as usize..room.rectangles.end as usize)
                .unwrap_or_default();

            for rect in rects {
                // Clip to the cell. The sizes come from the file, so the sums saturate instead
                // of overflowing.
                let x0 = rect.x.saturating_sub(origin_x).max(0);
                let y0 = rect.y.saturating_sub(origin_y).max(0);
                let x1 = rect
                    .x
                    .saturating_add(rect.width)
                    .saturating_sub(origin_x)
                    .min(size);
                let y1 = rect
                    .y
                    .saturating_add(rect.height)
                    .saturating_sub(origin_y)
                    .min(size);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }

                let grid = layers.entry(room.layer).or_insert_with(|| {
                    vec![NO_ROOM; cell_squares.saturating_mul(cell_squares)].into_boxed_slice()
                });
                for x in x0..x1 {
                    for y in y0..y1 {
                        // Clipped to `0..size`, so both conversions succeed.
                        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
                            continue;
                        };
                        if let Some(slot) = grid.get_mut(x * cell_squares + y) {
                            *slot = index;
                        }
                    }
                }
            }
        }

        RoomLookup {
            cell_squares,
            layers,
        }
    }

    /// Returns the index of the room a square is in, within the cell's room list, or `None`
    /// if the square is in no room.
    #[inline]
    pub fn room_at(&self, layer: i32, sub_x: usize, sub_y: usize) -> Option<u32> {
        let grid = self.layers.get(&layer)?;
        if sub_x >= self.cell_squares || sub_y >= self.cell_squares {
            return None;
        }
        grid.get(sub_x * self.cell_squares + sub_y)
            .copied()
            .filter(|&room| room != NO_ROOM)
    }

    /// Returns true if no square of the cell is in a room.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// A cell's container counts as `((room, kind), count)`, sorted by room, then kind.
pub type SortedContents = Vec<((u32, u32), u32)>;

/// Container counts for one cell, by room index within the cell and container kind.
#[derive(Clone, Debug, Default)]
pub struct CellContents {
    counts: AHashMap<(u32, u32), u32>,
}

impl CellContents {
    /// Counts the container tiles on one square in `room`.
    #[inline]
    pub fn record(&mut self, room: u32, tiles: impl Iterator<Item = TileId>, items: &ItemIndex) {
        for tile in tiles {
            if let Some(kind) = items.kind_of(tile) {
                let slot = self.counts.entry((room, kind)).or_insert(0);
                *slot = slot.saturating_add(1);
            }
        }
    }

    /// Returns the counts as `((room, kind), count)`, sorted by room, then kind.
    pub fn into_sorted(self) -> SortedContents {
        let mut entries: Vec<_> = self.counts.into_iter().collect();
        entries.sort_unstable();
        entries
    }
}

/// Appends each room's container counts to `items` and returns one range per room.
///
/// `sorted` is the result of [`CellContents::into_sorted`] for one cell. Rooms with no
/// containers get an empty range. Counts for a room index at or past `room_count` are dropped,
/// because the lotheader doesn't list that room.
///
/// # Errors
///
/// Returns [`BuildError::TooManyAnnotations`] if `items` grows past what a `u32` can index.
pub fn items_for_rooms(
    sorted: &[((u32, u32), u32)],
    room_count: usize,
    items: &mut Vec<RoomItem>,
) -> Result<Vec<Range<u32>>> {
    let span = |len: usize| {
        u32::try_from(len).map_err(|_| BuildError::TooManyAnnotations {
            what: "items",
            count: len,
        })
    };

    let mut ranges = Vec::with_capacity(room_count);
    let mut cursor = sorted.iter().peekable();

    for room in 0..room_count {
        let room = u32::try_from(room).unwrap_or(u32::MAX);
        let start = span(items.len())?;
        while let Some(((r, kind), count)) = cursor.peek().copied() {
            if *r != room {
                break;
            }
            items.push(RoomItem {
                kind: *kind,
                count: *count,
            });
            cursor.next();
        }
        ranges.push(start..span(items.len())?);
    }

    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string_table::StringTableBuilder;
    use libpzmap::sidecar::tiledefs::TileDefs;

    /// A `.tiles` file with one sheet whose tiles have the given properties.
    fn defs_file(sheet: &str, tiles: &[&[(&str, &str)]]) -> Vec<u8> {
        let mut out = b"tdef".to_vec();
        out.extend_from_slice(&1_u32.to_le_bytes());
        out.extend_from_slice(&1_u32.to_le_bytes());
        out.extend_from_slice(sheet.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(format!("{sheet}.png").as_bytes());
        out.push(b'\n');
        for n in [8_u32, 8, 0, u32::try_from(tiles.len()).unwrap()] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for props in tiles {
            out.extend_from_slice(&u32::try_from(props.len()).unwrap().to_le_bytes());
            for (k, v) in *props {
                out.extend_from_slice(k.as_bytes());
                out.push(b'\n');
                out.extend_from_slice(v.as_bytes());
                out.push(b'\n');
            }
        }
        out
    }

    fn index() -> (ItemIndex, Vec<TileId>) {
        let bytes = defs_file(
            "kitchen_01",
            &[
                &[("container", "fridge"), ("CustomName", "Fridge")],
                &[("container", "counter"), ("CustomName", "Counter")],
                &[("CustomName", "Chair")],
                &[("container", "shelves")],
                &[("container", "counter"), ("CustomName", "Counter")],
                &[("container", "plankstash"), ("CustomName", "CustomName")],
            ],
        );
        let defs = TileDefs::parse(&bytes).unwrap();

        let names = StringTableBuilder::<TileNames>::new();
        let ids: Vec<TileId> = [
            "kitchen_01_0",
            "kitchen_01_1",
            "kitchen_01_2",
            "kitchen_01_3",
            "kitchen_01_4",
            "kitchen_01_5",
            "floors_01_0",
            "kitchen_01_99",
        ]
        .iter()
        .map(|name| names.intern(name).unwrap())
        .collect();

        (ItemIndex::build(&names.into_reader(), &defs), ids)
    }

    #[test]
    fn containers_are_classified_and_labelled() {
        let (index, ids) = index();

        assert_eq!(index.item_tiles(), 5);
        let label = |tile: TileId| {
            index
                .kind_of(tile)
                .and_then(|k| index.kinds().get(k as usize))
                .map(|k| (k.container.as_ref(), k.label.as_ref()))
        };

        assert_eq!(label(ids[0]), Some(("fridge", "Fridge")));
        assert_eq!(label(ids[1]), Some(("counter", "Counter")));
        assert_eq!(label(ids[2]), None, "a chair is furniture, not a container");
        assert_eq!(label(ids[3]), Some(("shelves", "Shelves")));
        // Two sprites with the same container and label share one kind.
        assert_eq!(index.kind_of(ids[1]), index.kind_of(ids[4]));
        assert_eq!(label(ids[5]), Some(("plankstash", "Plankstash")));
        assert_eq!(index.kind_of(ids[6]), None, "unknown sheet");
        assert_eq!(index.kind_of(ids[7]), None, "index past the sheet");
        assert_eq!(index.kinds().len(), 4);
    }

    #[test]
    fn labels_prefer_custom_names() {
        assert_eq!(label_for("fridge", Some("Fridge")), "Fridge");
        assert_eq!(label_for("fridge", None), "Fridge");
        assert_eq!(label_for("fridge", Some("")), "Fridge");
        assert_eq!(label_for("fridge", Some("  ")), "Fridge");
        assert_eq!(label_for("plankstash", Some("CustomName")), "Plankstash");
        assert_eq!(label_for("stove", Some("Oven")), "Oven");
        assert_eq!(label_for("", None), "");
    }

    #[test]
    fn counts_aggregate_per_room_and_kind() {
        let (index, ids) = index();
        let mut cell = CellContents::default();

        // Room 0: a fridge and two counters over two squares. Room 2: shelves.
        cell.record(0, [ids[0], ids[1], ids[2]].into_iter(), &index);
        cell.record(0, [ids[4]].into_iter(), &index);
        cell.record(2, [ids[3], ids[6]].into_iter(), &index);
        // Room 7 does not exist in the header.
        cell.record(7, [ids[0]].into_iter(), &index);

        let sorted = cell.into_sorted();
        let mut items = Vec::new();
        let ranges = items_for_rooms(&sorted, 3, &mut items).unwrap();

        assert_eq!(ranges.len(), 3);
        let of = |r: &Range<u32>| &items[r.start as usize..r.end as usize];

        let room0: Vec<_> = of(&ranges[0])
            .iter()
            .map(|i| (index.kinds()[i.kind as usize].label.as_ref(), i.count))
            .collect();
        assert_eq!(room0, vec![("Fridge", 1), ("Counter", 2)]);
        assert!(of(&ranges[1]).is_empty());
        assert_eq!(of(&ranges[2]).len(), 1);
        assert_eq!(items.len(), 3, "the phantom room's fridge is dropped");
    }

    #[test]
    fn squares_map_to_rooms_by_rectangle() {
        use crate::string_table::{
            RoomNames,
            StringTableBuilder,
        };
        let names = StringTableBuilder::<RoomNames>::new();
        let name = names.intern("kitchen").unwrap();
        let room = |layer: i32, rectangles: Range<u32>| Room {
            name,
            layer,
            rectangles,
            objects: 0..0,
            items: 0..0,
        };
        // Cell (2, 3) of 8 squares spans world 16..24 x 24..32.
        let rooms = [room(0, 0..2), room(1, 2..3), room(0, 3..4)];
        let rectangles = [
            // Two rectangles making an L on layer 0.
            Rectangle {
                x: 17,
                y: 25,
                width: 2,
                height: 1,
            },
            Rectangle {
                x: 17,
                y: 26,
                width: 1,
                height: 2,
            },
            // Upstairs, hanging over the cell's right edge.
            Rectangle {
                x: 22,
                y: 24,
                width: 5,
                height: 2,
            },
            // A later room overlapping the first one wins.
            Rectangle {
                x: 18,
                y: 25,
                width: 1,
                height: 1,
            },
        ];
        let lookup = RoomLookup::new(2, 3, 8, &rooms, &rectangles);
        assert!(!lookup.is_empty());

        assert_eq!(lookup.room_at(0, 1, 1), Some(0));
        assert_eq!(lookup.room_at(0, 1, 3), Some(0));
        assert_eq!(
            lookup.room_at(0, 2, 1),
            Some(2),
            "overlap goes to the later room"
        );
        assert_eq!(lookup.room_at(0, 0, 0), None);
        assert_eq!(lookup.room_at(0, 2, 2), None);
        assert_eq!(lookup.room_at(1, 6, 0), Some(1));
        assert_eq!(lookup.room_at(1, 7, 1), Some(1));
        assert_eq!(
            lookup.room_at(1, 1, 1),
            None,
            "the ground-floor room is not upstairs"
        );
        assert_eq!(lookup.room_at(2, 1, 1), None);
        assert_eq!(lookup.room_at(0, 8, 0), None, "outside the cell");

        assert!(RoomLookup::new(0, 0, 8, &[], &[]).is_empty());
    }

    #[test]
    fn empty_cells_give_empty_ranges() {
        let mut items = Vec::new();
        let ranges = items_for_rooms(&[], 2, &mut items).unwrap();
        assert_eq!(ranges, vec![0..0, 0..0]);
        assert!(items.is_empty());
    }
}
