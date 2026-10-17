//! A reader for Lua files that hold a single table of data.
//!
//! Several of a map's files are written in Lua but contain only data: `objects.lua`,
//! `regions.lua`, `roomtones.lua` and `spawnpoints.lua`. Each one is a single table literal. The
//! table may be assigned to a name, returned with `return`, or returned from a function. This
//! module reads that subset of Lua and returns the table as a [`LuaValue`].
//!
//! Build 42's `spawnpoints.lua` also declares `local` tables inside the function and uses them by
//! name, either directly or through the game's `mergeTable(a, b, ...)`. The reader replaces those
//! names with the tables they refer to.
//!
//! The reader never runs any code. Anything that would need a real Lua interpreter, such as
//! other function calls or arithmetic, is rejected with [`ParseErrorKind::Syntax`] and the byte
//! offset where it appears. Only a minus sign in front of a number is allowed.
//!
//! Strings borrow from the input unless they contain escape sequences.

use crate::error::{
    Format,
    ParseError,
    ParseErrorKind,
    Result,
};
use std::borrow::Cow;

/// The deepest table nesting accepted. Deeper input is rejected so that it cannot overflow the
/// stack.
const MAX_DEPTH: u32 = 64;

/// The most values that uses of `local` tables may copy in one file.
///
/// Each use of a local copies its table. Without a limit, a small file could reuse its locals
/// to expand into an enormous value.
const MAX_COPIED: usize = 1 << 20;

/// A Lua literal.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaValue<'a> {
    /// `nil`.
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// An integer literal, including hexadecimal ones.
    Int(i64),
    /// A literal with a fraction or exponent, or an integer too large for `i64`.
    Float(f64),
    /// A quoted string, borrowed when it has no escapes.
    Str(Cow<'a, str>),
    /// A table constructor.
    Table(Table<'a>),
}

impl<'a> LuaValue<'a> {
    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            LuaValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The integer, if this is one. Floats are not converted.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            LuaValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    /// The number as a float. Integers are widened.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            LuaValue::Int(n) => Some(*n as f64),
            LuaValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// The table, if this is one.
    pub fn as_table(&self) -> Option<&Table<'a>> {
        match self {
            LuaValue::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// A table constructor, with its entries in source order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table<'a> {
    entries: Vec<Entry<'a>>,
}

/// One `key = value` or positional entry of a table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Entry<'a> {
    /// The key. Positional entries are numbered from 1 in source order, as Lua does.
    pub key: Key<'a>,
    /// The value.
    pub value: LuaValue<'a>,
    /// The byte offset where the entry starts, for error reports about its contents.
    pub at: usize,
}

/// A table key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    /// A positional entry or an explicit `[n] =` integer key.
    Index(i64),
    /// A `name =` or `["name"] =` key.
    Name(Cow<'a, str>),
}

impl<'a> Table<'a> {
    /// Every entry, in source order.
    pub fn entries(&self) -> &[Entry<'a>] {
        &self.entries
    }

    /// The number of entries, named and positional.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value under a `name =` key. The last assignment wins, as in Lua.
    pub fn get(&self, name: &str) -> Option<&LuaValue<'a>> {
        self.entries
            .iter()
            .rev()
            .find(|entry| matches!(&entry.key, Key::Name(k) if k == name))
            .map(|entry| &entry.value)
    }

    /// The positional entries, in order, without their keys.
    pub fn items(&self) -> impl Iterator<Item = &Entry<'a>> {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.key, Key::Index(_)))
    }
}

impl<'a> LuaValue<'a> {
    /// Parses a whole file: one value, optionally wrapped, followed only by whitespace and
    /// comments.
    ///
    /// The accepted wrappers are `name = value`, `local name = value`, `return value` and
    /// `function Name() local a = value ... return value end`. The name in the wrapper is
    /// discarded.
    pub(crate) fn parse(input: &'a [u8]) -> Result<LuaValue<'a>> {
        let src = source(input)?;
        let mut lexer = Lexer::new(src);

        let first = lexer.next_token()?;
        let first_at = first.at;
        let value = match first.kind {
            Token::Ident("return") => lexer.value(0)?,
            Token::Ident("function") => {
                lexer.ident("a function name")?;
                lexer.expect(Token::LParen, "`(`")?;
                lexer.expect(Token::RParen, "`)`")?;
                lexer.locals()?;
                let value = lexer.value(0)?;
                lexer.expect(Token::Ident("end"), "`end`")?;
                value
            }
            Token::Ident("local") => {
                lexer.ident("a variable name")?;
                lexer.expect(Token::Eq, "`=`")?;
                lexer.value(0)?
            }
            Token::Ident(name) if !is_keyword(name) => {
                lexer.expect(Token::Eq, "`=`")?;
                lexer.value(0)?
            }
            _ => {
                return Err(syntax(
                    first_at,
                    "`return`, `function`, or a table assignment",
                ));
            }
        };

        // A trailing `;` is legal after a statement.
        let end = lexer.next_token()?;
        let end = if end.kind == Token::Semi {
            lexer.next_token()?
        } else {
            end
        };
        if end.kind != Token::Eof {
            return Err(syntax(end.at, "end of file"));
        }

        Ok(value)
    }
}

/// Checks that a whole Lua file is UTF-8.
pub(super) fn source(input: &[u8]) -> Result<&str> {
    std::str::from_utf8(input).map_err(|e| {
        ParseError::new(
            Format::Lua,
            e.valid_up_to(),
            ParseErrorKind::Utf8 { field: "file" },
        )
    })
}

/// A Lua syntax error at byte `at`.
pub(super) fn syntax(at: usize, expected: &'static str) -> ParseError {
    ParseError::new(Format::Lua, at, ParseErrorKind::Syntax { expected })
}

/// Splits `src` into tokens, ending with [`Token::Eof`].
///
/// This is for Lua files that hold more than one literal, such as the function calls in
/// `worldmap-annotations.lua`. The caller decides what sequences of tokens are valid.
pub(super) fn tokens(src: &str) -> Result<Vec<Spanned<'_>>> {
    let mut lexer = Lexer::new(src);
    let mut out = Vec::new();
    loop {
        let token = lexer.next_token()?;
        let eof = token.kind == Token::Eof;
        out.push(token);
        if eof {
            return Ok(out);
        }
    }
}

/// Lua's reserved words. An identifier position never holds one of these.
fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "and"
            | "break"
            | "do"
            | "else"
            | "elseif"
            | "end"
            | "false"
            | "for"
            | "function"
            | "goto"
            | "if"
            | "in"
            | "local"
            | "nil"
            | "not"
            | "or"
            | "repeat"
            | "return"
            | "then"
            | "true"
            | "until"
            | "while"
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Token<'a> {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Eq,
    Comma,
    Semi,
    Minus,
    Dot,
    Colon,
    Ident(&'a str),
    Str(Cow<'a, str>),
    Int(i64),
    Float(f64),
    Eof,
}

/// A token with the byte offset it starts at.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Spanned<'a> {
    pub(super) kind: Token<'a>,
    pub(super) at: usize,
}

impl<'a> Spanned<'a> {
    fn is(&self, kind: &Token<'_>) -> bool {
        &self.kind == kind
    }
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    /// The `local` definitions seen so far. A later one with the same name hides an earlier one.
    locals: Vec<(&'a str, LuaValue<'a>)>,
    /// The number of values copied by uses of locals so far, checked against [`MAX_COPIED`].
    copied: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer {
            src,
            pos: 0,
            locals: Vec::new(),
            copied: 0,
        }
    }

    fn bytes(&self) -> &'a [u8] {
        self.src.as_bytes()
    }

    fn peek_byte(&self) -> Option<u8> {
        self.bytes().get(self.pos).copied()
    }

    fn byte_at(&self, pos: usize) -> Option<u8> {
        self.bytes().get(pos).copied()
    }

    /// The source between two byte offsets.
    ///
    /// The offsets always sit next to ASCII delimiters, so they fall on character boundaries.
    fn slice(&self, start: usize, end: usize) -> &'a str {
        self.src.get(start..end).unwrap_or_default()
    }

    /// Skips whitespace and comments.
    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            match self.peek_byte() {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) => self.pos += 1,
                Some(b'-') if self.byte_at(self.pos + 1) == Some(b'-') => {
                    let at = self.pos;
                    self.pos += 2;
                    if let Some(level) = self.long_bracket_open() {
                        self.skip_long_bracket(at, level)?;
                    } else {
                        while !matches!(self.peek_byte(), None | Some(b'\n')) {
                            self.pos += 1;
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// Consumes the opening of a long bracket, such as `[[` or `[==[`, if the cursor is at one.
    ///
    /// Returns the level, which is the number of `=` signs.
    fn long_bracket_open(&mut self) -> Option<usize> {
        if self.peek_byte() != Some(b'[') {
            return None;
        }
        let mut end = self.pos + 1;
        while self.byte_at(end) == Some(b'=') {
            end += 1;
        }
        if self.byte_at(end) != Some(b'[') {
            return None;
        }
        let level = end - self.pos - 1;
        self.pos = end + 1;
        Some(level)
    }

    /// Consumes everything up to and including the `]]` or `]==]` that closes a long bracket
    /// opened at `at`.
    fn skip_long_bracket(&mut self, at: usize, level: usize) -> Result<()> {
        loop {
            match self.peek_byte() {
                None => return Err(syntax(at, "a closing `]]`")),
                Some(b']') => {
                    let mut end = self.pos + 1;
                    while self.byte_at(end) == Some(b'=') {
                        end += 1;
                    }
                    if end - self.pos - 1 == level && self.byte_at(end) == Some(b']') {
                        self.pos = end + 1;
                        return Ok(());
                    }
                    self.pos += 1;
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn next_token(&mut self) -> Result<Spanned<'a>> {
        self.skip_trivia()?;
        let at = self.pos;
        let Some(byte) = self.peek_byte() else {
            return Ok(Spanned {
                kind: Token::Eof,
                at,
            });
        };

        let kind = match byte {
            b'{' => Token::LBrace,
            b'}' => Token::RBrace,
            b'[' => Token::LBracket,
            b']' => Token::RBracket,
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'=' => Token::Eq,
            b',' => Token::Comma,
            b';' => Token::Semi,
            b'-' => Token::Minus,
            b':' => Token::Colon,
            b'"' | b'\'' => return self.string(byte),
            b'0'..=b'9' => return self.number(),
            b'.' if self.byte_at(at + 1).is_some_and(|b| b.is_ascii_digit()) => {
                return self.number();
            }
            b'.' => Token::Dot,
            b'A'..=b'Z' | b'a'..=b'z' | b'_' => {
                let mut end = at + 1;
                while self
                    .byte_at(end)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    end += 1;
                }
                self.pos = end;
                return Ok(Spanned {
                    kind: Token::Ident(self.slice(at, end)),
                    at,
                });
            }
            _ => return Err(syntax(at, "a value, `}`, `,` or a field name")),
        };
        self.pos += 1;
        Ok(Spanned { kind, at })
    }

    /// A decimal or hexadecimal numeral. The cursor is at its first character.
    fn number(&mut self) -> Result<Spanned<'a>> {
        let at = self.pos;
        let bytes = self.bytes();

        if bytes
            .get(at..at + 2)
            .is_some_and(|p| p == b"0x" || p == b"0X")
        {
            let mut end = at + 2;
            while self.byte_at(end).is_some_and(|b| b.is_ascii_hexdigit()) {
                end += 1;
            }
            let digits = self.slice(at + 2, end);
            let value = i64::from_str_radix(digits, 16)
                .map_err(|_| syntax(at, "a hexadecimal integer that fits in 64 bits"))?;
            self.pos = end;
            return Ok(Spanned {
                kind: Token::Int(value),
                at,
            });
        }

        let mut end = at;
        let mut is_float = false;
        while self.byte_at(end).is_some_and(|b| b.is_ascii_digit()) {
            end += 1;
        }
        if self.byte_at(end) == Some(b'.') {
            is_float = true;
            end += 1;
            while self.byte_at(end).is_some_and(|b| b.is_ascii_digit()) {
                end += 1;
            }
        }
        if matches!(self.byte_at(end), Some(b'e' | b'E')) {
            is_float = true;
            end += 1;
            if matches!(self.byte_at(end), Some(b'+' | b'-')) {
                end += 1;
            }
            let digits_start = end;
            while self.byte_at(end).is_some_and(|b| b.is_ascii_digit()) {
                end += 1;
            }
            if end == digits_start {
                return Err(syntax(at, "digits after the exponent marker"));
            }
        }

        let text = self.slice(at, end);
        self.pos = end;

        if !is_float && let Ok(value) = text.parse::<i64>() {
            return Ok(Spanned {
                kind: Token::Int(value),
                at,
            });
        }
        let value = text.parse::<f64>().map_err(|_| syntax(at, "a number"))?;
        Ok(Spanned {
            kind: Token::Float(value),
            at,
        })
    }

    /// A quoted string. The cursor is at the opening quote.
    fn string(&mut self, quote: u8) -> Result<Spanned<'a>> {
        let at = self.pos;
        let mut pos = at + 1;
        let mut owned: Option<Vec<u8>> = None;

        loop {
            let Some(byte) = self.byte_at(pos) else {
                return Err(syntax(at, "a closing quote"));
            };
            match byte {
                b'\n' => return Err(syntax(pos, "a closing quote before the line ends")),
                b'\\' => {
                    let buffer = owned.get_or_insert_with(|| {
                        self.bytes().get(at + 1..pos).unwrap_or_default().to_vec()
                    });
                    pos = self.escape(pos, buffer)?;
                }
                _ if byte == quote => {
                    let kind = match owned {
                        None => Token::Str(Cow::Borrowed(self.slice(at + 1, pos))),
                        Some(bytes) => {
                            let text = String::from_utf8(bytes).map_err(|_| ParseError {
                                format: Format::Lua,
                                at,
                                kind: ParseErrorKind::Utf8 { field: "string" },
                            })?;
                            Token::Str(Cow::Owned(text))
                        }
                    };
                    self.pos = pos + 1;
                    return Ok(Spanned { kind, at });
                }
                _ => {
                    if let Some(buffer) = owned.as_mut() {
                        buffer.push(byte);
                    }
                    pos += 1;
                }
            }
        }
    }

    /// Decodes the escape sequence whose backslash is at `pos` and appends it to `out`.
    ///
    /// Returns the offset just after the sequence.
    fn escape(&self, pos: usize, out: &mut Vec<u8>) -> Result<usize> {
        let Some(code) = self.byte_at(pos + 1) else {
            return Err(syntax(pos, "an escape sequence"));
        };
        let simple = match code {
            b'n' => Some(b'\n'),
            b't' => Some(b'\t'),
            b'r' => Some(b'\r'),
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b'f' => Some(0x0c),
            b'v' => Some(0x0b),
            b'\\' => Some(b'\\'),
            b'"' => Some(b'"'),
            b'\'' => Some(b'\''),
            b'\n' => Some(b'\n'),
            _ => None,
        };
        if let Some(byte) = simple {
            out.push(byte);
            return Ok(pos + 2);
        }

        match code {
            b'x' => {
                let digits = self.slice(pos + 2, pos + 4);
                let value = (digits.len() == 2)
                    .then(|| u8::from_str_radix(digits, 16).ok())
                    .flatten()
                    .ok_or_else(|| syntax(pos, "two hex digits after `\\x`"))?;
                out.push(value);
                Ok(pos + 4)
            }
            b'z' => {
                let mut end = pos + 2;
                while self.byte_at(end).is_some_and(|b| b.is_ascii_whitespace()) {
                    end += 1;
                }
                Ok(end)
            }
            b'u' => {
                if self.byte_at(pos + 2) != Some(b'{') {
                    return Err(syntax(pos, "`{` after `\\u`"));
                }
                let mut end = pos + 3;
                while self.byte_at(end).is_some_and(|b| b.is_ascii_hexdigit()) {
                    end += 1;
                }
                if self.byte_at(end) != Some(b'}') || end == pos + 3 {
                    return Err(syntax(pos, "hex digits and `}` after `\\u{`"));
                }
                let scalar = u32::from_str_radix(self.slice(pos + 3, end), 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| syntax(pos, "a Unicode scalar value in `\\u{}`"))?;
                let mut buf = [0_u8; 4];
                out.extend_from_slice(scalar.encode_utf8(&mut buf).as_bytes());
                Ok(end + 1)
            }
            b'0'..=b'9' => {
                let mut end = pos + 1;
                while end < pos + 4 && self.byte_at(end).is_some_and(|b| b.is_ascii_digit()) {
                    end += 1;
                }
                let value = self
                    .slice(pos + 1, end)
                    .parse::<u8>()
                    .map_err(|_| syntax(pos, "a byte value below 256 after `\\`"))?;
                out.push(value);
                Ok(end)
            }
            _ => Err(syntax(pos, "a valid escape sequence")),
        }
    }

    fn expect(&mut self, kind: Token<'_>, expected: &'static str) -> Result<()> {
        let token = self.next_token()?;
        if token.is(&kind) {
            Ok(())
        } else {
            Err(syntax(token.at, expected))
        }
    }

    fn ident(&mut self, expected: &'static str) -> Result<&'a str> {
        let token = self.next_token()?;
        match token.kind {
            Token::Ident(name) if !is_keyword(name) => Ok(name),
            _ => Err(syntax(token.at, expected)),
        }
    }

    /// One value. `depth` is the number of enclosing tables.
    fn value(&mut self, depth: u32) -> Result<LuaValue<'a>> {
        let token = self.next_token()?;
        self.value_from(token, depth)
    }

    fn value_from(&mut self, token: Spanned<'a>, depth: u32) -> Result<LuaValue<'a>> {
        Ok(match token.kind {
            Token::Ident("nil") => LuaValue::Nil,
            Token::Ident("true") => LuaValue::Bool(true),
            Token::Ident("false") => LuaValue::Bool(false),
            Token::Int(n) => LuaValue::Int(n),
            Token::Float(f) => LuaValue::Float(f),
            Token::Str(s) => LuaValue::Str(s),
            Token::Minus => {
                let number = self.next_token()?;
                match number.kind {
                    Token::Int(n) => n
                        .checked_neg()
                        .map(LuaValue::Int)
                        .unwrap_or(LuaValue::Float(-(n as f64))),
                    Token::Float(f) => LuaValue::Float(-f),
                    _ => return Err(syntax(number.at, "a number after `-`")),
                }
            }
            Token::LBrace => {
                if depth >= MAX_DEPTH {
                    return Err(syntax(token.at, "a table nested less than 64 levels deep"));
                }
                LuaValue::Table(self.table(depth + 1)?)
            }
            Token::Ident("mergeTable") if self.peek_is(&Token::LParen)? => {
                self.merge_table(depth)?
            }
            Token::Ident(name) => self.local(name, token.at, depth)?,
            _ => return Err(syntax(token.at, "a value")),
        })
    }

    /// The `local name = value` statements at the start of a function body, through `return`.
    fn locals(&mut self) -> Result<()> {
        loop {
            let token = self.next_token()?;
            match token.kind {
                Token::Ident("return") => return Ok(()),
                Token::Ident("local") => {
                    let name = self.ident("a variable name")?;
                    self.expect(Token::Eq, "`=`")?;
                    let value = self.value(0)?;
                    self.locals.push((name, value));
                }
                _ => return Err(syntax(token.at, "`local` or `return`")),
            }
        }
    }

    /// A copy of the local `name`, referenced at byte `at` inside `depth` tables.
    fn local(&mut self, name: &str, at: usize, depth: u32) -> Result<LuaValue<'a>> {
        let value = self
            .locals
            .iter()
            .rev()
            .find(|(local, _)| *local == name)
            .map(|(_, value)| value)
            .ok_or_else(|| syntax(at, "a value"))?;
        let (count, nesting) = measure(value);
        if depth.saturating_add(nesting) > MAX_DEPTH {
            return Err(syntax(at, "a table nested less than 64 levels deep"));
        }
        self.copied = self.copied.saturating_add(count);
        if self.copied > MAX_COPIED {
            return Err(syntax(at, "fewer copies of local tables"));
        }
        Ok(value.clone())
    }

    /// Reads the arguments of `mergeTable(a, b, ...)`, whose name has been consumed.
    ///
    /// Like the game's function, the result holds the positional entries of every argument in
    /// order.
    fn merge_table(&mut self, depth: u32) -> Result<LuaValue<'a>> {
        self.expect(Token::LParen, "`(`")?;
        let mut entries = Vec::new();
        loop {
            let token = self.next_token()?;
            let at = token.at;
            let LuaValue::Table(table) = self.value_from(token, depth)? else {
                return Err(syntax(at, "a table"));
            };
            entries.extend(sequence(table));
            let separator = self.next_token()?;
            match separator.kind {
                Token::Comma => {}
                Token::RParen => break,
                _ => return Err(syntax(separator.at, "`)` or `,`")),
            }
        }
        for (entry, index) in entries.iter_mut().zip(1..) {
            entry.key = Key::Index(index);
        }
        Ok(LuaValue::Table(Table { entries }))
    }

    /// The entries of a table whose `{` has been consumed, through its `}`.
    fn table(&mut self, depth: u32) -> Result<Table<'a>> {
        let mut entries = Vec::new();
        let mut next_index: i64 = 1;

        loop {
            let token = self.next_token()?;
            let at = token.at;
            match token.kind {
                Token::RBrace => return Ok(Table { entries }),
                Token::Eof => return Err(syntax(at, "a value or `}`")),
                Token::LBracket => {
                    let key = match self.value(depth)? {
                        LuaValue::Int(n) => Key::Index(n),
                        LuaValue::Str(s) => Key::Name(s),
                        _ => return Err(syntax(at, "an integer or string key")),
                    };
                    self.expect(Token::RBracket, "`]`")?;
                    self.expect(Token::Eq, "`=`")?;
                    let value = self.value(depth)?;
                    entries.push(Entry { key, value, at });
                }
                Token::Ident(name) if !is_keyword(name) && self.peek_is_eq()? => {
                    self.expect(Token::Eq, "`=`")?;
                    let value = self.value(depth)?;
                    entries.push(Entry {
                        key: Key::Name(Cow::Borrowed(name)),
                        value,
                        at,
                    });
                }
                kind => {
                    let value = self.value_from(Spanned { kind, at }, depth)?;
                    entries.push(Entry {
                        key: Key::Index(next_index),
                        value,
                        at,
                    });
                    next_index = next_index.saturating_add(1);
                }
            }

            let separator = self.next_token()?;
            match separator.kind {
                Token::Comma | Token::Semi => {}
                Token::RBrace => return Ok(Table { entries }),
                _ => return Err(syntax(separator.at, "`}` or `,`")),
            }
        }
    }

    /// Whether the next token is `=`, without consuming it.
    fn peek_is_eq(&mut self) -> Result<bool> {
        self.peek_is(&Token::Eq)
    }

    /// Whether the next token is `kind`, without consuming it.
    fn peek_is(&mut self, kind: &Token<'_>) -> Result<bool> {
        let saved = self.pos;
        let token = self.next_token()?;
        self.pos = saved;
        Ok(token.is(kind))
    }
}

/// The number of values in `value` and how many tables deep it nests.
fn measure(value: &LuaValue<'_>) -> (usize, u32) {
    let LuaValue::Table(table) = value else {
        return (1, 0);
    };
    table
        .entries
        .iter()
        .map(|entry| measure(&entry.value))
        .fold((1, 1), |(count, nesting), (c, n)| {
            (count.saturating_add(c), nesting.max(n.saturating_add(1)))
        })
}

/// The entries at keys 1, 2, 3 and so on, up to the first missing or nil one.
///
/// These are the entries Lua's length operator `#` counts.
fn sequence(table: Table<'_>) -> Vec<Entry<'_>> {
    let mut by_index = std::collections::BTreeMap::new();
    for entry in table.entries {
        if let Key::Index(index) = entry.key
            && index >= 1
        {
            by_index.insert(index, entry);
        }
    }
    by_index
        .into_iter()
        .zip(1..)
        .map_while(|((index, entry), expected)| {
            (index == expected && entry.value != LuaValue::Nil).then_some(entry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Result<LuaValue<'_>> {
        LuaValue::parse(src.as_bytes())
    }

    fn table(src: &str) -> Table<'_> {
        match parse(src).unwrap() {
            LuaValue::Table(t) => t,
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[test]
    fn return_and_function_wrappers() {
        assert!(parse("return { 1 }").unwrap().as_table().is_some());
        let value = parse("function SpawnPoints()\nreturn {\n a = 1 }\nend\n").unwrap();
        assert_eq!(value.as_table().unwrap().get("a"), Some(&LuaValue::Int(1)));
        assert!(parse("local t = {}").unwrap().as_table().is_some());
    }

    #[test]
    fn spawnpoints_locals_are_substituted() {
        let src = "function SpawnPoints()
    local poor_houses = {
        { posX = 10770, posY = 10271, posZ = 0 },
        { posX = 10637, posY = 10267, posZ = 0 },
    }
    local police_station = {
    {
        posX = 10637, posY = 10418, posZ = 0 },
    }
return {
    policeofficer = mergeTable(poor_houses, police_station),
    unemployed = poor_houses,
}
end
";
        let t = table(src);
        let unemployed = t.get("unemployed").unwrap().as_table().unwrap();
        assert_eq!(unemployed.len(), 2);
        let police = t.get("policeofficer").unwrap().as_table().unwrap();
        let keys: Vec<_> = police.entries().iter().map(|e| e.key.clone()).collect();
        assert_eq!(keys, vec![Key::Index(1), Key::Index(2), Key::Index(3)]);
        let last = police.items().last().unwrap().value.as_table().unwrap();
        assert_eq!(last.get("posY"), Some(&LuaValue::Int(10418)));
    }

    #[test]
    fn locals_only_hold_literals_and_earlier_locals() {
        let cases: &[(&str, &str)] = &[
            ("local a = b", "a value"),
            ("local a = f()", "a value"),
            ("local f = {} local a = f()", "`local` or `return`"),
            ("local a = mergeTable(1)", "a table"),
            ("local a = {} local b = a.x", "`local` or `return`"),
            ("return { a }", "a value"),
            ("x = 1", "`local` or `return`"),
        ];
        for (body, expected) in cases {
            let src = format!("function F() {body} return {{}} end");
            let err = parse(&src).unwrap_err();
            assert!(
                matches!(err.kind, ParseErrorKind::Syntax { expected: e } if e == *expected),
                "{src:?}: {err}"
            );
        }
        assert!(parse("function F() local a = 1 + 1 return {} end").is_err());
    }

    #[test]
    fn positional_and_named_entries_are_numbered_like_lua() {
        let t = table("x = { 10, a = 20, 30, [7] = 40, ['b'] = 50, 60; }");
        let keys: Vec<_> = t.entries().iter().map(|e| e.key.clone()).collect();
        assert_eq!(
            keys,
            vec![
                Key::Index(1),
                Key::Name("a".into()),
                Key::Index(2),
                Key::Index(7),
                Key::Name("b".into()),
                Key::Index(3),
            ]
        );
        assert_eq!(t.items().count(), 4);
        assert_eq!(t.get("b"), Some(&LuaValue::Int(50)));
        assert_eq!(t.get("zzz"), None);
    }

    #[test]
    fn last_duplicate_key_wins() {
        let t = table("x = { a = 1, a = 2 }");
        assert_eq!(t.get("a"), Some(&LuaValue::Int(2)));
    }

    #[test]
    fn numbers_cover_the_forms_the_game_writes() {
        let t = table("x = { 12, -1, 0.0, .5, -.25, 1e3, 2.5E-1, 0x1F, 9223372036854775808 }");
        let values: Vec<_> = t.items().map(|e| e.value.clone()).collect();
        assert_eq!(
            values,
            vec![
                LuaValue::Int(12),
                LuaValue::Int(-1),
                LuaValue::Float(0.0),
                LuaValue::Float(0.5),
                LuaValue::Float(-0.25),
                LuaValue::Float(1000.0),
                LuaValue::Float(0.25),
                LuaValue::Int(31),
                LuaValue::Float(9_223_372_036_854_775_808.0),
            ]
        );
        assert_eq!(LuaValue::Int(3).as_f64(), Some(3.0));
        assert_eq!(LuaValue::Float(3.0).as_i64(), None);
    }

    #[test]
    fn strings_borrow_unless_escaped() {
        let t = table(
            r#"x = { "plain", 'single', "tab\tnew\nline", "q\"uote", "\65\x42\u{43}", "a\z
                           b" }"#,
        );
        let values: Vec<_> = t.items().map(|e| e.value.clone()).collect();
        assert!(matches!(&values[0], LuaValue::Str(Cow::Borrowed("plain"))));
        assert!(matches!(&values[1], LuaValue::Str(Cow::Borrowed("single"))));
        assert!(matches!(&values[2], LuaValue::Str(Cow::Owned(s)) if s == "tab\tnew\nline"));
        assert_eq!(values[3].as_str(), Some("q\"uote"));
        assert_eq!(values[4].as_str(), Some("ABC"));
        assert_eq!(values[5].as_str(), Some("ab"));
    }

    #[test]
    fn comments_are_skipped() {
        let t = table(
            "-- leading\nx = { -- inline\n 1, --[[ block\n comment ]] 2, --[==[ levelled ]==] 3 }\n-- trailing",
        );
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn booleans_nil_and_nesting() {
        let t = table("x = { { name = \"a\", ok = true, off = false, none = nil } }");
        let inner = t.items().next().unwrap().value.as_table().unwrap();
        assert_eq!(inner.get("ok"), Some(&LuaValue::Bool(true)));
        assert_eq!(inner.get("off"), Some(&LuaValue::Bool(false)));
        assert_eq!(inner.get("none"), Some(&LuaValue::Nil));
        assert_eq!(inner.get("name").and_then(LuaValue::as_str), Some("a"));
    }

    #[test]
    fn entry_offsets_point_at_the_source() {
        let src = "x = {\n  { a = 1 },\n  { b = 2 },\n}";
        let t = table(src);
        let ats: Vec<_> = t.entries().iter().map(|e| e.at).collect();
        assert_eq!(ats, vec![8, 21]);
        assert_eq!(&src[8..9], "{");
    }

    #[test]
    fn syntax_errors_carry_the_offset() {
        let cases: &[(&str, usize, &str)] = &[
            ("x = { 1 2 }", 8, "`}` or `,`"),
            ("x = { a = }", 10, "a value"),
            ("x = { -\"s\" }", 7, "a number after `-`"),
            ("x = {", 5, "a value or `}`"),
            ("x = { \"open }", 6, "a closing quote"),
            ("x = { \"bad\\q\" }", 10, "a valid escape sequence"),
            ("x = { f(1) }", 6, "a value"),
            ("x = { 1 } y", 10, "end of file"),
            ("{ 1 }", 0, "`return`, `function`, or a table assignment"),
            ("x = { [true] = 1 }", 6, "an integer or string key"),
            ("x = 1e", 4, "digits after the exponent marker"),
            ("--[[ never closed\nx = {}", 0, "a closing `]]`"),
        ];
        for (src, at, expected) in cases {
            let err = parse(src).unwrap_err();
            assert_eq!(
                err,
                ParseError {
                    format: Format::Lua,
                    at: *at,
                    kind: ParseErrorKind::Syntax { expected }
                },
                "{src:?}"
            );
        }
    }

    #[test]
    fn deep_nesting_is_rejected_instead_of_recursing() {
        let deep = format!("x = {}{}", "{".repeat(200), "}".repeat(200));
        let err = parse(&deep).unwrap_err();
        assert!(
            matches!(
                err,
                ParseError {
                    at: 68,
                    kind: ParseErrorKind::Syntax { .. },
                    ..
                }
            ),
            "{err}"
        );

        let fine = format!("x = {}{}", "{".repeat(64), "}".repeat(64));
        assert!(parse(&fine).is_ok());
    }

    #[test]
    fn invalid_utf8_is_reported_before_lexing() {
        let err = LuaValue::parse(b"x = { \"\xff\" }").unwrap_err();
        assert_eq!(
            err,
            ParseError {
                format: Format::Lua,
                at: 7,
                kind: ParseErrorKind::Utf8 { field: "file" }
            }
        );
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn muldraugh_sidecars_parse() {
        let dir = crate::fixtures::muldraugh();
        for file in ["objects.lua", "regions.lua", "roomtones.lua"] {
            let bytes = crate::fixtures::read(&dir.join(file));
            let value = LuaValue::parse(&bytes).unwrap();
            assert!(!value.as_table().unwrap().is_empty(), "{file}");
        }

        // Every map's spawn points name professions, each with a list of positions.
        let maps = crate::fixtures::install_dir().join("media/maps");
        let mut checked = 0;
        for map in std::fs::read_dir(&maps).unwrap() {
            let path = map.unwrap().path().join("spawnpoints.lua");
            if !path.exists() {
                continue;
            }
            let bytes = crate::fixtures::read(&path);
            let value = LuaValue::parse(&bytes).unwrap();
            let professions = value.as_table().unwrap();
            assert!(professions.get("unemployed").is_some(), "{path:?}");
            for entry in professions.entries() {
                for point in entry.value.as_table().unwrap().entries() {
                    let point = point.value.as_table().unwrap();
                    for axis in ["posX", "posY", "posZ"] {
                        assert!(
                            point.get(axis).and_then(LuaValue::as_i64).is_some(),
                            "{path:?}"
                        );
                    }
                }
            }
            checked += 1;
        }
        assert!(checked > 0);
    }
}
