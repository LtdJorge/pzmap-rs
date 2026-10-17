//! Finds the installed copies of the game, for when no path is given on the command line.
//!
//! It looks first in the folder named by the `PZ_INSTALL_DIR` environment variable, then in every
//! Steam library. A Steam library is a folder where Steam installs games. The libraries that
//! Steam lists as holding the game are searched first. A copy reachable from several places is
//! reported once.

use std::{
    fmt,
    path::{
        Path,
        PathBuf,
    },
};

/// Project Zomboid's Steam app id.
const APP_ID: &str = "108600";

/// Deepest nesting of blocks that [`parse_vdf`] accepts. Steam's own files nest three levels.
const MAX_VDF_DEPTH: usize = 32;

/// The places to search, gathered from the environment or written out in tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// The value of `PZ_INSTALL_DIR`.
    pub env: Option<PathBuf>,
    /// Steam installs that may exist, in the order to try them.
    pub steam_roots: Vec<PathBuf>,
}

/// How an install was found.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The folder named by `PZ_INSTALL_DIR`.
    EnvVar,
    /// A Steam library.
    SteamLibrary,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Source::EnvVar => "PZ_INSTALL_DIR",
            Source::SteamLibrary => "Steam library",
        })
    }
}

/// One installed copy of the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// The folder holding `media`, with links resolved.
    pub dir: PathBuf,
    /// Where this copy was found.
    pub source: Source,
}

impl Install {
    /// The copy's `media/texturepacks` folder.
    pub fn texturepacks(&self) -> PathBuf {
        texturepacks(&self.dir)
    }
}

/// Why no install was found, listing what was searched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotFound {
    /// The value of `PZ_INSTALL_DIR`, if it was set.
    pub env: Option<PathBuf>,
    /// The Steam folders that were searched.
    pub steam_roots: Vec<PathBuf>,
}

impl fmt::Display for NotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "found no Project Zomboid install. Looked for Steam in ")?;
        if self.steam_roots.is_empty() {
            write!(f, "no known place")?;
        }
        for (index, root) in self.steam_roots.iter().enumerate() {
            let separator = if index == 0 { "" } else { ", " };
            write!(f, "{separator}{}", root.display())?;
        }
        match &self.env {
            Some(env) => write!(
                f,
                ". PZ_INSTALL_DIR is {}, which has no media/texturepacks",
                env.display()
            ),
            None => write!(
                f,
                ". Set PZ_INSTALL_DIR to the game's folder, the one holding media"
            ),
        }
    }
}

impl std::error::Error for NotFound {}

impl Places {
    /// Reads `PZ_INSTALL_DIR` and lists where Steam usually lives on this platform.
    pub fn detect() -> Self {
        Self {
            env: std::env::var_os("PZ_INSTALL_DIR").map(PathBuf::from),
            steam_roots: steam_roots(),
        }
    }

    /// Returns every install with texture packs, each once, in the order found.
    pub fn collect(&self) -> Vec<Install> {
        let from_env = self.env.iter().map(|env| (env.clone(), Source::EnvVar));
        let from_steam = self
            .steam_roots
            .iter()
            .flat_map(|root| libraries(root))
            .map(|library| {
                let game = library
                    .join("steamapps")
                    .join("common")
                    .join("ProjectZomboid");
                (game, Source::SteamLibrary)
            });

        let mut found: Vec<Install> = Vec::new();
        for (game, source) in from_env.chain(from_steam) {
            for dir in layouts(&game) {
                if !texturepacks(&dir).is_dir() {
                    continue;
                }
                let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
                if found.iter().all(|install| install.dir != dir) {
                    found.push(Install { dir, source });
                }
            }
        }
        found
    }

    /// Returns every install found.
    ///
    /// # Errors
    ///
    /// Returns [`NotFound`], which lists the places searched, when there is no install.
    pub fn find(&self) -> Result<Vec<Install>, NotFound> {
        let found = self.collect();
        if found.is_empty() {
            Err(NotFound {
                env: self.env.clone(),
                steam_roots: self.steam_roots.clone(),
            })
        } else {
            Ok(found)
        }
    }
}

fn texturepacks(install: &Path) -> PathBuf {
    install.join("media").join("texturepacks")
}

/// The folders under `game` that may hold the game's `media` folder.
///
/// On Windows it is `game` itself. On Linux it is `game/projectzomboid`. On macOS it is inside
/// the app bundle.
fn layouts(game: &Path) -> [PathBuf; 3] {
    [
        game.to_path_buf(),
        game.join("projectzomboid"),
        game.join("Project Zomboid.app")
            .join("Contents")
            .join("Java"),
    ]
}

/// Lists the libraries of the Steam install at `root`, with the ones holding the game first.
///
/// `root` is always a library. If `libraryfolders.vdf` cannot be read, it is the only one.
fn libraries(root: &Path) -> Vec<PathBuf> {
    let file = root.join("steamapps").join("libraryfolders.vdf");
    let listed = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| parse_vdf(&text).ok())
        .map(|entries| library_folders(&entries))
        .unwrap_or_default();

    let (mut with_game, others): (Vec<_>, Vec<_>) =
        listed.into_iter().partition(|(_, has_game)| *has_game);
    with_game.extend(others);
    let mut paths: Vec<PathBuf> = with_game.into_iter().map(|(path, _)| path).collect();
    if !paths.contains(&root.to_path_buf()) {
        paths.push(root.to_path_buf());
    }
    paths
}

/// Reads each library's path from a parsed `libraryfolders.vdf`, paired with whether the
/// library holds the game.
fn library_folders(entries: &[(String, Value)]) -> Vec<(PathBuf, bool)> {
    let Some(Value::Block(folders)) = lookup(entries, "libraryfolders") else {
        return Vec::new();
    };
    folders
        .iter()
        .filter_map(|(_, folder)| {
            let Value::Block(fields) = folder else {
                return None;
            };
            let Some(Value::Str(path)) = lookup(fields, "path") else {
                return None;
            };
            let has_game = matches!(
                lookup(fields, "apps"),
                Some(Value::Block(apps)) if lookup(apps, APP_ID).is_some()
            );
            Some((PathBuf::from(path), has_game))
        })
        .collect()
}

/// Returns the first value under `key`. Keys are compared ignoring case, as Steam does.
fn lookup<'a>(entries: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

/// Where Steam may be installed on this platform, most likely first.
fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        roots.extend(registry_steam_path());
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Library/Application Support/Steam"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".local/share/Steam"));
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }

    // `~/.steam/steam` is usually a link to `~/.local/share/Steam`, so resolve links to search
    // each install once.
    let mut unique: Vec<PathBuf> = Vec::new();
    for root in roots {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        if !unique.contains(&root) {
            unique.push(root);
        }
    }
    unique
}

/// Reads `SteamPath` from `HKEY_CURRENT_USER\Software\Valve\Steam`.
#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{
            HKEY_CURRENT_USER,
            RRF_RT_REG_SZ,
            RegGetValueW,
        },
    };

    let key: Vec<u16> = "Software\\Valve\\Steam\0".encode_utf16().collect();
    let name: Vec<u16> = "SteamPath\0".encode_utf16().collect();
    let mut buffer = vec![0u16; 1024];
    let mut bytes = u32::try_from(buffer.len() * 2).ok()?;
    // SAFETY: both names end in a NUL, and `bytes` is the buffer's size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let written = buffer.get(..usize::try_from(bytes).ok()? / 2)?;
    let text = written.split(|&unit| unit == 0).next()?;
    (!text.is_empty()).then(|| PathBuf::from(std::ffi::OsString::from_wide(text)))
}

/// A value in Valve's KeyValues text format, which Steam uses for its `.vdf` files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// A quoted string.
    Str(String),
    /// A block in braces, holding key-value pairs in file order.
    Block(Vec<(String, Value)>),
}

/// Why a KeyValues file could not be read. Offsets are byte positions in the text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum VdfError {
    #[error("unexpected {found:?} at byte {offset}")]
    Unexpected { offset: usize, found: char },
    #[error("string starting at byte {offset} never ends")]
    UnterminatedString { offset: usize },
    #[error("key at byte {offset} has no value")]
    MissingValue { offset: usize },
    #[error("unmatched closing brace at byte {offset}")]
    UnmatchedClose { offset: usize },
    #[error("{open} block(s) still open at the end of the file")]
    Unclosed { open: usize },
    #[error("blocks nest deeper than {MAX_VDF_DEPTH} at byte {offset}")]
    TooDeep { offset: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Str(String),
    Open,
    Close,
}

/// Parses KeyValues text made of quoted strings, braces and `//` comments.
///
/// Unquoted words and `#include` lines are rejected. Steam never writes them in the files read
/// here.
///
/// # Errors
///
/// Returns a [`VdfError`] describing the first thing that does not fit the format.
pub fn parse_vdf(text: &str) -> Result<Vec<(String, Value)>, VdfError> {
    // Each open block's pairs and the key it will be stored under.
    let mut stack: Vec<(String, Vec<(String, Value)>)> = Vec::new();
    let mut current: Vec<(String, Value)> = Vec::new();
    let mut pending_key: Option<(String, usize)> = None;
    let mut tokens = Tokenizer { text, position: 0 };

    while let Some((offset, token)) = tokens.next_token()? {
        match (token, pending_key.take()) {
            (Token::Str(key), None) => pending_key = Some((key, offset)),
            (Token::Str(value), Some((key, _))) => current.push((key, Value::Str(value))),
            (Token::Open, Some((key, _))) => {
                if stack.len() >= MAX_VDF_DEPTH {
                    return Err(VdfError::TooDeep { offset });
                }
                stack.push((key, std::mem::take(&mut current)));
            }
            (Token::Open, None) => return Err(VdfError::Unexpected { offset, found: '{' }),
            (Token::Close, Some((_, key_offset))) => {
                return Err(VdfError::MissingValue { offset: key_offset });
            }
            (Token::Close, None) => {
                let Some((key, mut parent)) = stack.pop() else {
                    return Err(VdfError::UnmatchedClose { offset });
                };
                parent.push((key, Value::Block(std::mem::take(&mut current))));
                current = parent;
            }
        }
    }

    if let Some((_, offset)) = pending_key {
        return Err(VdfError::MissingValue { offset });
    }
    if !stack.is_empty() {
        return Err(VdfError::Unclosed { open: stack.len() });
    }
    Ok(current)
}

struct Tokenizer<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Tokenizer<'a> {
    fn rest(&self) -> &'a str {
        self.text.get(self.position..).unwrap_or_default()
    }

    /// Returns the next token and its byte offset, or `None` at the end of the text.
    fn next_token(&mut self) -> Result<Option<(usize, Token)>, VdfError> {
        loop {
            let rest = self.rest();
            let trimmed = rest.trim_start();
            self.position += rest.len() - trimmed.len();
            if trimmed.starts_with("//") {
                let line = trimmed.find('\n').unwrap_or(trimmed.len());
                self.position += line;
                continue;
            }
            break;
        }

        let offset = self.position;
        let mut chars = self.rest().chars();
        let Some(first) = chars.next() else {
            return Ok(None);
        };
        self.position += first.len_utf8();
        match first {
            '{' => Ok(Some((offset, Token::Open))),
            '}' => Ok(Some((offset, Token::Close))),
            '"' => self.string(offset).map(|s| Some((offset, Token::Str(s)))),
            found => Err(VdfError::Unexpected { offset, found }),
        }
    }

    /// Reads a quoted string whose opening quote has already been consumed.
    ///
    /// The escapes `\\`, `\"`, `\n` and `\t` are understood. After a backslash, any other
    /// character is kept as it is.
    fn string(&mut self, start: usize) -> Result<String, VdfError> {
        let mut out = String::new();
        let mut chars = self.rest().char_indices();
        while let Some((index, c)) = chars.next() {
            match c {
                '"' => {
                    self.position += index + 1;
                    return Ok(out);
                }
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, escaped)) => out.push(escaped),
                    None => break,
                },
                other => out.push(other),
            }
        }
        Err(VdfError::UnterminatedString { offset: start })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Shaped like the file Steam writes, including a Windows path with escaped backslashes.
    const LIBRARY_FOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"contentid"		"484720118350252305"
		"totalsize"		"0"
		"apps"
		{
			"228980"		"235109032"
		}
	}
	"1"
	{
		"path"		"/opt/steam/SteamLibrary"
		"label"		""
		"apps"
		{
			"70"		"667402149"
			"108600"		"7200710660"
		}
	}
}
"#;

    #[test]
    fn a_real_library_list_parses() {
        let entries = parse_vdf(LIBRARY_FOLDERS).unwrap();
        assert_eq!(
            library_folders(&entries),
            vec![
                (PathBuf::from(r"C:\Program Files (x86)\Steam"), false),
                (PathBuf::from("/opt/steam/SteamLibrary"), true),
            ]
        );
    }

    #[test]
    fn comments_and_escapes_are_understood() {
        let entries = parse_vdf("// header\n\"a\" \"say \\\"hi\\\"\\n\" // trailing\n").unwrap();
        assert_eq!(
            entries,
            vec![("a".to_owned(), Value::Str("say \"hi\"\n".to_owned()))]
        );
    }

    #[test]
    fn malformed_text_is_an_error() {
        assert_eq!(
            parse_vdf("\"a\" { \"b\" \"c\""),
            Err(VdfError::Unclosed { open: 1 })
        );
        assert_eq!(
            parse_vdf("\"a\" \"b\" }"),
            Err(VdfError::UnmatchedClose { offset: 8 })
        );
        assert_eq!(
            parse_vdf("\"a\" \"unterminated"),
            Err(VdfError::UnterminatedString { offset: 4 })
        );
        assert_eq!(
            parse_vdf("\"a\""),
            Err(VdfError::MissingValue { offset: 0 })
        );
        assert_eq!(
            parse_vdf("\"a\" { \"b\" }"),
            Err(VdfError::MissingValue { offset: 6 })
        );
        assert_eq!(
            parse_vdf("key \"value\""),
            Err(VdfError::Unexpected {
                offset: 0,
                found: 'k'
            })
        );
        assert_eq!(
            parse_vdf("{ }"),
            Err(VdfError::Unexpected {
                offset: 0,
                found: '{'
            })
        );
        assert_eq!(
            parse_vdf("\"a\\"),
            Err(VdfError::UnterminatedString { offset: 0 })
        );
        let deep = "\"k\" {".repeat(MAX_VDF_DEPTH + 1);
        assert!(matches!(parse_vdf(&deep), Err(VdfError::TooDeep { .. })));
    }

    /// A scratch folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("pack-view-install-{}-{test}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            // Resolved, since the installs found are.
            Self(std::fs::canonicalize(&path).unwrap())
        }

        fn mkdir(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn library_list(libraries: &[(&Path, bool)]) -> String {
        let mut text = String::from("\"libraryfolders\"\n{\n");
        for (index, (path, has_game)) in libraries.iter().enumerate() {
            let apps = if *has_game {
                "\"108600\" \"1\""
            } else {
                "\"70\" \"1\""
            };
            text.push_str(&format!(
                "\"{index}\" {{ \"path\" \"{}\" \"apps\" {{ {apps} }} }}\n",
                path.display()
            ));
        }
        text.push_str("}\n");
        text
    }

    #[cfg(unix)]
    #[test]
    fn every_install_is_collected_once() {
        let scratch = Scratch::new("collect");
        let root = scratch.mkdir("Steam");
        let first = scratch.mkdir("first");
        let second = scratch.mkdir("second");
        // The first library holds the Windows layout, the second the Linux one.
        let windows = scratch.mkdir("first/steamapps/common/ProjectZomboid/media/texturepacks");
        let linux = scratch
            .mkdir("second/steamapps/common/ProjectZomboid/projectzomboid/media/texturepacks");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            library_list(&[(&root, false), (&first, false), (&second, true)]),
        )
        .unwrap();
        // A link to the first library, as `~/.steam/steam` links to `~/.local/share/Steam`.
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&first, &link).unwrap();

        let places = Places {
            env: Some(second.join("steamapps/common/ProjectZomboid/projectzomboid")),
            steam_roots: vec![scratch.0.join("missing"), root, link],
        };
        let found = places.collect();
        let dirs: Vec<PathBuf> = found.iter().map(Install::texturepacks).collect();
        assert_eq!(dirs, vec![linux, windows]);
        assert_eq!(found[0].source, Source::EnvVar);
        assert_eq!(found[1].source, Source::SteamLibrary);
    }

    #[test]
    fn every_layout_of_one_game_folder_is_tried() {
        let scratch = Scratch::new("layouts");
        let root = scratch.mkdir("Steam");
        let game = "Steam/steamapps/common/ProjectZomboid";
        let mac = scratch.mkdir(&format!(
            "{game}/Project Zomboid.app/Contents/Java/media/texturepacks"
        ));
        let linux = scratch.mkdir(&format!("{game}/projectzomboid/media/texturepacks"));
        let windows = scratch.mkdir(&format!("{game}/media/texturepacks"));
        let places = Places {
            env: None,
            steam_roots: vec![root],
        };

        // Without a library list, the Steam root is the only library.
        let dirs: Vec<PathBuf> = places.collect().iter().map(Install::texturepacks).collect();
        assert_eq!(dirs, vec![windows, linux, mac]);
    }

    #[test]
    fn nothing_found_names_the_places_searched() {
        let scratch = Scratch::new("none");
        let places = Places {
            env: Some(scratch.0.join("wrong")),
            steam_roots: vec![scratch.0.join("Steam")],
        };
        let message = places.find().unwrap_err().to_string();
        assert!(
            message.contains(&scratch.0.join("Steam").display().to_string()),
            "{message}"
        );
        assert!(message.contains("PZ_INSTALL_DIR is"), "{message}");
    }
}
