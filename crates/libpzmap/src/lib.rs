#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod build;
pub mod error;
pub mod glossary;
mod input;
pub mod lotheader;
pub mod lotpack;
pub mod sidecar;
pub mod texturepack;

pub use build::{
    B41,
    B42,
    Build,
    VersionMetadata,
};
pub use error::{
    Format,
    ParseError,
    ParseErrorKind,
    Result,
};

/// Paths to game files for the tests that need a real Project Zomboid installation.
///
/// Set `PZ_INSTALL_DIR` to point at the installation. The tests that use it are `#[ignore]`d, so
/// a plain `cargo test` passes on a machine without the game.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::PathBuf;

    /// The Build 42 installation named by `PZ_INSTALL_DIR`.
    ///
    /// # Panics
    ///
    /// If `PZ_INSTALL_DIR` is unset.
    pub fn install_dir() -> PathBuf {
        std::env::var_os("PZ_INSTALL_DIR").map(PathBuf::from).expect(
            "set PZ_INSTALL_DIR to a Project Zomboid Build 42 installation to run the ignored tests",
        )
    }

    /// The Build 41 installation named by `PZ_B41_INSTALL_DIR` if it is set.
    ///
    /// Build 41 needs its own installation because its files differ from Build 42's. Tests that
    /// need it will fail with a message asking for it when unset.
    pub fn b41_install_dir() -> Option<PathBuf> {
        std::env::var_os("PZ_B41_INSTALL_DIR").map(PathBuf::from)
    }

    /// Path to a Build 41 lotheader, or `None` if no Build 41 installation is configured.
    pub fn b41_lotheader(map: &str, x: i32, y: i32) -> Option<PathBuf> {
        Some(
            b41_install_dir()?
                .join("media/maps")
                .join(map)
                .join(format!("{x}_{y}.lotheader")),
        )
    }

    pub fn map_dir(map: &str) -> PathBuf {
        install_dir().join("media/maps").join(map)
    }

    pub fn muldraugh() -> PathBuf {
        map_dir("Muldraugh, KY")
    }

    pub fn texture_pack(name: &str) -> PathBuf {
        install_dir().join("media/texturepacks").join(name)
    }

    pub fn lotheader(map: &str, x: i32, y: i32) -> PathBuf {
        map_dir(map).join(format!("{x}_{y}.lotheader"))
    }

    pub fn lotpack(map: &str, x: i32, y: i32) -> PathBuf {
        map_dir(map).join(format!("world_{x}_{y}.lotpack"))
    }

    /// Reads a game file, panicking with setup instructions if it cannot be read.
    pub fn read(path: &std::path::Path) -> Vec<u8> {
        std::fs::read(path).unwrap_or_else(|e| {
            panic!(
                "fixture {} is unreadable ({e}). Set PZ_INSTALL_DIR to a Project Zomboid install, \
                 or run without --ignored.",
                path.display()
            )
        })
    }
}
