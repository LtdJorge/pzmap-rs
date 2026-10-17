//! Settings for the build phase.
//!
//! [`BuildConfig`] says where the map and texture packs are, which packs to load in which
//! order, and which outputs to build. [`BuildLimits`] caps the memory and input sizes the build
//! accepts.
use crate::{
    runtime::BuildTargets,
    textures::PlantsConfig,
};
use libpzmap::Build;
use std::path::PathBuf;

/// The game's 2x texture packs, in the same load order pzmap2dzi uses.
///
/// When two packs contain a sprite with the same name, the later pack wins. Packs that aren't
/// installed are skipped.
pub const VANILLA_2X_PACKS: &[&str] = &[
    "Erosion.pack",
    "ApCom.pack",
    "Overlays2x.floor.pack",
    "JumboTrees2x.pack",
    "Overlays2x.pack",
    "Tiles2x.floor.pack",
    // Build 42 keeps the large tree sprites in a pack of their own.
    "JumboTreesBigs2x.pack",
    "Tiles2x.pack",
];

/// Packs with no `1x` or `2x` in their name, which belong in a pack set of either scale.
pub const SCALE_NEUTRAL_PACKS: &[&str] = &["Erosion.pack", "ApCom.pack"];

/// The 1x versions of [`VANILLA_2X_PACKS`]. Their sprites are half the size, so they load and
/// render faster.
pub const VANILLA_1X_PACKS: &[&str] = &[
    "Erosion.pack",
    "ApCom.pack",
    "JumboTrees1x.pack",
    "Overlays1x.pack",
    "Tiles1x.floor.pack",
    "Tiles1x.pack",
];

/// Limits on memory, input file sizes and skipped cells for the build phase.
///
/// The defaults fit the vanilla map plus a large set of mods. A server that builds maps from
/// uploaded files should keep them or make them tighter. To change a limit, start from
/// [`BuildLimits::default`] and assign the fields you need.
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct BuildLimits {
    /// The largest width or height a texture page may declare, in pixels.
    ///
    /// A page is one image inside a texture pack that holds many sprites.
    pub max_page_dimension: u32,

    /// The largest number of pixels one texture page may have.
    ///
    /// This is checked as well as [`Self::max_page_dimension`]. With the defaults, a 4096 x 4096
    /// page passes and a 1 x 4,000,000,000 strip does not.
    pub max_page_pixels: u64,

    /// The largest size one texture page may have once decoded, in bytes.
    ///
    /// Pages are decoded in parallel, so decoding can hold up to this much memory per thread.
    pub max_page_bytes: u64,

    /// The largest size of all loaded sprites together, in bytes.
    pub max_blob_bytes: u64,

    /// The largest input file the build will open, in bytes.
    ///
    /// The size is checked from the file's metadata before the file is read.
    pub max_input_bytes: u64,

    /// The most memory the build's large allocations may use together, in bytes.
    pub max_build_bytes: u64,

    /// How many cells may fail to parse before the build fails. Zero by default.
    ///
    /// Cells that fail within this limit are left out of the output.
    pub max_skipped_cells: u32,

    /// The same limit as a fraction of the cells found, from 0.0 to 1.0. Zero by default.
    ///
    /// The build uses whichever of the two limits allows more cells.
    pub max_skipped_fraction: f64,
}

impl Default for BuildLimits {
    fn default() -> Self {
        BuildLimits {
            max_page_dimension: 4096,
            max_page_pixels: 4096 * 4096,
            max_page_bytes: 4096 * 4096 * 4,
            max_blob_bytes: 2 * 1024 * 1024 * 1024,
            max_input_bytes: 4 * 1024 * 1024 * 1024,
            max_build_bytes: 6 * 1024 * 1024 * 1024,
            max_skipped_cells: 0,
            max_skipped_fraction: 0.0,
        }
    }
}

/// Whether the build may assume no other process changes its input files while it reads
/// them.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum InputTrust {
    /// Memory-maps the files. This is the fastest option.
    ///
    /// Use it only when nothing else can write to the files. If a mapped file is truncated
    /// while the build reads it, the process is killed by a signal.
    Trusted,

    /// Reads each file into memory, up to [`BuildLimits::max_input_bytes`].
    ///
    /// A file that changes while it is read gives an error instead of killing the process.
    /// Lotpacks are the exception: they are always memory-mapped.
    #[default]
    Untrusted,
}

/// Everything [`build`](crate::builder::build) needs to know about one build.
///
/// [`BuildConfig::new`] takes the map and texture paths and sets defaults for the rest. Change
/// the other settings with the `with_*` methods.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct BuildConfig {
    /// The map directory, which holds the `{x}_{y}.lotheader` and `world_{x}_{y}.lotpack`
    /// files.
    pub map_path: PathBuf,
    /// Directories searched for the packs named in `pack_load_order`, in order. The first
    /// directory that has a pack wins.
    pub texture_paths: Vec<PathBuf>,
    /// Pack file names, earliest first. Sprites in later packs replace sprites with the same
    /// name in earlier packs.
    pub pack_load_order: Vec<String>,
    /// Which outputs to build.
    pub targets: BuildTargets,
    /// Season, snow and tree-size settings for vegetation.
    ///
    /// The game draws trees and plants from several sprites at run time. The build combines
    /// them into single sprites using these settings. `None` skips that step.
    pub plants: Option<PlantsConfig>,
    /// Memory, input size and skipped-cell limits.
    pub limits: BuildLimits,
    /// Which game build the map files come from.
    ///
    /// The build does not detect the version from the files. It checks the files against this
    /// setting and returns an error if they don't match.
    pub game_build: Build,

    /// Whether the input files can be memory-mapped safely. Defaults to
    /// [`InputTrust::Untrusted`], which reads them into memory instead.
    pub input_trust: InputTrust,

    /// The game's `newtiledefinitions.tiles`, which says which sprites are containers.
    ///
    /// Required when [`BuildTargets::contents`] is set, and ignored otherwise.
    pub tile_defs: Option<PathBuf>,
}

impl BuildConfig {
    /// Creates a config for the map in `map_path`, loading texture packs from `texture_paths`.
    ///
    /// The other settings default to [`VANILLA_2X_PACKS`], the raster target, the default
    /// [`PlantsConfig`] and [`BuildLimits`], Build 42 map files, untrusted input, and no tile
    /// definitions.
    pub fn new(map_path: PathBuf, texture_paths: Vec<PathBuf>) -> Self {
        BuildConfig {
            map_path,
            texture_paths,
            pack_load_order: VANILLA_2X_PACKS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            targets: BuildTargets::default(),
            plants: Some(PlantsConfig::default()),
            limits: BuildLimits::default(),
            game_build: Build::B42,
            input_trust: InputTrust::default(),
            tile_defs: None,
        }
    }

    /// Sets which outputs to build.
    #[must_use]
    pub fn with_targets(mut self, targets: BuildTargets) -> Self {
        self.targets = targets;
        self
    }

    /// Replaces the pack load order, earliest first.
    #[must_use]
    pub fn with_pack_load_order(mut self, packs: Vec<String>) -> Self {
        self.pack_load_order = packs;
        self
    }

    /// Sets the vegetation settings. `None` skips combining vegetation sprites.
    #[must_use]
    pub fn with_plants(mut self, plants: Option<PlantsConfig>) -> Self {
        self.plants = plants;
        self
    }

    /// Sets the memory, input size and skipped-cell limits.
    #[must_use]
    pub fn with_limits(mut self, limits: BuildLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets which game build the map files come from.
    #[must_use]
    pub fn with_game_build(mut self, game_build: Build) -> Self {
        self.game_build = game_build;
        self
    }

    /// Sets whether the input files can be memory-mapped.
    #[must_use]
    pub fn with_input_trust(mut self, input_trust: InputTrust) -> Self {
        self.input_trust = input_trust;
        self
    }

    /// Sets the game's `newtiledefinitions.tiles`, needed for room contents.
    #[must_use]
    pub fn with_tile_defs(mut self, tile_defs: Option<PathBuf>) -> Self {
        self.tile_defs = tile_defs;
        self
    }

    /// Returns the full path of each pack in `pack_load_order`, skipping packs that aren't
    /// found in any of `texture_paths`.
    pub fn resolve_packs(&self) -> Vec<PathBuf> {
        let mut resolved = Vec::with_capacity(self.pack_load_order.len());

        for name in &self.pack_load_order {
            if let Some(found) = self
                .texture_paths
                .iter()
                .map(|dir| dir.join(name))
                .find(|p: &PathBuf| p.is_file())
            {
                resolved.push(found);
            }
        }

        resolved
    }

    /// Puts every installed pack of one scale at the front of the load order, smallest file
    /// first.
    ///
    /// It looks in `texture_paths` for `*.pack` files whose name contains `scale_suffix`
    /// (`"2x"` or `"1x"`), plus the packs in [`SCALE_NEUTRAL_PACKS`]. Packs of the other scale
    /// are left out, because their sprites would be drawn at the wrong size. The packs already
    /// in `pack_load_order` go after the ones found, so they still win when names clash.
    #[must_use]
    pub fn with_stacked_packs(mut self, scale_suffix: &str) -> Self {
        let mut sized: Vec<(u64, String)> = Vec::new();

        for dir in &self.texture_paths {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("pack") {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.contains(scale_suffix) && !SCALE_NEUTRAL_PACKS.contains(&name) {
                    continue;
                }
                // As in `resolve_packs`, the first directory with a given name wins.
                if sized.iter().any(|(_, seen)| seen == name) {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                sized.push((metadata.len(), name.to_owned()));
            }
        }

        // Sorting by size, then name, keeps the order stable when two packs have the same size.
        sized.sort();

        let mut order: Vec<String> = sized.into_iter().map(|(_, name)| name).collect();
        order.append(&mut self.pack_load_order);
        self.pack_load_order = order;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_packs_select_by_suffix_and_order_by_size() {
        let dir = tempfile::tempdir().expect("temp dir");
        let files: &[(&str, usize)] = &[
            ("Tiles2x.pack", 300),
            ("Tiles1x.pack", 42),
            ("JumboTreesBigs2x.pack", 79),
            ("JumboTrees2x.pack", 8),
            ("ApCom.pack", 2),
            ("ApCom_old.pack", 1),
            ("blair_temp.pack", 5),
            ("UI2.pack.backup", 11),
            ("B42ChunkCaching2x.txt", 1),
        ];
        for (name, size) in files {
            std::fs::write(dir.path().join(name), vec![0_u8; *size]).expect("write pack");
        }

        let config = BuildConfig {
            map_path: PathBuf::new(),
            texture_paths: vec![dir.path().to_path_buf()],
            pack_load_order: vec!["SomeMod.pack".to_owned()],
            targets: BuildTargets::RASTER,
            plants: None,
            limits: BuildLimits::default(),
            game_build: Build::B42,
            input_trust: InputTrust::Trusted,
            tile_defs: None,
        }
        .with_stacked_packs("2x");

        assert_eq!(
            config.pack_load_order,
            [
                "ApCom.pack",
                "JumboTrees2x.pack",
                "JumboTreesBigs2x.pack",
                "Tiles2x.pack",
                // The caller's own entry stays last.
                "SomeMod.pack",
            ]
        );
    }
}
