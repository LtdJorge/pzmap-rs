//! Helpers for the ignored integration tests, which need a game install.

use pzmap2img::{
    config::{
        BuildConfig,
        InputTrust,
    },
    runtime::BuildTargets,
};
use std::path::PathBuf;

/// Returns the game install directory named by `PZ_INSTALL_DIR`.
///
/// # Panics
///
/// Panics if `PZ_INSTALL_DIR` is unset.
pub fn install_dir() -> PathBuf {
    std::env::var_os("PZ_INSTALL_DIR").map(PathBuf::from).expect(
        "set PZ_INSTALL_DIR to a Project Zomboid Build 42 installation to run the ignored tests",
    )
}

/// Returns a config for a vanilla 2x build of the map named by `PZ_MAP`.
///
/// `PZ_MAP` is relative to `media/maps`, for example `challengemaps/Kingsmouth`. It defaults to
/// `Muldraugh, KY`.
pub fn vanilla_muldraugh_2x() -> BuildConfig {
    let install = install_dir();
    let map = std::env::var_os("PZ_MAP").unwrap_or_else(|| "Muldraugh, KY".into());
    BuildConfig::new(
        install.join("media/maps").join(map),
        vec![install.join("media/texturepacks")],
    )
    .with_targets(BuildTargets::BOTH)
    .with_input_trust(InputTrust::Trusted)
    .with_tile_defs(Some(install.join("media/newtiledefinitions.tiles")))
}
