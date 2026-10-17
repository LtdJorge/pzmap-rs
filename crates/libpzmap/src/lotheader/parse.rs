use crate::{
    build::{
        VersionMetadata,
        repeat,
        sealed::{
            LayerEncoding,
            Sealed,
        },
        widen,
    },
    error::{
        Format,
        ParseError,
        ParseErrorKind,
        Result,
    },
    input::Input,
    lotheader::{
        Building,
        LotHeader,
        Object,
        Rectangle,
        Room,
    },
};
use std::marker::PhantomData;
use tracing::debug;

/// The smallest size each repeated element can have in the file.
///
/// A count that needs more bytes than the file has left is rejected before allocating.
/// These are lower bounds, not exact sizes.
mod min_bytes {
    /// An empty name is still one newline byte.
    pub const STRING: usize = 1;
    /// Name, layer, rectangle count, and object count.
    pub const ROOM: usize = 1 + 4 + 4 + 4;
    /// X, Y, width, and height.
    pub const RECTANGLE: usize = 16;
    /// Type, X and Y.
    pub const OBJECT: usize = 12;
    /// A room count of zero, with no rooms.
    pub const BUILDING: usize = 4;
    /// One `u32` room index.
    pub const ROOM_INDEX: usize = 4;
}

impl<'inp, V: VersionMetadata> LotHeader<'inp, V> {
    /// Parses the `.lotheader` of cell `(x, y)`, written by build `V`.
    ///
    /// `x` and `y` must be the numbers in the file name. The file stores room and object
    /// positions relative to the cell, and the parser uses `x` and `y` to turn them into world
    /// coordinates. With the wrong cell position, every position in the result is wrong.
    ///
    /// ```no_run
    /// use libpzmap::{B42, lotheader::LotHeader};
    ///
    /// let bytes = std::fs::read("0_18.lotheader")?;
    /// let header = LotHeader::<B42>::parse(0, 18, &bytes)?;
    /// println!("{} distinct tile names", header.strings.len());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the file is truncated, if a count is larger than the remaining data,
    /// if a name is not UTF-8, or if the version or block size does not match that of `V`.
    #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
    pub fn parse(x: i32, y: i32, bytes: &'inp [u8]) -> Result<Self> {
        debug!("Parsing header for coords ({x},{y})");

        let mut input = Input::new(bytes, Format::LotHeader);

        let at = input.offset();
        let version = input.optional_magic_version("LOTH")?;
        if version != const { <V as VersionMetadata>::VERSION_NUMBER } {
            return Err(ParseError {
                format: Format::LotHeader,
                at,
                kind: ParseErrorKind::Version {
                    what: V::NAME,
                    found: version,
                },
            });
        }

        let num_strings = input.i32_le()?;
        let num_strings =
            input.checked_count("string count", num_strings as i64, min_bytes::STRING)?;
        let strings = repeat(&mut input, num_strings, |inp| inp.line("tile name"))?;

        if const { <V as Sealed>::NULL_AFTER_STRING_TABLE } {
            input.skip(1)?;
        }

        // A block size that does not match `V` means the file is from a different build
        expect_block_size::<V>(&mut input, "block width")?;
        expect_block_size::<V>(&mut input, "block height")?;

        let (min_layer, max_layer) = parse_layers::<V>(&mut input)?;

        let origin = world_origin::<V>(x, y, &input)?;
        let room_count = input.i32_le()?;
        let room_count = input.checked_count("room count", room_count as i64, min_bytes::ROOM)?;
        let rooms = repeat(&mut input, room_count, |inp| parse_room(inp, origin))?;

        let buildings_count = input.i32_le()?;
        let buildings_count = input.checked_count(
            "building count",
            buildings_count as i64,
            min_bytes::BUILDING,
        )?;
        let buildings = repeat(&mut input, buildings_count, parse_building)?;

        let side = const { <V as VersionMetadata>::CELL_SIZE_IN_BLOCKS };
        let zpop = input.take(side * side)?.to_vec();

        let trailing_bytes = input.remaining_len();
        if trailing_bytes != 0 {
            tracing::warn!(
                trailing_bytes,
                cell_x = x,
                cell_y = y,
                "lotheader was not fully consumed"
            );
        }

        Ok(LotHeader {
            x,
            y,
            version,
            strings: strings.into(),
            min_layer,
            max_layer,
            rooms: rooms.into_boxed_slice(),
            buildings: buildings.into_boxed_slice(),
            zpop,
            trailing_bytes,
            marker: PhantomData,
        })
    }
}

/// The world position of the first square of cell `(x, y)`
fn world_origin<V: VersionMetadata>(x: i32, y: i32, input: &Input<'_>) -> Result<(i32, i32)> {
    let stride = const { <V as VersionMetadata>::CELL_SIZE_IN_SQUARES };

    let origin = x
        .checked_mul(stride)
        .zip(y.checked_mul(stride))
        .ok_or(ParseError {
            format: Format::LotHeader,
            at: input.offset(),
            kind: ParseErrorKind::OutOfRange {
                field: "cell coordinate",
                value: x as i64,
                min: (i32::MIN / stride) as i64,
                max: (i32::MAX / stride) as i64,
            },
        })?;

    Ok(origin)
}

/// Reads the cell's layer range and returns it as `(lowest, one past the highest)`.
///
/// Build 41 stores only the maximum, and its minimum is always zero. Build 42 stores both. The
/// range is clamped to the layers the build supports.
#[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
fn parse_layers<V: VersionMetadata>(input: &mut Input<'_>) -> Result<(i32, i32)> {
    let (min_layer, max_layer) = match const { <V as Sealed>::LAYER_ENCODING } {
        LayerEncoding::MaxOnly => {
            let max_layer = input.i32_le()?;
            (V::MIN_LAYER, max_layer.min(V::MAX_LAYER))
        }
        LayerEncoding::MinAndMax => {
            let min_layer = input.i32_le()?;
            let max_layer = input.i32_le()?;

            let max_layer = max_layer.checked_add(1).ok_or(ParseError {
                format: Format::LotHeader,
                at: input.offset(),
                kind: ParseErrorKind::OutOfRange {
                    field: "max layer",
                    value: max_layer as i64,
                    min: V::MIN_LAYER as i64,
                    max: V::MAX_LAYER as i64,
                },
            })?;

            (min_layer.max(V::MIN_LAYER), max_layer.min(V::MAX_LAYER))
        }
    };

    debug!(min_layer, max_layer);

    Ok((min_layer, max_layer))
}

/// Reads one room and adds `origin` to its positions to make them world coordinates
fn parse_room<'inp>(input: &mut Input<'inp>, origin: (i32, i32)) -> Result<Room<'inp>> {
    let name = input.line("room name")?;
    let layer = input.i32_le()?;

    let rectangle_count = input.i32_le()?;
    let rectangle_count = input.checked_count(
        "rectangle count",
        rectangle_count as i64,
        min_bytes::RECTANGLE,
    )?;
    let rectangles = repeat(input, rectangle_count, |inp| parse_rectangle(inp, origin))?;

    let object_count = input.i32_le()?;
    let object_count =
        input.checked_count("object count", object_count as i64, min_bytes::OBJECT)?;
    let objects = repeat(input, object_count, |inp| parse_object(inp, origin))?;

    Ok(Room {
        name,
        layer,
        rectangles,
        objects,
    })
}

fn parse_rectangle(input: &mut Input<'_>, (wx, wy): (i32, i32)) -> Result<Rectangle> {
    let x = input.i32_le()?;
    let y = input.i32_le()?;
    let width = input.i32_le()?;
    let height = input.i32_le()?;

    Ok(Rectangle {
        x: add_world(input, "rectangle x", x, wx)?,
        y: add_world(input, "rectangle y", y, wy)?,
        width,
        height,
    })
}

fn parse_object(input: &mut Input<'_>, (wx, wy): (i32, i32)) -> Result<Object> {
    let kind = input.i32_le()?;
    let x = input.i32_le()?;
    let y = input.i32_le()?;

    Ok(Object {
        kind,
        x: add_world(input, "object x", x, wx)?,
        y: add_world(input, "object y", y, wy)?,
    })
}

fn parse_building(input: &mut Input<'_>) -> Result<Building> {
    let room_count = input.i32_le()?;
    let room_count = input.checked_count(
        "building room count",
        room_count as i64,
        min_bytes::ROOM_INDEX,
    )?;
    let room_indices = repeat(input, room_count, |inp| inp.u32_le())?;

    Ok(Building { room_indices })
}

/// Adds the cell's world position to a position within the cell.
#[inline]
fn add_world(input: &Input<'_>, field: &'static str, local: i32, origin: i32) -> Result<i32> {
    local.checked_add(origin).ok_or(ParseError {
        format: Format::LotHeader,
        at: input.offset(),
        kind: ParseErrorKind::OutOfRange {
            field,
            value: local as i64,
            min: i32::MIN as i64,
            max: i32::MAX as i64,
        },
    })
}

/// Reads one of the two block-size fields and checks that it matches build `V`
fn expect_block_size<V: VersionMetadata>(input: &mut Input<'_>, field: &'static str) -> Result<()> {
    let expected = const { widen(<V as VersionMetadata>::BLOCK_SIZE_IN_SQUARES) };
    let found = input.i32_le()?;

    input.require_range(field, i64::from(found), expected, expected)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        build::{
            B41,
            B42,
        },
        error::{
            Format,
            ParseError,
            ParseErrorKind,
        },
        fixtures,
        lotheader::LotHeader,
    };

    #[test]
    #[ignore = "requires a Build 41 install; set PZ_B41_INSTALL_DIR"]
    fn lotheader_b41() {
        let path = fixtures::b41_lotheader("Muldraugh, KY", 0, 18)
            .expect("set PZ_B41_INSTALL_DIR to a Build 41 install to run this test");

        let bytes = fixtures::read(&path);
        let header = LotHeader::<B41>::parse(0, 18, &bytes).expect("B41 lotheader should parse");

        assert!(!header.strings.is_empty());
    }

    #[test]
    fn a_version_other_than_the_builds_is_rejected() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"LOTH");
        bytes.extend_from_slice(&1_u32.to_le_bytes());

        let err = LotHeader::<B41>::parse(0, 0, &bytes).expect_err("version 1 is not B41");

        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::LotHeader,
                    at: 0,
                    kind: ParseErrorKind::Version { found: 1, .. },
                }
            ),
            "unexpected error: {err}"
        );
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn lotheader_b42() {
        let bytes = fixtures::read(&fixtures::lotheader("Muldraugh, KY", 0, 18));
        let header = LotHeader::<B42>::parse(0, 18, &bytes).expect("B42 lotheader should parse");

        assert!(!header.strings.is_empty());
        assert_eq!(header.trailing_bytes, 0);
    }
}

#[cfg(test)]
mod exhaustive_tests {
    use crate::{
        build::B42,
        fixtures,
        lotheader::LotHeader,
    };
    use std::fs;

    /// Every lotheader shipped with Build 42 parses with no bytes left over.
    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn every_b42_lotheader_is_fully_consumed() -> anyhow::Result<()> {
        let dir = fixtures::muldraugh();
        let mut checked = 0_usize;

        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lotheader") {
                continue;
            }

            let bytes = fs::read(&path)?;
            let header = LotHeader::<B42>::parse(0, 0, &bytes)
                .unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));

            assert_eq!(
                header.trailing_bytes,
                0,
                "{} left {} bytes unconsumed",
                path.display(),
                header.trailing_bytes
            );
            checked += 1;
        }

        assert!(checked > 0, "no lotheaders found in {}", dir.display());
        println!("checked {checked} lotheaders");
        Ok(())
    }
}
