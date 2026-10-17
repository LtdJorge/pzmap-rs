//! A reader for `streets.xml`, the list of named streets.
//!
//! Build 42 maps keep this file in their map folder under `media/maps`. It holds the street names
//! on the world map. Each street has a name, a width in squares, and a centerline: the line
//! along the middle of the street, given as points in world coordinates.
//!
//! ```xml
//! <streets version="1">
//!   <street name="Oak St" width="8">
//!     <points>
//!       <point x="10895.5" y="6666.5"/>
//!       <point x="10943.0" y="6666.0"/>
//!     </points>
//!   </street>
//! </streets>
//! ```
//!
//! [`Street::parse_all`] returns the streets in file order. It rejects malformed XML, a
//! `version` other than 1, and streets or points with missing or non-numeric numbers. It ignores
//! elements other than `street`, `points` and `point`, so new additions to the format do not
//! break it.

use crate::error::{
    Format,
    ParseError,
    ParseErrorKind,
    Result,
};
use quick_xml::{
    Reader,
    XmlVersion,
    events::{
        BytesStart,
        Event,
    },
};
use std::borrow::Cow;

/// The only `version` this reader understands.
const SUPPORTED_VERSION: u32 = 1;

/// One street: a name, a width, and the line along its middle.
///
/// The name is an owned `String` rather than borrowed from the input. The XML reader normalizes
/// attribute values, so they usually differ from the bytes in the file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Street {
    /// The street's name, such as `Oak St`.
    pub name: String,
    /// The width in squares.
    pub width: f32,
    /// The points of the centerline in world coordinates, in file order.
    ///
    /// A value ending in `.5` is the center of a square.
    pub points: Vec<(f32, f32)>,
}

impl Street {
    /// A street with the given name, width and centerline.
    pub fn new(name: impl Into<String>, width: f32, points: Vec<(f32, f32)>) -> Self {
        Street {
            name: name.into(),
            width,
            points,
        }
    }

    /// Parses every `<street>` in a `streets.xml`, in file order.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed XML, a `version` other than 1, a `<street>` without a
    /// numeric `width`, or a `<point>` outside a street or without numeric `x` and `y`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libpzmap::sidecar::streets::Street;
    ///
    /// let bytes = std::fs::read("streets.xml")?;
    /// let streets = Street::parse_all(&bytes)?;
    /// println!("{} streets", streets.len());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse_all(input: &[u8]) -> Result<Vec<Street>> {
        let mut reader = Reader::from_reader(input);
        reader.config_mut().trim_text(true);

        let mut streets: Vec<Street> = Vec::new();
        let mut open: Option<Street> = None;

        loop {
            let at = position(&reader);
            let event = reader
                .read_event()
                .map_err(|_| syntax(at, "well-formed XML"))?;
            match event {
                Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                    "streets" => check_version(&e, at)?,
                    "street" => {
                        if open.is_some() {
                            return Err(syntax(at, "`</street>` before the next `<street>`"));
                        }
                        open = Some(street_from(&e, at)?);
                    }
                    "point" => {
                        let street = open
                            .as_mut()
                            .ok_or_else(|| syntax(at, "a `<point>` inside a `<street>`"))?;
                        street.points.push(point_from(&e, at)?);
                    }
                    _ => {}
                },
                Event::End(e) if e.name().as_ref() == "street" => {
                    let street = open
                        .take()
                        .ok_or_else(|| syntax(at, "a `<street>` before `</street>`"))?;
                    streets.push(street);
                }
                Event::Eof => break,
                _ => {}
            }
        }

        if open.is_some() {
            return Err(syntax(input.len(), "`</street>`"));
        }
        Ok(streets)
    }
}

fn position<R>(reader: &Reader<R>) -> usize {
    usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX)
}

fn syntax(at: usize, expected: &'static str) -> ParseError {
    ParseError::new(Format::Streets, at, ParseErrorKind::Syntax { expected })
}

/// The normalized value of the attribute `key`, or `None` if the element does not have it.
fn attribute<'e>(
    element: &'e BytesStart<'_>,
    key: &str,
    at: usize,
) -> Result<Option<Cow<'e, str>>> {
    for attr in element.attributes() {
        let attr = attr.map_err(|_| syntax(at, "well-formed attributes"))?;
        if attr.key.as_ref() == key {
            let value = attr
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|_| syntax(at, "a known entity reference"))?;
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn number(element: &BytesStart<'_>, key: &str, at: usize, expected: &'static str) -> Result<f32> {
    attribute(element, key, at)?
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .ok_or_else(|| syntax(at, expected))
}

fn check_version(element: &BytesStart<'_>, at: usize) -> Result<()> {
    let Some(value) = attribute(element, "version", at)? else {
        return Ok(());
    };
    let found = value
        .trim()
        .parse::<u32>()
        .map_err(|_| syntax(at, "an integer `version`"))?;
    if found != SUPPORTED_VERSION {
        return Err(ParseError {
            format: Format::Streets,
            at,
            kind: ParseErrorKind::Version {
                what: "streets.xml",
                found,
            },
        });
    }
    Ok(())
}

fn street_from(element: &BytesStart<'_>, at: usize) -> Result<Street> {
    Ok(Street {
        name: attribute(element, "name", at)?
            .map(Cow::into_owned)
            .unwrap_or_default(),
        width: number(element, "width", at, "a numeric `width` on `<street>`")?,
        points: Vec::new(),
    })
}

fn point_from(element: &BytesStart<'_>, at: usize) -> Result<(f32, f32)> {
    Ok((
        number(element, "x", at, "a numeric `x` on `<point>`")?,
        number(element, "y", at, "a numeric `y` on `<point>`")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<streets version="1">
    <street name="Oak St" width="8">
        <points>
            <point x="10895.5" y="6666.5"/>
            <point x="10943.0" y="6666.0"/>
        </points>
    </street>
    <street name="Ohio &amp; 10th" width="6.5">
        <points>
            <point x="11266.0" y="6691.0"/>
        </points>
    </street>
    <street name="Empty" width="8"><points/></street>
</streets>
"#;

    fn parse(src: &str) -> Result<Vec<Street>> {
        Street::parse_all(src.as_bytes())
    }

    #[test]
    fn streets_keep_their_points_in_order() {
        let streets = parse(SAMPLE).unwrap();
        assert_eq!(streets.len(), 3);

        assert_eq!(streets[0].name, "Oak St");
        assert_eq!(streets[0].width, 8.0);
        assert_eq!(
            streets[0].points,
            vec![(10895.5, 6666.5), (10943.0, 6666.0)]
        );

        assert_eq!(streets[1].name, "Ohio & 10th");
        assert_eq!(streets[1].width, 6.5);
        assert_eq!(streets[1].points.len(), 1);

        assert!(streets[2].points.is_empty());
    }

    #[test]
    fn malformed_files_are_syntax_errors_with_offsets() {
        let err =
            parse("<streets><street name=\"x\"><points><point x=\"1\" y=\"2\"/></points></street>")
                .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::Streets,
                    at: 9,
                    kind: ParseErrorKind::Syntax {
                        expected: "a numeric `width` on `<street>`"
                    }
                }
            ),
            "{err}"
        );

        let err = parse("<streets><point x=\"1\" y=\"2\"/></streets>").unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::Syntax {
                        expected: "a `<point>` inside a `<street>`",
                        ..
                    },
                    ..
                }
            ),
            "{err}"
        );

        let err = parse("<streets><street name=\"x\" width=\"8\"><point x=\"nope\" y=\"2\"/></street></streets>")
            .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::Syntax {
                        expected: "a numeric `x` on `<point>`",
                        ..
                    },
                    ..
                }
            ),
            "{err}"
        );

        let err = parse("<streets><street name=\"x\" width=\"8\">").unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::Syntax { .. },
                    ..
                }
            ),
            "{err}"
        );

        let err = parse("<streets version=\"2\"/>").unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::Streets,
                    kind: ParseErrorKind::Version { found: 2, .. },
                    ..
                }
            ),
            "{err}"
        );

        let err = parse("<streets><street name=\"x\" width=\"8\"><street name=\"y\" width=\"8\"/></street></streets>")
            .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::Syntax {
                        expected: "`</street>` before the next `<street>`",
                        ..
                    },
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn unknown_elements_are_ignored() {
        let streets = parse(
            "<streets version=\"1\"><meta/><street name=\"a\" width=\"1\"><points><point x=\"0\" y=\"0\"><tag/></point></points><extra/></street></streets>",
        )
        .unwrap();
        assert_eq!(streets.len(), 1);
        assert_eq!(streets[0].points, vec![(0.0, 0.0)]);
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn muldraugh_streets_parse() {
        let bytes = crate::fixtures::read(&crate::fixtures::muldraugh().join("streets.xml"));
        let streets = Street::parse_all(&bytes).unwrap();
        assert!(!streets.is_empty());
        assert!(streets.iter().any(|s| !s.points.is_empty()));
        assert!(streets.iter().all(|s| !s.name.is_empty() && s.width > 0.0));
        assert!(streets.iter().any(|s| s.name == "Oak St"));
    }
}
