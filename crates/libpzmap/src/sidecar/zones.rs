//! A reader for map zone files: `objects.lua`, `regions.lua` and `roomtones.lua`.
//!
//! A zone is an area of the map that the game treats specially, such as a forest, a parking
//! stall, a town boundary, flowing water or a room with its own ambient sound. Each map folder
//! under `media/maps` can have these files next to its cells. Each file is one Lua table with
//! one entry per zone.
//!
//! A zone has a `type` string, sits on one layer (a level, in the game's terms), and has one of three shapes.
//! Build 41 maps use only rectangles. Build 42 adds polygons and polylines, written as
//! `geometry = "polygon"` with a flat `points` list. Any extra fields sit in a `properties`
//! table.
//!
//! [`Zone::parse_all`] returns the zones in file order. It rejects a zone with no `type`, with
//! coordinates that are not integers, or with an unknown `geometry`.

use crate::{
    error::{
        Format,
        ParseError,
        ParseErrorKind,
        Result,
    },
    sidecar::lua::{
        Key,
        LuaValue,
        Table,
        syntax,
    },
};
use std::borrow::Cow;

/// One entry of the zone table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Zone<'a> {
    /// A free-form label.
    ///
    /// It is empty for most zones. Regions and some town zones have one.
    pub name: Cow<'a, str>,
    /// The zone type, such as `Vegitation`, `Nav`, `TownZone` or `WaterFlow`.
    ///
    /// It is kept exactly as written, including the game's misspellings. A type this crate has
    /// never seen still parses.
    pub kind: Cow<'a, str>,
    /// The layer the zone sits on, from the file's `z` key.
    pub layer: i32,
    /// The zone's footprint, in world squares.
    pub geometry: Geometry,
    /// The `properties` sub-table, as `(key, value)` pairs in source order.
    pub properties: Vec<(Cow<'a, str>, LuaValue<'a>)>,
}

/// A zone's footprint.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Geometry {
    /// An axis-aligned rectangle: `x`, `y`, `width`, `height`.
    Rect {
        /// The world X coordinate of the top-left square.
        x: i32,
        /// The world Y coordinate of the top-left square.
        y: i32,
        /// The width in squares.
        width: i32,
        /// The height in squares.
        height: i32,
    },
    /// A closed ring of world squares, from `geometry = "polygon"`.
    Polygon {
        /// The vertices in file order. The last one connects back to the first.
        points: Vec<(i32, i32)>,
    },
    /// An open path of world squares, from `geometry = "polyline"`.
    Polyline {
        /// The vertices in file order.
        points: Vec<(i32, i32)>,
        /// The `lineWidth` in squares, if the file gives one.
        line_width: Option<f32>,
    },
}

impl Geometry {
    /// The smallest rectangle `(x, y, width, height)` that contains the shape.
    ///
    /// Returns `None` for a polygon or polyline with no points.
    pub fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        match self {
            Geometry::Rect {
                x,
                y,
                width,
                height,
            } => Some((*x, *y, *width, *height)),
            Geometry::Polygon { points } | Geometry::Polyline { points, .. } => {
                let (first, rest) = points.split_first()?;
                let (mut min_x, mut min_y) = *first;
                let (mut max_x, mut max_y) = *first;
                for &(x, y) in rest {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
                Some((
                    min_x,
                    min_y,
                    max_x.saturating_sub(min_x),
                    max_y.saturating_sub(min_y),
                ))
            }
        }
    }
}

impl<'a> Zone<'a> {
    /// Parses a zone file into its zones, in file order.
    ///
    /// The name the table is assigned to (`objects`, `regions` and so on) is not checked. A
    /// missing layer is 0 and a missing `name` is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if the file is not a single Lua table, or if any zone in it is
    /// malformed. Each zone must have a `type` string. A rectangle needs integer `x`, `y`,
    /// `width` and `height`. A polygon or polyline needs a `points` list with an even number of
    /// integers.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libpzmap::sidecar::zones::Zone;
    ///
    /// let bytes = std::fs::read("objects.lua")?;
    /// let zones = Zone::parse_all(&bytes)?;
    /// let parking = zones.iter().filter(|z| z.kind == "ParkingStall").count();
    /// println!("{parking} parking stalls");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse_all(input: &'a [u8]) -> Result<Vec<Zone<'a>>> {
        let LuaValue::Table(table) = LuaValue::parse(input)? else {
            return Err(syntax(0, "a table of zones"));
        };

        let mut zones = Vec::with_capacity(table.len());
        for entry in table.items() {
            let Some(fields) = entry.value.as_table() else {
                return Err(syntax(entry.at, "a zone table"));
            };
            zones.push(zone_from(fields, entry.at)?);
        }
        Ok(zones)
    }

    /// The value of the property named `key`, if the zone has one.
    pub fn property(&self, key: &str) -> Option<&LuaValue<'a>> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }
}

fn to_i32(value: i64, field: &'static str, at: usize) -> Result<i32> {
    i32::try_from(value).map_err(|_| ParseError {
        format: Format::Lua,
        at,
        kind: ParseErrorKind::OutOfRange {
            field,
            value,
            min: i32::MIN as i64,
            max: i32::MAX as i64,
        },
    })
}

fn zone_from<'a>(fields: &Table<'a>, at: usize) -> Result<Zone<'a>> {
    let string = |key: &str, expected: &'static str, required: bool| match fields.get(key) {
        Some(LuaValue::Str(s)) => Ok(Some(s.clone())),
        None if !required => Ok(None),
        _ => Err(syntax(at, expected)),
    };
    let int = |key: &'static str, required: bool| -> Result<Option<i32>> {
        match fields.get(key) {
            Some(LuaValue::Int(n)) => to_i32(*n, key, at).map(Some),
            None if !required => Ok(None),
            _ => Err(syntax(at, "an integer zone coordinate or size")),
        }
    };

    let kind = string("type", "a zone with a `type` string", true)?.unwrap_or_default();
    let name = string("name", "a string zone `name`", false)?.unwrap_or_default();
    let layer = int("z", false)?.unwrap_or_default();

    let geometry = match string("geometry", "a string `geometry`", false)?.as_deref() {
        None => Geometry::Rect {
            x: int("x", true)?.unwrap_or_default(),
            y: int("y", true)?.unwrap_or_default(),
            width: int("width", true)?.unwrap_or_default(),
            height: int("height", true)?.unwrap_or_default(),
        },
        Some("polygon") => Geometry::Polygon {
            points: points_from(fields, at)?,
        },
        Some("polyline") => {
            let line_width = match fields.get("lineWidth") {
                None | Some(LuaValue::Nil) => None,
                Some(value) => {
                    let width = value
                        .as_f64()
                        .ok_or_else(|| syntax(at, "a numeric `lineWidth`"))?;
                    // Widths are small integers in practice, and f32 holds those exactly.
                    #[allow(clippy::cast_possible_truncation)]
                    let width = width as f32;
                    Some(width)
                }
            };
            Geometry::Polyline {
                points: points_from(fields, at)?,
                line_width,
            }
        }
        Some(_) => return Err(syntax(at, "a `geometry` of \"polygon\" or \"polyline\"")),
    };

    let properties = match fields.get("properties") {
        None | Some(LuaValue::Nil) => Vec::new(),
        Some(LuaValue::Table(props)) => props
            .entries()
            .iter()
            .filter_map(|entry| match &entry.key {
                Key::Name(k) => Some((k.clone(), entry.value.clone())),
                Key::Index(_) => None,
            })
            .collect(),
        Some(_) => return Err(syntax(at, "a `properties` table")),
    };

    Ok(Zone {
        name,
        kind,
        layer,
        geometry,
        properties,
    })
}

/// Reads the flat `points = { x1, y1, x2, y2, … }` list as `(x, y)` pairs.
fn points_from(fields: &Table<'_>, at: usize) -> Result<Vec<(i32, i32)>> {
    let Some(LuaValue::Table(list)) = fields.get("points") else {
        return Err(syntax(at, "a `points` table"));
    };
    let mut flat = Vec::with_capacity(list.len());
    for entry in list.items() {
        let Some(n) = entry.value.as_i64() else {
            return Err(syntax(entry.at, "an integer point coordinate"));
        };
        flat.push(to_i32(n, "point coordinate", entry.at)?);
    }
    if flat.len() % 2 != 0 {
        return Err(syntax(at, "an even number of point coordinates"));
    }
    let xs = flat.iter().step_by(2);
    let ys = flat.iter().skip(1).step_by(2);
    Ok(xs.zip(ys).map(|(&x, &y)| (x, y)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"objects = {
  { name = "", type = "Nav", x = 12592, y = 966, z = 0, width = 8, height = 234 },
  { name = "Main St", type = "TownZone", x = 12541, y = 1090, width =32, height = 58, },
  { name = "", type = "WaterFlow", x = 8640, y = 15653, z = 0, width = 1, height = 1, properties = { WaterDirection = 315, WaterSpeed = 0.0 } },
  { name = "", type = "RoomTone", x = 1, y = 2, z = 3, width = 1, height = 1, properties = { RoomTone = "ParkRestrooms", EntireBuilding = true } },
  { name = "Factory", type = "ZombiesType", z = 0, geometry = "polygon", points = { 10, 20, 30, 20, 30, 45, 10, 45 } },
  { name = "", type = "Nav", z = 0, geometry = "polyline", lineWidth = 6, points = { 1, 2, 3, 4 } },
  { name = "", type = "Nav", z = 0, geometry = "polyline", points = { 5, 6 } }
}
"#;

    #[test]
    fn rectangles_decode_with_defaults_and_properties() {
        let zones = Zone::parse_all(SAMPLE.as_bytes()).unwrap();
        assert_eq!(zones.len(), 7);

        assert_eq!(zones[0].kind, "Nav");
        assert_eq!(
            zones[0].geometry,
            Geometry::Rect {
                x: 12592,
                y: 966,
                width: 8,
                height: 234
            }
        );
        assert!(zones[0].properties.is_empty());

        // The layer defaults to 0 and odd spacing does not matter.
        assert_eq!(zones[1].name, "Main St");
        assert_eq!(zones[1].layer, 0);
        assert_eq!(zones[1].geometry.bounds(), Some((12541, 1090, 32, 58)));

        assert_eq!(
            zones[2].property("WaterDirection"),
            Some(&LuaValue::Int(315))
        );
        assert_eq!(zones[2].property("WaterSpeed"), Some(&LuaValue::Float(0.0)));
        assert_eq!(zones[2].property("Missing"), None);

        assert_eq!(zones[3].layer, 3);
        assert_eq!(
            zones[3].property("RoomTone").and_then(LuaValue::as_str),
            Some("ParkRestrooms")
        );
        assert_eq!(
            zones[3].property("EntireBuilding"),
            Some(&LuaValue::Bool(true))
        );
    }

    #[test]
    fn polygons_and_polylines_pair_their_points() {
        let zones = Zone::parse_all(SAMPLE.as_bytes()).unwrap();

        assert_eq!(zones[4].name, "Factory");
        assert_eq!(
            zones[4].geometry,
            Geometry::Polygon {
                points: vec![(10, 20), (30, 20), (30, 45), (10, 45)]
            }
        );
        assert_eq!(zones[4].geometry.bounds(), Some((10, 20, 20, 25)));

        assert_eq!(
            zones[5].geometry,
            Geometry::Polyline {
                points: vec![(1, 2), (3, 4)],
                line_width: Some(6.0),
            }
        );
        assert_eq!(
            zones[6].geometry,
            Geometry::Polyline {
                points: vec![(5, 6)],
                line_width: None,
            }
        );
        assert_eq!(zones[6].geometry.bounds(), Some((5, 6, 0, 0)));
        assert_eq!(Geometry::Polygon { points: vec![] }.bounds(), None);
    }

    #[test]
    fn missing_or_mistyped_fields_point_at_the_zone() {
        let no_type = "objects = {\n  { x = 1, y = 2, width = 3, height = 4 } }";
        assert_eq!(
            Zone::parse_all(no_type.as_bytes()).unwrap_err(),
            ParseError {
                format: Format::Lua,
                at: 14,
                kind: ParseErrorKind::Syntax {
                    expected: "a zone with a `type` string"
                }
            }
        );

        let string_x = "objects = { { type = \"Nav\", x = \"1\", y = 2, width = 3, height = 4 } }";
        assert!(matches!(
            Zone::parse_all(string_x.as_bytes()).unwrap_err(),
            ParseError {
                at: 12,
                kind: ParseErrorKind::Syntax { .. },
                ..
            }
        ));

        let not_a_table = "objects = { 5 }";
        assert!(matches!(
            Zone::parse_all(not_a_table.as_bytes()).unwrap_err(),
            ParseError {
                at: 12,
                kind: ParseErrorKind::Syntax {
                    expected: "a zone table",
                    ..
                },
                ..
            }
        ));

        let huge = "objects = { { type = \"Nav\", x = 4294967296, y = 2, width = 3, height = 4 } }";
        assert!(matches!(
            Zone::parse_all(huge.as_bytes()).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::OutOfRange {
                    field: "x",
                    value: 4_294_967_296,
                    ..
                },
                ..
            }
        ));

        let scalar = "objects = 1";
        assert!(matches!(
            Zone::parse_all(scalar.as_bytes()).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::Syntax {
                    expected: "a table of zones",
                    ..
                },
                ..
            }
        ));

        let odd = "objects = { { type = \"Nav\", geometry = \"polygon\", points = { 1, 2, 3 } } }";
        assert!(matches!(
            Zone::parse_all(odd.as_bytes()).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::Syntax {
                    expected: "an even number of point coordinates",
                    ..
                },
                ..
            }
        ));

        let float_point =
            "objects = { { type = \"Nav\", geometry = \"polygon\", points = { 1, 2.5 } } }";
        assert!(matches!(
            Zone::parse_all(float_point.as_bytes()).unwrap_err(),
            ParseError {
                at: 64,
                kind: ParseErrorKind::Syntax {
                    expected: "an integer point coordinate",
                    ..
                },
                ..
            }
        ));

        let no_points = "objects = { { type = \"Nav\", geometry = \"polyline\" } }";
        assert!(matches!(
            Zone::parse_all(no_points.as_bytes()).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::Syntax {
                    expected: "a `points` table",
                    ..
                },
                ..
            }
        ));

        let unknown = "objects = { { type = \"Nav\", geometry = \"circle\", points = {} } }";
        assert!(matches!(
            Zone::parse_all(unknown.as_bytes()).unwrap_err(),
            ParseError {
                kind: ParseErrorKind::Syntax {
                    expected: "a `geometry` of \"polygon\" or \"polyline\"",
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn muldraugh_zones_parse() {
        let bytes = crate::fixtures::read(&crate::fixtures::muldraugh().join("objects.lua"));
        let zones = Zone::parse_all(&bytes).unwrap();
        assert!(!zones.is_empty());

        let has = |kind: &str| zones.iter().any(|z| z.kind == kind);
        for kind in ["Vegitation", "ParkingStall", "Nav", "TownZone", "Forest"] {
            assert!(has(kind), "no {kind} zone");
        }

        // Every shape occurs, and every zone has bounds.
        assert!(
            zones
                .iter()
                .any(|z| matches!(z.geometry, Geometry::Rect { .. }))
        );
        assert!(
            zones
                .iter()
                .any(|z| matches!(z.geometry, Geometry::Polygon { .. }))
        );
        assert!(zones.iter().any(|z| matches!(
            z.geometry,
            Geometry::Polyline {
                line_width: Some(_),
                ..
            }
        )));
        assert!(zones.iter().all(|z| z.geometry.bounds().is_some()));

        // Every WaterFlow zone carries a direction.
        assert!(
            zones
                .iter()
                .filter(|z| z.kind == "WaterFlow")
                .all(|z| z.property("WaterDirection").is_some())
        );

        let regions = crate::fixtures::read(&crate::fixtures::muldraugh().join("regions.lua"));
        let regions = Zone::parse_all(&regions).unwrap();
        assert!(
            regions
                .iter()
                .any(|z| z.name == "Jefferson" && z.kind == "Region")
        );
        assert!(regions.iter().any(|z| z.kind == "Mannequin"));
    }
}
