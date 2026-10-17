//! Errors from the build phase.
//!
//! Each [`BuildError`] variant carries the values involved as fields, not just a message.
//! [`BuildError::is_cell_local`] says whether an error affects only one
//! cell, which the build can skip, or the whole build.

use libpzmap::{
    Format,
    ParseError,
    ParseErrorKind,
};
use std::path::PathBuf;
use thiserror::Error;

/// An error from the build phase.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BuildError {
    /// A map file or sidecar file could not be parsed.
    #[error("parsing {}", path.display())]
    Parse {
        /// The file that failed to parse.
        path: PathBuf,
        #[source]
        source: ParseError,
    },

    /// A texture pack could not be parsed.
    #[error("parsing texture pack {pack}")]
    ParsePack {
        /// The pack's file name.
        pack: String,
        #[source]
        source: ParseError,
    },

    /// A file or directory could not be opened or read.
    #[error("{op} {}", path.display())]
    Io {
        /// What was being done, such as `"opening"`, `"mmapping"` or `"reading"`.
        op: &'static str,
        /// The file or directory involved.
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A texture page's image could not be decoded.
    #[error("decoding atlas page {page}")]
    Decode {
        /// The page's name. Page names are unique across all loaded packs.
        page: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A tile or room name could not be added to its name table, usually because the table
    /// is full.
    #[error("interning a {namespace} name")]
    Intern {
        /// Which table: tile names or room names.
        namespace: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// None of the packs in the load order were found in the texture directories.
    #[error("none of the configured texture packs were found under {searched:?}")]
    NoTexturePacks {
        /// The directories that were searched.
        searched: Vec<PathBuf>,
    },

    /// Room contents were requested, but no tile definitions file was given. That file lists
    /// which sprites are containers.
    #[error("counting room contents needs the game's newtiledefinitions.tiles (see --tile-defs)")]
    MissingTileDefs,

    /// The configured game build is one this crate can't read yet.
    #[error("{build} maps are not supported yet")]
    UnsupportedBuild {
        /// The build's name.
        build: &'static str,
    },

    /// The build would use more memory than
    /// [`BuildLimits::max_build_bytes`](crate::config::BuildLimits::max_build_bytes) allows.
    #[error(
        "the build needs {bytes} more bytes for {what}, having committed {charged} of its {ceiling}"
    )]
    OutOfBudget {
        /// What the memory was for.
        what: &'static str,
        /// The bytes asked for.
        bytes: u64,
        /// The bytes already in use.
        charged: u64,
        /// The limit.
        ceiling: u64,
    },

    /// More cells failed to parse than the skip limits in
    /// [`BuildLimits`](crate::config::BuildLimits) allow.
    #[error("{skipped} of {discovered} cells failed to parse, over the {allowed} allowed")]
    TooManyCellsSkipped {
        /// Cells that failed.
        skipped: usize,
        /// Cells found in the map directory.
        discovered: usize,
        /// Cells allowed to fail.
        allowed: usize,
    },

    /// An input file is larger than
    /// [`BuildLimits::max_input_bytes`](crate::config::BuildLimits::max_input_bytes).
    #[error("{what} {} is {bytes} bytes, over the {max} byte limit", path.display())]
    InputTooLarge {
        /// The kind of file.
        what: &'static str,
        /// The file.
        path: PathBuf,
        /// The file's size.
        bytes: u64,
        /// The limit.
        max: u64,
    },

    /// A texture page's PNG header declares a size over the page limits in
    /// [`BuildLimits`](crate::config::BuildLimits).
    #[error(
        "atlas page {page} declares {width}x{height} ({pixels} pixels), over the limit of {max}"
    )]
    PageTooLarge {
        /// The page's name.
        page: String,
        /// The declared width, in pixels.
        width: u32,
        /// The declared height, in pixels.
        height: u32,
        /// `width * height`.
        pixels: u64,
        /// The limit that was exceeded.
        max: u64,
    },

    /// The sprites the map uses would take more memory than
    /// [`BuildLimits::max_blob_bytes`](crate::config::BuildLimits::max_blob_bytes) allows.
    #[error("the selected sprites need {bytes} bytes, over the {max} byte texture budget")]
    TextureBudget {
        /// The bytes needed.
        bytes: u64,
        /// The limit.
        max: u64,
    },

    /// The sprite data would be too large for the `u32` offsets in
    /// [`TexMeta::offset`](crate::textures::TexMeta::offset).
    #[error("texture blob would be {bytes} bytes, over the {max} addressable by TexMeta::offset")]
    BlobTooLarge {
        /// The size the sprite data would have.
        bytes: usize,
        /// The largest size a `u32` offset can address.
        max: u32,
    },

    /// A sprite's size or anchor doesn't fit in [`TexMeta`](crate::textures::TexMeta), which
    /// stores the size as `u16` and the anchor as `i16`.
    #[error(
        "sprite {name} is {width}x{height} at anchor ({offset_x},{offset_y}), which does not fit in TexMeta"
    )]
    SpriteTooLarge {
        /// The sprite's name.
        name: String,
        /// The sprite's width, in pixels.
        width: u32,
        /// The sprite's height, in pixels.
        height: u32,
        /// The anchor's x offset, in pixels.
        offset_x: i32,
        /// The anchor's y offset, in pixels.
        offset_y: i32,
    },

    /// The map has more of one kind of annotation than the `u32` indices in
    /// [`Annotations`](crate::runtime::vector::Annotations) can address.
    #[error(
        "the map holds {count} {what}, over the {} a u32 range endpoint addresses",
        u32::MAX
    )]
    TooManyAnnotations {
        /// The kind of annotation, such as `"rooms"`.
        what: &'static str,
        /// How many the map has.
        count: usize,
    },

    /// A cell has more tile references than its `u32` indices can address.
    #[error(
        "cell {cell_x},{cell_y} holds {tiles} tile references, over the {max} a u32 CSR index addresses"
    )]
    CellTooLarge {
        /// The cell's x coordinate, in cells.
        cell_x: i32,
        /// The cell's y coordinate, in cells.
        cell_y: i32,
        /// How many tile references the cell has.
        tiles: usize,
        /// The most a cell may have.
        max: u32,
    },
}

/// A `Result` whose error type defaults to [`BuildError`].
pub type Result<T, E = BuildError> = core::result::Result<T, E>;

impl BuildError {
    /// Returns true if the error affects only one cell, so the build can skip that cell and
    /// continue.
    ///
    /// Only damaged lotheaders and lotpacks, and cells too large to store, count. A texture
    /// pack error is never cell-local, because every cell uses the same packs. A wrong magic
    /// number or version isn't either, because every other cell would most likely fail the
    /// same way.
    pub fn is_cell_local(&self) -> bool {
        match self {
            BuildError::CellTooLarge { .. } => true,
            BuildError::Parse { source, .. } => {
                // Other files, such as the tile definitions, describe the whole map, so a broken
                // one fails the build. Any error kind not listed here also fails the build.
                matches!(source.format, Format::LotHeader | Format::LotPack)
                    && matches!(
                        source.kind,
                        ParseErrorKind::UnexpectedEof { .. }
                            | ParseErrorKind::ImplausibleCount { .. }
                            | ParseErrorKind::OffsetOutOfBounds { .. }
                            | ParseErrorKind::TrailingBytes { .. }
                            | ParseErrorKind::Utf8 { .. }
                            | ParseErrorKind::OutOfRange { .. }
                    )
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod cell_local_tests {
    use super::*;
    use libpzmap::{
        Format,
        ParseError,
    };
    use std::path::PathBuf;

    fn parse_error(format: Format, kind: ParseErrorKind) -> BuildError {
        BuildError::Parse {
            path: PathBuf::from("cell.lotpack"),
            source: ParseError::new(format, 0, kind),
        }
    }

    #[test]
    fn only_damage_to_one_cell_is_cell_local() {
        let truncated = ParseErrorKind::UnexpectedEof {
            needed: 400,
            len: 40,
        };
        let cases = [
            (
                "a version mismatch",
                parse_error(
                    Format::LotPack,
                    ParseErrorKind::Version {
                        what: "lotpack",
                        found: 41,
                    },
                ),
                false,
            ),
            (
                "a bad magic",
                parse_error(
                    Format::LotPack,
                    ParseErrorKind::Magic {
                        expected: "LOTP",
                        found: *b"XXXX",
                    },
                ),
                false,
            ),
            (
                "a truncated lotpack",
                parse_error(Format::LotPack, truncated.clone()),
                true,
            ),
            (
                "an implausible count",
                parse_error(
                    Format::LotPack,
                    ParseErrorKind::ImplausibleCount {
                        field: "block count",
                        count: 4_000_000_000,
                        needed: 16_000_000_000,
                        available: 400,
                    },
                ),
                true,
            ),
            (
                "an offset out of bounds",
                parse_error(
                    Format::LotHeader,
                    ParseErrorKind::OffsetOutOfBounds {
                        what: "string table",
                        offset: 9_000,
                        len: 400,
                    },
                ),
                true,
            ),
            (
                "a truncated texture pack",
                parse_error(Format::TexturePack, truncated),
                false,
            ),
            (
                "a cell that is too large",
                BuildError::CellTooLarge {
                    cell_x: 1,
                    cell_y: 2,
                    tiles: 9_000_000,
                    max: 1_000_000,
                },
                true,
            ),
        ];

        for (what, err, cell_local) in cases {
            assert_eq!(err.is_cell_local(), cell_local, "{what}");
        }
    }
}
