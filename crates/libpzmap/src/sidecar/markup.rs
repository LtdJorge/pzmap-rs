//! A tokenizer for the game's rich-text markup.
//!
//! Town descriptions and many other translated texts use this markup. It is plain text with
//! tags in angle brackets that change the size, color and alignment of the text:
//!
//! ```text
//! <CENTRE> <SIZE:medium> SPRINGFIELD <LINE> <LINE><LEFT> <SIZE:small> Springfield has a school and a gas station.
//! ```
//!
//! [`tokens`] splits markup into text and tags. It never fails:
//!
//! - Tag names must match exactly, in the game's capitals. A tag this module does not know
//!   becomes [`Tag::Other`].
//! - A `<` with no `>` before the next `<` is text.
//! - The escapes `&lt;` and `&gt;` become the text tokens `<` and `>`.
//! - Newlines stay in the text. The game's text panels draw each one as a `<LINE>`.

/// A piece of markup: text or a tag.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Token<'a> {
    /// Plain text, spaces and newlines included.
    Text(&'a str),
    /// A tag.
    Tag(Tag<'a>),
}

/// A markup tag, such as `<LINE>` or `<RGB:1,0,0>`.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Tag<'a> {
    /// `<LINE>`: starts a new line.
    Line,
    /// `<BR>`: starts a new line after a blank one.
    Break,
    /// `<H1>`: large, white, centered heading text.
    H1,
    /// `<H2>`: medium, light gray, left-aligned heading text.
    H2,
    /// `<TEXT>`: normal, gray, left-aligned body text.
    Text,
    /// `<CENTRE>`: centers the current line.
    Centre,
    /// `<LEFT>`: left-aligns the current line.
    Left,
    /// `<RIGHT>`: right-aligns the current line.
    Right,
    /// `<SIZE:name>`: the font size. The game knows `small`, `medium`, `large`, `intro`,
    /// `credits1` and `credits2`.
    Size(&'a str),
    /// `<RGB:r,g,b>`: the text color, each component from 0 to 1.
    Rgb([f32; 3]),
    /// `<PUSHRGB:r,g,b>`: sets the text color and saves the previous one.
    PushRgb([f32; 3]),
    /// `<POPRGB>`: restores the color saved by the last `<PUSHRGB>`.
    PopRgb,
    /// `<GHC>`: the player's "good" highlight color.
    GoodHighlight,
    /// `<BHC>`: the player's "bad" highlight color.
    BadHighlight,
    /// `<RED>`: red text.
    Red,
    /// `<ORANGE>`: orange text.
    Orange,
    /// `<GREEN>`: green text.
    Green,
    /// `<IMAGE:texture>` or `<IMAGE:texture,width,height>`: an inline image.
    Image(Image<'a>),
    /// `<IMAGECENTRE:texture>` or `<IMAGECENTRE:texture,width,height>`: a centered image.
    ImageCentre(Image<'a>),
    /// `<VIDEOCENTRE:name,width,height>`: a centered video. It holds everything after the colon.
    VideoCentre(&'a str),
    /// `<JOYPAD:button>` or `<JOYPAD:button,width,height>`: a controller button icon.
    Joypad(Image<'a>),
    /// `<INDENT:pixels>`: the indent of following lines.
    Indent(f32),
    /// `<SETX:pixels>`: moves the pen to a horizontal position.
    SetX(f32),
    /// `<SPACE>`: a space, unless at the start of a line.
    Space,
    /// `<KEY:binding>`: the key bound to a game action, such as `<KEY:Toggle Inventory>`.
    Key(&'a str),
    /// Any other tag, or a known tag whose arguments do not parse.
    ///
    /// It holds the text between the brackets.
    Other(&'a str),
}

/// An image reference in an `<IMAGE>`, `<IMAGECENTRE>` or `<JOYPAD>` tag.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Image<'a> {
    /// The texture path or button name.
    pub name: &'a str,
    /// The width and height in pixels, if given. Otherwise the game uses the texture's size.
    pub size: Option<(f32, f32)>,
}

/// Splits `markup` into its tokens, in order.
///
/// # Examples
///
/// ```
/// use libpzmap::sidecar::markup::{Tag, Token, tokens};
///
/// let parts: Vec<Token> = tokens("<CENTRE>Muldraugh<LINE>").collect();
/// assert_eq!(
///     parts,
///     [Token::Tag(Tag::Centre), Token::Text("Muldraugh"), Token::Tag(Tag::Line)],
/// );
/// ```
pub fn tokens(markup: &str) -> Tokens<'_> {
    Tokens { rest: markup }
}

/// An iterator over the tokens of some markup, returned by [`tokens`].
#[derive(Clone, Debug)]
pub struct Tokens<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let rest = self.rest;
        if rest.is_empty() {
            return None;
        }
        for (escape, text) in [("&lt;", "<"), ("&gt;", ">")] {
            if let Some(tail) = rest.strip_prefix(escape) {
                self.rest = tail;
                return Some(Token::Text(text));
            }
        }
        if let Some((body, tail)) = tag_at(rest) {
            self.rest = tail;
            return Some(Token::Tag(tag(body)));
        }
        // Text runs up to the next `<` or escape. The search starts after the first character,
        // which is text even if it is a `<` that does not start a tag.
        let end = rest
            .char_indices()
            .skip(1)
            .find(|&(i, c)| {
                c == '<'
                    || (c == '&'
                        && rest
                            .get(i..)
                            .is_some_and(|s| s.starts_with("&lt;") || s.starts_with("&gt;")))
            })
            .map_or(rest.len(), |(i, _)| i);
        let (text, tail) = rest.split_at(end);
        self.rest = tail;
        Some(Token::Text(text))
    }
}

/// If `s` starts with a tag, returns the text between its brackets and the text after it.
fn tag_at(s: &str) -> Option<(&str, &str)> {
    let after = s.strip_prefix('<')?;
    let end = after.find(['<', '>'])?;
    let tail = after.get(end..)?.strip_prefix('>')?;
    Some((after.get(..end)?, tail))
}

fn tag(body: &str) -> Tag<'_> {
    let known = match body {
        "LINE" => Some(Tag::Line),
        "BR" => Some(Tag::Break),
        "H1" => Some(Tag::H1),
        "H2" => Some(Tag::H2),
        "TEXT" => Some(Tag::Text),
        "CENTRE" => Some(Tag::Centre),
        "LEFT" => Some(Tag::Left),
        "RIGHT" => Some(Tag::Right),
        "POPRGB" => Some(Tag::PopRgb),
        "GHC" => Some(Tag::GoodHighlight),
        "BHC" => Some(Tag::BadHighlight),
        "RED" => Some(Tag::Red),
        "ORANGE" => Some(Tag::Orange),
        "GREEN" => Some(Tag::Green),
        "SPACE" => Some(Tag::Space),
        _ => body.split_once(':').and_then(|(name, args)| match name {
            "SIZE" => Some(Tag::Size(args)),
            "RGB" => rgb(args).map(Tag::Rgb),
            "PUSHRGB" => rgb(args).map(Tag::PushRgb),
            "IMAGE" => image(args).map(Tag::Image),
            "IMAGECENTRE" => image(args).map(Tag::ImageCentre),
            "VIDEOCENTRE" => Some(Tag::VideoCentre(args)),
            "JOYPAD" => image(args).map(Tag::Joypad),
            "INDENT" => number(args).map(Tag::Indent),
            "SETX" => number(args).map(Tag::SetX),
            "KEY" => Some(Tag::Key(args)),
            _ => None,
        }),
    };
    known.unwrap_or(Tag::Other(body))
}

fn number(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// `r,g,b`.
fn rgb(args: &str) -> Option<[f32; 3]> {
    let mut parts = args.split(',').map(number);
    Some([parts.next()??, parts.next()??, parts.next()??])
}

/// `name` or `name,width,height`.
fn image(args: &str) -> Option<Image<'_>> {
    let mut parts = args.split(',');
    let name = parts.next()?.trim();
    let size = match (parts.next(), parts.next()) {
        (Some(w), Some(h)) => Some((number(w)?, number(h)?)),
        (None, _) => None,
        (Some(_), None) => return None,
    };
    Some(Image { name, size })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_town_description_tokenizes() {
        let markup = "<CENTRE> <SIZE:medium> SPRINGFIELD - A QUIET TOWN <LINE> <LINE>\n<LEFT> <SIZE:small> Springfield has a school and a gas station. <LINE> <LINE> Its motto is \"come back soon\".";
        use Token::Text;
        let tag = Token::Tag;
        let tokens: Vec<Token> = tokens(markup).collect();
        assert_eq!(
            tokens,
            [
                tag(Tag::Centre),
                Text(" "),
                tag(Tag::Size("medium")),
                Text(" SPRINGFIELD - A QUIET TOWN "),
                tag(Tag::Line),
                Text(" "),
                tag(Tag::Line),
                Text("\n"),
                tag(Tag::Left),
                Text(" "),
                tag(Tag::Size("small")),
                Text(" Springfield has a school and a gas station. "),
                tag(Tag::Line),
                Text(" "),
                tag(Tag::Line),
                Text(" Its motto is \"come back soon\"."),
            ]
        );
    }

    #[test]
    fn tags_with_arguments_parse() {
        let tags: Vec<Token> =
            tokens("<RGB:1,0.5,0><IMAGE:media/ui/x.png,32,16><IMAGECENTRE:y.png><KEY:Toggle Inventory><INDENT:20><RGB:red><bzzt><line>")
                .collect();
        assert_eq!(
            tags,
            [
                Token::Tag(Tag::Rgb([1.0, 0.5, 0.0])),
                Token::Tag(Tag::Image(Image {
                    name: "media/ui/x.png",
                    size: Some((32.0, 16.0)),
                })),
                Token::Tag(Tag::ImageCentre(Image {
                    name: "y.png",
                    size: None,
                })),
                Token::Tag(Tag::Key("Toggle Inventory")),
                Token::Tag(Tag::Indent(20.0)),
                Token::Tag(Tag::Other("RGB:red")),
                Token::Tag(Tag::Other("bzzt")),
                Token::Tag(Tag::Other("line")),
            ]
        );
    }

    #[test]
    fn stray_brackets_and_escapes_are_text() {
        let tokens: Vec<Token> = tokens("a < b <LINE>1 &lt; 2 &gt; 0 <open").collect();
        assert_eq!(
            tokens,
            [
                Token::Text("a "),
                Token::Text("< b "),
                Token::Tag(Tag::Line),
                Token::Text("1 "),
                Token::Text("<"),
                Token::Text(" 2 "),
                Token::Text(">"),
                Token::Text(" 0 "),
                Token::Text("<open"),
            ]
        );
        assert_eq!(super::tokens("").next(), None);
        assert_eq!(
            super::tokens("<>").collect::<Vec<_>>(),
            [Token::Tag(Tag::Other(""))]
        );
    }
}
