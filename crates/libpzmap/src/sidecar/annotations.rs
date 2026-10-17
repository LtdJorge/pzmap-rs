//! A reader for `worldmap-annotations.lua`, the text labels on the game's world map.
//!
//! The map folder under `media/maps` should contain this file. It includes the town, place, and
//! water names on the world map. Unlike the other Lua files, it is code rather than a table:
//! a function adding one text symbol at a time to the map.
//!
//! ```lua
//! return function(mapUI)
//!     local symbolsAPI = mapUI.javaObject:getAPIv3():getSymbolsAPIv2()
//!     local symbol
//!     symbol = symbolsAPI:addUntranslatedText("MapLabel_OhioRiver", "text-water-nofade", 18600, 400)
//!     symbol:setScale(3.000)
//!     symbol:setRotation(89.3)
//!     symbol:setMinZoom(0.00)
//!     symbol:setMaxZoom(24.00)
//! end
//! ```
//!
//! The reader splits the file into Lua tokens and looks for known calls. It never runs the file.
//! Each `addUntranslatedText` or `addTranslatedText` call starts a new label. Calls to `setScale`,
//! `setRotation`, `setMinZoom` and `setMaxZoom` on the variable for a label set its properties.
//! Every other statement and call is skipped.
//!
//! [`MapText::parse_all`] returns the labels in file order. It rejects files that are not UTF-8
//! or not valid Lua tokens, and label calls whose arguments are unrecognized.

use super::lua::{
    Spanned,
    Token,
    source,
    syntax,
    tokens,
};
use crate::error::Result;
use std::collections::HashMap;

/// One text symbol from the annotations file.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct MapText {
    /// The text or translation key as written, such as `MapLabel_Louisville` or
    /// `Louisville<br>General<br>Hospital`.
    pub key: Box<str>,
    /// Whether it came from `addTranslatedText` rather than `addUntranslatedText`.
    ///
    /// The game looks up both kinds in its translation tables.
    pub translated: bool,
    /// The game's style name, such as `text-town` or `text-water-nofade`.
    pub style: String,
    /// The X coordinate of the label's anchor, in world squares.
    pub x: f64,
    /// The Y coordinate of the label's anchor, in world squares.
    pub y: f64,
    /// The size factor. It is 1 if the file never sets it.
    pub scale: f64,
    /// The clockwise rotation in degrees, as seen on the world map. It is 0 if the file never
    /// sets it.
    pub rotation: f64,
    /// The lowest world map zoom level at which the label shows. It is 0 if the file never sets
    /// it.
    pub min_zoom: f64,
    /// The highest world map zoom level at which the label shows. It is 24 if the file never
    /// sets it.
    pub max_zoom: f64,
}

impl MapText {
    /// An untranslated label at `(x, y)`, with scale, rotation, and zoom range at their defaults.
    pub fn new(key: impl AsRef<str>, style: impl Into<String>, x: f64, y: f64) -> Self {
        MapText {
            key: Box::from(key.as_ref()),
            translated: false,
            style: style.into(),
            x,
            y,
            scale: 1.0,
            rotation: 0.0,
            min_zoom: 0.0,
            max_zoom: 24.0,
        }
    }

    /// Parses every text label in a `worldmap-annotations.lua`, in file order.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use libpzmap::sidecar::annotations::MapText;
    ///
    /// let bytes = std::fs::read("worldmap-annotations.lua")?;
    /// let texts = MapText::parse_all(&bytes)?;
    /// println!("{} labels", texts.len());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ParseErrorKind::Utf8`](crate::error::ParseErrorKind::Utf8) if the file is not
    /// UTF-8. Returns [`ParseErrorKind::Syntax`](crate::error::ParseErrorKind::Syntax) if the file
    /// cannot be split into Lua tokens, or if a label call's arguments are not a string key, a
    /// string style, and two numbers. A setter called on a label with anything other than one
    /// number is also an error.
    pub fn parse_all(input: &[u8]) -> Result<Vec<MapText>> {
        let src = source(input)?;
        let tokens = tokens(src)?;

        let mut texts: Vec<MapText> = Vec::new();
        let mut bound: HashMap<&str, usize> = HashMap::new();
        let mut i = 0;
        while let Some(token) = tokens.get(i) {
            let call = match (&token.kind, tokens.get(i + 1), tokens.get(i + 2)) {
                (
                    Token::Colon,
                    Some(Spanned {
                        kind: Token::Ident(method),
                        at,
                    }),
                    Some(Spanned {
                        kind: Token::LParen,
                        ..
                    }),
                ) => Some((*method, *at)),
                _ => None,
            };
            let Some((method, at)) = call else {
                i += 1;
                continue;
            };
            let receiver = ident_at(&tokens, i.checked_sub(1));
            let (args, next) = arguments(&tokens, i + 3);

            match method {
                "addUntranslatedText" | "addTranslatedText" => {
                    let text = label(method, args.as_deref(), at)?;
                    // `name = receiver:add…(…)` binds the label to `name` for the setters.
                    if i >= 3
                        && tokens.get(i - 2).is_some_and(|t| t.kind == Token::Eq)
                        && let Some(name) = ident_at(&tokens, Some(i - 3))
                    {
                        bound.insert(name, texts.len());
                    }
                    texts.push(text);
                }
                "setScale" | "setRotation" | "setMinZoom" | "setMaxZoom" => {
                    let target = receiver
                        .and_then(|name| bound.get(name))
                        .and_then(|&index| texts.get_mut(index));
                    if let Some(text) = target {
                        let value = match args.as_deref() {
                            Some([Literal::Number(value)]) => *value,
                            _ => return Err(syntax(at, "one number")),
                        };
                        match method {
                            "setScale" => text.scale = value,
                            "setRotation" => text.rotation = value,
                            "setMinZoom" => text.min_zoom = value,
                            _ => text.max_zoom = value,
                        }
                    }
                }
                _ => {}
            }
            i = next;
        }
        Ok(texts)
    }
}

/// A literal argument of a call.
#[derive(Clone, Debug, PartialEq)]
enum Literal {
    Str(String),
    Number(f64),
    Other,
}

/// The identifier at `index`, if that token is one.
fn ident_at<'a>(tokens: &[Spanned<'a>], index: Option<usize>) -> Option<&'a str> {
    match tokens.get(index?)?.kind {
        Token::Ident(name) => Some(name),
        _ => None,
    }
}

/// Reads the arguments of a call, starting at `start`, just after the `(`.
///
/// Returns the arguments and the index just after the closing `)`. The arguments are `None` if
/// any of them is not a literal, such as a variable or a nested call. In that case the tokens
/// are skipped up to the matching `)`.
fn arguments(tokens: &[Spanned<'_>], start: usize) -> (Option<Vec<Literal>>, usize) {
    let mut args = Vec::new();
    let mut i = start;
    loop {
        let Some(token) = tokens.get(i) else {
            return (None, i);
        };
        if token.kind == Token::RParen && args.is_empty() {
            return (Some(args), i + 1);
        }
        let (value, width) = match (&token.kind, tokens.get(i + 1).map(|t| &t.kind)) {
            (Token::Str(s), _) => (Some(Literal::Str(s.clone().into_owned())), 1),
            (Token::Int(n), _) => (Some(Literal::Number(*n as f64)), 1),
            (Token::Float(f), _) => (Some(Literal::Number(*f)), 1),
            (Token::Minus, Some(Token::Int(n))) => (Some(Literal::Number(-(*n as f64))), 2),
            (Token::Minus, Some(Token::Float(f))) => (Some(Literal::Number(-*f)), 2),
            (Token::Ident("true" | "false" | "nil"), _) => (Some(Literal::Other), 1),
            _ => (None, 0),
        };
        let separator = tokens.get(i + width).map(|t| &t.kind);
        match (value, separator) {
            (Some(value), Some(Token::Comma)) => {
                args.push(value);
                i += width + 1;
            }
            (Some(value), Some(Token::RParen)) => {
                args.push(value);
                return (Some(args), i + width + 1);
            }
            _ => return (None, skip_to_close(tokens, i)),
        }
    }
}

/// The index just after the `)` that closes the call whose arguments include `from`.
fn skip_to_close(tokens: &[Spanned<'_>], from: usize) -> usize {
    let mut depth = 1_u32;
    let mut i = from;
    while let Some(token) = tokens.get(i) {
        i += 1;
        match token.kind {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    i
}

/// A label from `add*Text(key, style, x, y)`.
fn label(method: &str, args: Option<&[Literal]>, at: usize) -> Result<MapText> {
    let Some(
        [
            Literal::Str(key),
            Literal::Str(style),
            Literal::Number(x),
            Literal::Number(y),
        ],
    ) = args
    else {
        return Err(syntax(
            at,
            "string key, string style and numeric x and y arguments",
        ));
    };
    Ok(MapText {
        translated: method == "addTranslatedText",
        ..MapText::new(key.clone(), style.clone(), *x, *y)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{
        Format,
        ParseError,
        ParseErrorKind,
    };
    use std::ops::Deref;

    fn parse(src: &str) -> Result<Vec<MapText>> {
        MapText::parse_all(src.as_bytes())
    }

    const SAMPLE: &str = r#"return function(mapUI)
	local mapAPI = mapUI.javaObject:getAPIv3()
	local symbolsAPI = mapAPI:getSymbolsAPIv2()
	local symbol
	symbol = symbolsAPI:addUntranslatedText("MapLabel_SaltRiver", "text-water-nofade", 12511, 6734)
	symbol:setRGBA(0.000, 0.000, 0.000, 0.000)
	symbol:setScale(0.666)
	symbol:setAnchor(0.50, 0.50)
	symbol:setRotation(345.8)
	symbol:setMatchPerspective(true)
	symbol:setMinZoom(13.50)
	symbol:setMaxZoom(16.50)

	-- A second label on the same variable.
	symbol = symbolsAPI:addTranslatedText("Louisville<br>Expo Center", "text-building", 12976.5, -1292)
	symbol:setScale(0.600)
end
"#;

    #[test]
    fn setters_apply_to_the_label_they_follow() {
        let texts = parse(SAMPLE).unwrap();
        assert_eq!(
            texts,
            vec![
                MapText {
                    key: "MapLabel_SaltRiver".into(),
                    translated: false,
                    style: "text-water-nofade".into(),
                    x: 12511.0,
                    y: 6734.0,
                    scale: 0.666,
                    rotation: 345.8,
                    min_zoom: 13.5,
                    max_zoom: 16.5,
                },
                MapText {
                    key: "Louisville<br>Expo Center".into(),
                    translated: true,
                    style: "text-building".into(),
                    x: 12976.5,
                    y: -1292.0,
                    scale: 0.6,
                    rotation: 0.0,
                    min_zoom: 0.0,
                    max_zoom: 24.0,
                },
            ]
        );
    }

    #[test]
    fn setters_on_unknown_variables_and_other_calls_are_ignored() {
        let texts = parse(
            "local other = api:addTexture(\"x\", 1, 2)\nother:setScale(9)\n\
             local s = api:addUntranslatedText(\"A\", \"text-town\", 1, 2)\n\
             print(s:getScale(f(1, g(2))))\n",
        )
        .unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].scale, 1.0);
    }

    #[test]
    fn non_literal_label_arguments_are_errors() {
        let err = parse("s = api:addUntranslatedText(name, \"text-town\", 1, 2)").unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    format: Format::Lua,
                    at: 8,
                    kind: ParseErrorKind::Syntax { .. }
                }
            ),
            "{err}"
        );

        let err = parse("s = api:addUntranslatedText(\"A\", \"text-town\", 1, 2)\ns:setScale(big)")
            .unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    kind: ParseErrorKind::Syntax {
                        expected: "one number",
                        ..
                    },
                    ..
                }
            ),
            "{err}"
        );

        assert!(parse("s = api:addUntranslatedText(\"open").is_err());
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn muldraugh_annotations_parse() {
        let path = crate::fixtures::muldraugh().join("worldmap-annotations.lua");
        let texts = MapText::parse_all(&crate::fixtures::read(&path)).unwrap();
        assert!(!texts.is_empty());
        assert!(texts.iter().any(|t| t.style == "text-town"));
        assert!(
            texts
                .iter()
                .any(|t| t.key.deref() == "MapLabel_Louisville" || t.key.contains("Louisville"))
        );
        assert!(texts.iter().all(|t| t.max_zoom > t.min_zoom));
    }
}
