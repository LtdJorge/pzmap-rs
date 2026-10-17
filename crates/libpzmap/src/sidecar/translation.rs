//! A reader for the game's translation files.
//!
//! The game keeps one directory per language under `media/lua/shared/Translate/`, such as
//! `Translate/EN/`. Each file in it maps translation keys to text. These files turn world map
//! label keys such as `MapLabel_Louisville` into names, and hold town titles and descriptions.
//!
//! Current builds write each file as one JSON object, such as `MapLabel.json`:
//!
//! ```json
//! {
//!     "MapLabel_Louisville": "LOUISVILLE",
//!     "MapLabel_OhioRiver": "OHIO RIVER"
//! }
//! ```
//!
//! Older builds write `MapLabel_EN.txt`, which looks like a Lua table:
//!
//! ```lua
//! MapLabel_EN = {
//!     MapLabel_Louisville = "LOUISVILLE",
//!     MapLabel_OhioRiver = "OHIO " ..
//!         "RIVER",
//! }
//! ```
//!
//! This format is read line by line, not as Lua:
//!
//! - A line `Key = "Text",` defines an entry. The key is everything before the first `=`.
//! - The text runs from the first to the last `"` after the `=`. Quotes inside it need no
//!   escaping.
//! - A line ending in `..` continues on the following lines that start with `"`. Each of those
//!   adds the text between its first and last quote.
//! - If a key is defined twice, the last text wins, as in a Lua table.
//! - Every other line is ignored, including the `MapLabel_EN = {` header, the closing brace,
//!   `--` comments and blank lines.
//!
//! The line format never fails to parse. A JSON file fails only if it is not a JSON object.
//! Both formats keep escapes such as `\n` as written. The text may contain the game's rich-text
//! markup, which [`markup`](super::markup) reads.

use std::{
    collections::{
        BTreeSet,
        HashMap,
    },
    fs,
    io::{
        self,
        ErrorKind,
    },
    path::{
        Path,
        PathBuf,
    },
};

/// The texts from one translation file, looked up by translation key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Translations {
    entries: HashMap<String, String>,
}

/// The two layouts a translation file can have.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranslationFormat {
    /// A JSON object of strings, such as `MapLabel.json`.
    Json,
    /// The older line format, such as `MapLabel_EN.txt`.
    Txt,
}

impl TranslationFormat {
    /// Guesses the format of a file from its contents.
    ///
    /// The file is JSON if its first character is `{`, ignoring whitespace and a byte order
    /// mark. Otherwise it is the line format.
    pub fn detect(input: &[u8]) -> Self {
        let input = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(input);
        match input.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{') => TranslationFormat::Json,
            _ => TranslationFormat::Txt,
        }
    }
}

/// A translation file that could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TranslationError {
    /// A JSON file that is not an object.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Reading a file or listing a directory failed.
    #[error("reading {}: {error}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The underlying error.
        error: io::Error,
    },

    /// A file in a translation directory is not valid JSON.
    #[error("{}: {error}", path.display())]
    File {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        error: serde_json::Error,
    },
}

impl Translations {
    /// Reads a translation file in either format, detected with [`TranslationFormat::detect`].
    ///
    /// Invalid UTF-8 is replaced rather than rejected.
    ///
    /// # Errors
    ///
    /// Returns an error if the file starts with `{` but is not a JSON object.
    pub fn parse(input: &[u8]) -> Result<Self, TranslationError> {
        Self::parse_as(input, TranslationFormat::detect(input))
    }

    /// Reads a translation file in the given format.
    ///
    /// In JSON, members whose value is not a string are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the format is JSON and the file is not a JSON object. The line format
    /// never fails.
    pub fn parse_as(input: &[u8], format: TranslationFormat) -> Result<Self, TranslationError> {
        let text = String::from_utf8_lossy(input);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let entries = match format {
            TranslationFormat::Json => parse_json(text)?,
            TranslationFormat::Txt => parse_txt(text),
        };
        Ok(Translations { entries })
    }

    /// The text for `key`, if the file defines it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Every key and its text, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the file defined no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The translation directory for one language, such as `media/lua/shared/Translate/EN/`.
///
/// The files in it are grouped by category. Current builds store the `MapLabel` category as
/// `MapLabel.json`. Older builds store it as `MapLabel_<LANG>.txt`, such as `MapLabel_EN.txt`.
#[derive(Clone, Debug)]
pub struct TranslateDir {
    path: PathBuf,
    language: String,
}

impl TranslateDir {
    /// The directory at `path`. Its name is the language code, such as `EN`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let language = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        TranslateDir { path, language }
    }

    /// The directory's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The language code, taken from the directory name.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Reads the table for `category`.
    ///
    /// It tries `<category>.json` first, then `<category>_<LANG>.txt`. Returns `None` if neither
    /// file exists.
    ///
    /// # Errors
    ///
    /// Returns an error if a file exists but cannot be read, or if the JSON file is not an
    /// object.
    pub fn table(&self, category: &str) -> Result<Option<Translations>, TranslationError> {
        let json = self.path.join(format!("{category}.json"));
        if let Some(bytes) = read_optional(&json)? {
            let table = Translations::parse_as(&bytes, TranslationFormat::Json);
            return table.map(Some).map_err(|e| match e {
                TranslationError::Json(error) => TranslationError::File { path: json, error },
                other => other,
            });
        }
        let txt = self.path.join(format!("{category}_{}.txt", self.language));
        let bytes = read_optional(&txt)?;
        Ok(bytes.map(|bytes| Translations {
            entries: parse_txt(&String::from_utf8_lossy(&bytes)),
        }))
    }

    /// Every table in the directory, keyed by category.
    ///
    /// Town files such as `Muldraugh, KY.json` are read as tables too. They have `title` and
    /// `description` keys. Other files and subdirectories are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be listed, or any error from
    /// [`TranslateDir::table`].
    pub fn tables(&self) -> Result<Vec<(String, Translations)>, TranslationError> {
        let io_error = |error| TranslationError::Io {
            path: self.path.clone(),
            error,
        };
        let suffix = format!("_{}.txt", self.language);
        let mut categories = BTreeSet::new();
        for entry in fs::read_dir(&self.path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            if !entry.file_type().map_err(io_error)?.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let category = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix(&suffix));
            if let Some(category) = category {
                categories.insert(category.to_owned());
            }
        }
        let mut tables = Vec::with_capacity(categories.len());
        for category in categories {
            if let Some(table) = self.table(&category)? {
                tables.push((category, table));
            }
        }
        Ok(tables)
    }
}

/// Reads a file that may not exist. A missing file returns `None`.
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, TranslationError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(TranslationError::Io {
            path: path.to_path_buf(),
            error,
        }),
    }
}

fn parse_json(text: &str) -> Result<HashMap<String, String>, serde_json::Error> {
    let object: HashMap<String, serde_json::Value> = serde_json::from_str(text)?;
    Ok(object
        .into_iter()
        .filter_map(|(key, value)| match value {
            serde_json::Value::String(text) => Some((key, text)),
            _ => None,
        })
        .collect())
}

/// Reads the line format described in the module documentation.
fn parse_txt(text: &str) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    // The entry being read, if its last line ended in `..` and more lines may follow.
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((_, text)) = &mut open {
            if trimmed.is_empty() || trimmed.starts_with("--") {
                continue;
            }
            if trimmed.starts_with('"') {
                text.push_str(quoted(trimmed).unwrap_or_default());
                if !trimmed.ends_with("..") {
                    entries.extend(open.take());
                }
                continue;
            }
            entries.extend(open.take());
        }
        let Some((key, text)) = entry(trimmed) else {
            continue;
        };
        let entry = (key.to_owned(), text.to_owned());
        if trimmed.ends_with("..") {
            open = Some(entry);
        } else {
            entries.extend([entry]);
        }
    }
    entries.extend(open);
    entries
}

/// The key and text of a `Key = "Text"` line, or `None` for any other line.
fn entry(line: &str) -> Option<(&str, &str)> {
    if line.starts_with("--") {
        return None;
    }
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains('"') || rest.trim_start().starts_with('{') {
        return None;
    }
    Some((key, quoted(rest)?))
}

/// The text between the first and the last `"`, or `None` if there are fewer than two quotes.
fn quoted(s: &str) -> Option<&str> {
    let start = s.find('"')?;
    let end = s.rfind('"')?;
    s.get(start + 1..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(text: &str) -> Translations {
        Translations::parse_as(text.as_bytes(), TranslationFormat::Txt).unwrap()
    }

    #[test]
    fn txt_entries_are_key_equals_quoted_text() {
        let t = txt(r#"MapLabel_EN = {
    -- towns
    MapLabel_Louisville = "LOUISVILLE",
    MapLabel_Louisville = "SECOND",
    -- MapLabel_Old = "OLD",
    ContextMenu_EvolvedRecipe_Stir fry = "Stir fry", -- a key with a space
    UI_Dashes = "--- Server status ---";
    ItemName_Base.223Box = "Box of .223 Rounds",
    MapLabel_Dots = "Wait...",
    MapLabel_Single = 'SINGLE',
    MapLabel_Number = 5,
    Tooltip_Quote = "Delilah in Brandenburg "indefinitely" - see page 2",

    Broken line without equals
}
"#);
        assert_eq!(t.get("MapLabel_Louisville"), Some("SECOND"));
        assert_eq!(
            t.get("ContextMenu_EvolvedRecipe_Stir fry"),
            Some("Stir fry")
        );
        assert_eq!(t.get("UI_Dashes"), Some("--- Server status ---"));
        assert_eq!(t.get("MapLabel_Old"), None);
        assert_eq!(t.get("ItemName_Base.223Box"), Some("Box of .223 Rounds"));
        assert_eq!(t.get("MapLabel_Dots"), Some("Wait..."));
        assert_eq!(
            t.get("Tooltip_Quote"),
            Some(r#"Delilah in Brandenburg "indefinitely" - see page 2"#)
        );
        // Only double quotes delimit text.
        assert_eq!(t.get("MapLabel_Single"), None);
        assert_eq!(t.get("MapLabel_Number"), None);
        assert_eq!(t.get("MapLabel_EN"), None);
        assert_eq!(t.len(), 6);
    }

    #[test]
    fn txt_values_continue_across_lines_ending_in_dots() {
        let t = txt(r#"Print_Text_EN = {
    Print_Text_Info = "Rangers triumph - see page 3\n"..
"BRANDENBURG FUNDS ON THE WAY\n\n"..

    -- "PHONE LINES STILL DOWN\n\n"..
    "Governor says funds are "on the way."",
    Print_Text_Next = "Next",
    Print_Text_Joined = "One" .. "Two",
}
"#);
        assert_eq!(
            t.get("Print_Text_Info"),
            Some(
                r#"Rangers triumph - see page 3\nBRANDENBURG FUNDS ON THE WAY\n\nGovernor says funds are "on the way.""#
            )
        );
        assert_eq!(t.get("Print_Text_Next"), Some("Next"));
        // Pieces on one line are not joined: the text runs from the first to the last quote.
        assert_eq!(t.get("Print_Text_Joined"), Some(r#"One" .. "Two"#));
    }

    #[test]
    fn txt_headers_are_optional() {
        assert_eq!(txt("Key = \"Value\"\n").get("Key"), Some("Value"));
        let t = txt("\u{feff}UI_EN = {\r\nKey = \"Value\"\r\n}");
        assert_eq!((t.get("Key"), t.len()), (Some("Value"), 1));
        assert_eq!(
            txt("DynamicRadio_AR {\n Key = \"Value\"\n}").get("Key"),
            Some("Value")
        );
    }

    #[test]
    fn json_tables_skip_non_strings() {
        let t = Translations::parse(
            "\u{feff} {\"MapLabel_A\": \"A\\\\nB\", \"UI_Quote\": \"say \\\"hi\\\"\", \"n\": 1}"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(t.get("MapLabel_A"), Some("A\\nB"));
        assert_eq!(t.get("UI_Quote"), Some("say \"hi\""));
        assert_eq!(t.len(), 2);
        assert!(Translations::parse(b"{ not json").is_err());
        assert!(
            Translations::parse(b"MapLabel_EN = {\n}")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn directories_read_both_formats() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("EN");
        fs::create_dir_all(dir.join("Muldraugh, KY")).unwrap();
        fs::write(dir.join("MapLabel.json"), r#"{"MapLabel_A": "A"}"#).unwrap();
        fs::write(
            dir.join("MapLabel_EN.txt"),
            "MapLabel_EN = {\n MapLabel_A = \"B\",\n}",
        )
        .unwrap();
        fs::write(dir.join("UI_EN.txt"), "UI_EN = {\n UI_Ok = \"OK\",\n}").unwrap();
        fs::write(
            dir.join("UI_FR.txt"),
            "UI_FR = {\n UI_Ok = \"D'accord\",\n}",
        )
        .unwrap();
        fs::write(dir.join("glossary.tbx"), "<martif/>").unwrap();

        let translate = TranslateDir::new(&dir);
        assert_eq!(translate.language(), "EN");
        let map_label = translate.table("MapLabel").unwrap().unwrap();
        assert_eq!(map_label.get("MapLabel_A"), Some("A"));
        assert!(translate.table("Tooltip").unwrap().is_none());
        let tables = translate.tables().unwrap();
        let names: Vec<&str> = tables.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(names, ["MapLabel", "UI"]);

        fs::write(dir.join("Broken.json"), "{").unwrap();
        assert!(matches!(
            translate.table("Broken"),
            Err(TranslationError::File { .. })
        ));
    }

    #[test]
    #[ignore = "requires an installed copy of the game"]
    fn english_tables_from_the_install() {
        let dir =
            TranslateDir::new(crate::fixtures::install_dir().join("media/lua/shared/Translate/EN"));
        let tables = dir.tables().unwrap();
        let table = |name: &str| {
            &tables
                .iter()
                .find(|(c, _)| c == name)
                .unwrap_or_else(|| panic!("no {name} table"))
                .1
        };
        assert_eq!(
            table("MapLabel").get("MapLabel_Louisville"),
            Some("LOUISVILLE")
        );
        assert!(!table("ItemName").is_empty());
        assert!(!table("Tooltip").is_empty());
        // Newspaper texts span many lines in the older format.
        assert!(
            table("Print_Text")
                .iter()
                .any(|(_, text)| text.matches("\\n").count() > 5)
        );
    }
}
