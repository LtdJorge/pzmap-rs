//! A reader for tile definition files, such as `newtiledefinitions.tiles`.
//!
//! The game keeps `newtiledefinitions.tiles` in its `media` folder, next to other files in the
//! same format, such as `tiledefinitions_erosion.tiles`. A tile definitions file lists every
//! tile sheet the game knows. A tile sheet is one image cut into a grid of sprites. For each
//! tile of each sheet, the file gives its properties, such as `container = fridge`,
//! `IsoType = IsoDoor` or `solid`. The properties tell, for example, which sprites are
//! containers or furniture.
//!
//! The file is binary. All numbers are little-endian `u32`s and all strings end with a newline:
//!
//! ```text
//! magic "tdef", u32 version, u32 sheet count
//! per sheet: name "\n", image "\n", u32 columns, u32 rows, u32 sheet number, u32 tile count
//! per tile:  u32 property count, then per property: key "\n", value "\n"
//! ```
//!
//! Tiles are stored in sheet order, so tile `i` of sheet `roofs_01` is the sprite `roofs_01_i`.
//! A property with no value, such as `solid`, has an empty value line.
//!
//! [`TileDefs::parse`] rejects a file with the wrong magic bytes, a version other than 1, counts
//! larger than the file could hold, strings that are not UTF-8, or bytes left over at the end.

use crate::{
    error::{
        Format,
        ParseError,
        ParseErrorKind,
        Result,
    },
    input::Input,
};
use ahash::AHashMap;
use std::ops::Range;

/// The only file version this reader understands.
const SUPPORTED_VERSION: u32 = 1;

/// The smallest size in bytes of each repeated element.
///
/// A count that needs more bytes than the file has left is rejected before anything is
/// allocated for it.
mod min_bytes {
    /// Two newlines and four `u32`s.
    pub const SHEET: usize = 1 + 1 + 4 * 4;
    /// A property count with no properties.
    pub const TILE: usize = 4;
    /// Two newlines.
    pub const PROPERTY: usize = 2;
}

/// A parsed tile definitions file.
///
/// Its strings borrow from the input buffer.
#[derive(Clone, Debug)]
pub struct TileDefs<'a> {
    version: u32,
    sheets: Vec<Sheet<'a>>,
    /// The index in `properties` where each tile's properties start, plus a final entry equal
    /// to `properties.len()`. Tiles are numbered across all sheets in file order.
    offsets: Vec<u32>,
    properties: Vec<(&'a str, &'a str)>,
    by_name: AHashMap<&'a str, usize>,
}

/// One tile sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sheet<'a> {
    /// The sheet's name, such as `roofs_01`. Its sprite names start with it.
    pub name: &'a str,
    /// The image file the sheet is cut from, such as `roofs_01.png`.
    pub image: &'a str,
    /// The number of tiles in each row of the image.
    pub columns: u32,
    /// The number of rows of tiles in the image.
    pub rows: u32,
    /// The sheet's number in the game's global tile numbering.
    pub number: u32,
    /// The positions of this sheet's tiles among all the tiles in the file.
    tiles: Range<usize>,
}

impl Sheet<'_> {
    /// The number of tiles the sheet defines.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
}

impl<'a> TileDefs<'a> {
    /// The file format version. The parser accepts only version 1.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Every sheet, in file order.
    pub fn sheets(&self) -> &[Sheet<'a>] {
        &self.sheets
    }

    /// The sheet with this name.
    ///
    /// If the file has two sheets with the same name, this returns the later one.
    pub fn sheet(&self, name: &str) -> Option<&Sheet<'a>> {
        self.sheets.get(*self.by_name.get(name)?)
    }

    /// The total number of tiles in all sheets.
    pub fn tile_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// Every property of one tile, in file order.
    ///
    /// Returns `None` if the sheet does not exist or has no tile at `index`.
    pub fn properties(&self, sheet: &str, index: u32) -> Option<&[(&'a str, &'a str)]> {
        let sheet = self.sheet(sheet)?;
        let tile = sheet
            .tiles
            .start
            .checked_add(usize::try_from(index).ok()?)?;
        if tile >= sheet.tiles.end {
            return None;
        }
        self.tile_properties(tile)
    }

    /// The properties of a sprite named `<sheet>_<index>`, such as `fixtures_counters_01_12`.
    pub fn find(&self, sprite: &str) -> Option<&[(&'a str, &'a str)]> {
        let (sheet, index) = split_sprite(sprite)?;
        self.properties(sheet, index)
    }

    /// The value of one property of a sprite.
    ///
    /// A property without a value, such as `solid`, returns `Some("")`.
    pub fn property(&self, sprite: &str, key: &str) -> Option<&'a str> {
        self.find(sprite)?
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }

    /// Every tile as `(sheet, index within the sheet, properties)`, in file order.
    pub fn tiles(&self) -> impl Iterator<Item = (&Sheet<'a>, u32, &[(&'a str, &'a str)])> {
        self.sheets.iter().flat_map(move |sheet| {
            sheet
                .tiles
                .clone()
                .enumerate()
                .filter_map(move |(i, tile)| {
                    Some((sheet, u32::try_from(i).ok()?, self.tile_properties(tile)?))
                })
        })
    }

    fn tile_properties(&self, tile: usize) -> Option<&[(&'a str, &'a str)]> {
        let start = usize::try_from(*self.offsets.get(tile)?).ok()?;
        let end = usize::try_from(*self.offsets.get(tile.checked_add(1)?)?).ok()?;
        self.properties.get(start..end)
    }
}

/// Splits `fixtures_counters_01_12` into `("fixtures_counters_01", 12)`.
fn split_sprite(sprite: &str) -> Option<(&str, u32)> {
    let (sheet, index) = sprite.rsplit_once('_')?;
    let index = index.parse::<u32>().ok()?;
    Some((sheet, index))
}

impl<'a> TileDefs<'a> {
    /// Parses a `.tiles` file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not start with `tdef`, has a version other than 1, ends
    /// early, has a count larger than the file could hold, has a string that is not UTF-8, or
    /// has bytes left over after the last sheet.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libpzmap::sidecar::tiledefs::TileDefs;
    ///
    /// let bytes = std::fs::read("newtiledefinitions.tiles")?;
    /// let defs = TileDefs::parse(&bytes)?;
    /// if let Some(kind) = defs.property("appliances_refrigeration_01_20", "container") {
    ///     println!("that sprite is a {kind}");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(input: &'a [u8]) -> Result<TileDefs<'a>> {
        let mut input = Input::new(input, Format::TileDefs);
        input.magic("tdef")?;

        let version_at = input.offset();
        let version = input.u32_le()?;
        if version != SUPPORTED_VERSION {
            return Err(ParseError {
                format: Format::TileDefs,
                at: version_at,
                kind: ParseErrorKind::Version {
                    what: "tile definitions",
                    found: version,
                },
            });
        }

        let sheet_count = input.u32_le()?;
        let sheet_count =
            input.checked_count("sheet count", sheet_count as i64, min_bytes::SHEET)?;

        let mut sheets = Vec::with_capacity(sheet_count);
        let mut offsets = vec![0_u32];
        let mut properties = Vec::new();
        let mut by_name = AHashMap::with_capacity(sheet_count);

        for _ in 0..sheet_count {
            let name = input.line("sheet name")?;
            let image = input.line("sheet image")?;
            let columns = input.u32_le()?;
            let rows = input.u32_le()?;
            let number = input.u32_le()?;
            let tile_count = input.u32_le()?;
            let tile_count =
                input.checked_count("tile count", tile_count as i64, min_bytes::TILE)?;

            let first_tile = offsets.len() - 1;
            for _ in 0..tile_count {
                let property_count = input.u32_le()?;
                let property_count = input.checked_count(
                    "property count",
                    property_count as i64,
                    min_bytes::PROPERTY,
                )?;
                for _ in 0..property_count {
                    let key = input.line("property name")?;
                    let value = input.line("property value")?;
                    properties.push((key, value));
                }
                let end = u32::try_from(properties.len()).map_err(|_| ParseError {
                    format: Format::TileDefs,
                    at: input.offset(),
                    kind: ParseErrorKind::OutOfRange {
                        field: "total properties",
                        value: i64::try_from(properties.len()).unwrap_or(i64::MAX),
                        min: 0,
                        max: u32::MAX as i64,
                    },
                })?;
                offsets.push(end);
            }

            by_name.insert(name, sheets.len());
            sheets.push(Sheet {
                name,
                image,
                columns,
                rows,
                number,
                tiles: first_tile..first_tile + tile_count,
            });
        }

        input.expect_eof()?;

        Ok(TileDefs {
            version,
            sheets,
            offsets,
            properties,
            by_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type SheetSpec<'a> = (
        &'a str,
        &'a str,
        u32,
        u32,
        u32,
        Vec<Vec<(&'a str, &'a str)>>,
    );

    /// Builds a file with the given sheets, each `(name, image, cols, rows, number, tiles)`.
    #[allow(clippy::cast_possible_truncation)]
    fn file(sheets: &[SheetSpec<'_>]) -> Vec<u8> {
        let mut out = b"tdef".to_vec();
        out.extend_from_slice(&1_u32.to_le_bytes());
        out.extend_from_slice(&(sheets.len() as u32).to_le_bytes());
        for (name, image, cols, rows, number, tiles) in sheets {
            out.extend_from_slice(name.as_bytes());
            out.push(b'\n');
            out.extend_from_slice(image.as_bytes());
            out.push(b'\n');
            for n in [*cols, *rows, *number, tiles.len() as u32] {
                out.extend_from_slice(&n.to_le_bytes());
            }
            for props in tiles {
                out.extend_from_slice(&(props.len() as u32).to_le_bytes());
                for (k, v) in props {
                    out.extend_from_slice(k.as_bytes());
                    out.push(b'\n');
                    out.extend_from_slice(v.as_bytes());
                    out.push(b'\n');
                }
            }
        }
        out
    }

    fn sample() -> Vec<u8> {
        file(&[
            (
                "appliances_01",
                "appliances_01.png",
                8,
                16,
                0x58,
                vec![
                    vec![("container", "fridge"), ("solid", "")],
                    vec![],
                    vec![("IsoType", "IsoDoor")],
                ],
            ),
            ("empty_01", "empty_01.png", 8, 8, 0x60, vec![]),
            (
                "roofs_01",
                "roofs_01.png",
                8,
                8,
                0x61,
                vec![vec![("CustomName", "Roof")]],
            ),
        ])
    }

    #[test]
    fn sheets_and_tiles_are_addressable() {
        let bytes = sample();
        let defs = TileDefs::parse(&bytes).unwrap();

        assert_eq!(defs.version(), 1);
        assert_eq!(defs.sheets().len(), 3);
        assert_eq!(defs.tile_count(), 4);

        let sheet = defs.sheet("appliances_01").unwrap();
        assert_eq!((sheet.columns, sheet.rows, sheet.number), (8, 16, 0x58));
        assert_eq!(sheet.image, "appliances_01.png");
        assert_eq!(sheet.tile_count(), 3);
        assert_eq!(defs.sheet("empty_01").unwrap().tile_count(), 0);

        assert_eq!(
            defs.properties("appliances_01", 0),
            Some(&[("container", "fridge"), ("solid", "")][..])
        );
        assert_eq!(defs.properties("appliances_01", 1), Some(&[][..]));
        assert_eq!(defs.properties("appliances_01", 3), None);
        assert_eq!(defs.properties("nope", 0), None);

        // Sheets after an empty one still index correctly.
        assert_eq!(defs.find("roofs_01_0"), Some(&[("CustomName", "Roof")][..]));
        assert_eq!(
            defs.property("appliances_01_0", "container"),
            Some("fridge")
        );
        assert_eq!(defs.property("appliances_01_0", "solid"), Some(""));
        assert_eq!(defs.property("appliances_01_0", "missing"), None);
        assert_eq!(defs.property("appliances_01_2", "IsoType"), Some("IsoDoor"));
        assert_eq!(defs.find("no_underscore"), None);
        assert_eq!(defs.find("appliances_01_x"), None);

        let listed: Vec<_> = defs
            .tiles()
            .map(|(sheet, i, props)| (sheet.name, i, props.len()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("appliances_01", 0, 2),
                ("appliances_01", 1, 0),
                ("appliances_01", 2, 1),
                ("roofs_01", 0, 1),
            ]
        );
    }

    #[test]
    fn malformed_files_are_rejected() {
        let bytes = sample();

        let mut wrong_magic = bytes.clone();
        wrong_magic[0..4].copy_from_slice(b"LOTH");
        assert!(matches!(
            TileDefs::parse(&wrong_magic).unwrap_err(),
            ParseError {
                format: Format::TileDefs,
                at: 0,
                kind: ParseErrorKind::Magic { .. }
            }
        ));

        let mut wrong_version = bytes.clone();
        wrong_version[4..8].copy_from_slice(&7_u32.to_le_bytes());
        assert!(matches!(
            TileDefs::parse(&wrong_version).unwrap_err(),
            ParseError {
                at: 4,
                kind: ParseErrorKind::Version { found: 7, .. },
                ..
            }
        ));

        let mut absurd_sheets = bytes.clone();
        absurd_sheets[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            TileDefs::parse(&absurd_sheets).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::ImplausibleCount {
                    field: "sheet count",
                    ..
                },
                ..
            }
        ));

        let truncated = &bytes[..bytes.len() - 3];
        assert!(matches!(
            TileDefs::parse(truncated).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::UnexpectedEof { .. },
                ..
            }
        ));

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            TileDefs::parse(&trailing).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::TrailingBytes { len: 1, .. },
                ..
            }
        ));

        let mut bad_utf8 = bytes.clone();
        let name_at = 12;
        bad_utf8[name_at] = 0xff;
        assert!(matches!(
            TileDefs::parse(&bad_utf8).unwrap_err(),
            ParseError {
                at: 12,
                kind: ParseErrorKind::Utf8 {
                    field: "sheet name",
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn muldraugh_tile_definitions_are_consumed_whole() {
        let path = crate::fixtures::install_dir().join("media/newtiledefinitions.tiles");
        let bytes = crate::fixtures::read(&path);
        let defs = TileDefs::parse(&bytes).unwrap();

        assert!(!defs.sheets().is_empty());

        let mut containers = std::collections::BTreeSet::new();
        for (_, _, props) in defs.tiles() {
            for (k, v) in props {
                if *k == "container" {
                    containers.insert(*v);
                }
            }
        }
        assert!(containers.contains("fridge"));
        assert!(containers.contains("counter"));

        // Patch files share the layout.
        let patch =
            crate::fixtures::install_dir().join("media/tiledefinitions_noiseworks.patch.tiles");
        let patch = crate::fixtures::read(&patch);
        assert!(!TileDefs::parse(&patch).unwrap().sheets().is_empty());
    }
}
