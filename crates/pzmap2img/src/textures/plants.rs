//! Which sprites make up each vegetation tile, following pzmap2dzi's rules.
//!
//! Maps use tile names such as `vegetation_trees_01_11` and `jumbo_tree_01_0` that no texture
//! pack has a sprite for. The game draws them from several sprites at run time: a trunk, a
//! seasonal layer of leaves, and sometimes flowers or snow. [`plant_mapping`] lists the sprites
//! for each such name, and the texture build combines them into one sprite.

/// Tree species as `(sprite name prefix, evergreen)`, in pzmap2dzi's order.
///
/// [`PlantsConfig::jumbo_tree_type`] and [`PlantsConfig::unify_tree_type`] index this list.
const TREE_DEF: [(&str, bool); 11] = [
    ("e_americanholly", true),
    ("e_canadianhemlock", true),
    ("e_virginiapine", true),
    ("e_riverbirch", false),
    ("e_cockspurhawthorn", false),
    ("e_dogwood", false),
    ("e_carolinasilverbell", false),
    ("e_yellowwood", false),
    ("e_easternredbud", false),
    ("e_redmaple", false),
    ("e_americanlinden", false),
];

const _: () = assert!(!TREE_DEF.is_empty());

/// Returns the [`TREE_DEF`] entry for `index`. An index past the end gives the last entry.
fn tree_def(index: usize) -> (&'static str, bool) {
    let entry = TREE_DEF.get(index.min(TREE_DEF.len() - 1));
    assert!(entry.is_some(), "TREE_DEF is empty");
    entry.copied().unwrap_or(("", false))
}

/// The season vegetation is drawn in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Season {
    /// Spring foliage.
    Spring,
    /// The first of the game's two summer foliage sprites for trees.
    Summer,
    /// The second of the game's two summer foliage sprites for trees.
    #[default]
    Summer2,
    /// Autumn foliage.
    Autumn,
    /// Winter. Deciduous trees and bushes have no leaves.
    Winter,
}

/// How to draw vegetation: season, snow, flowers and tree sizes.
///
/// The options and defaults are the same as pzmap2dzi's.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PlantsConfig {
    /// Season the vegetation is drawn in.
    pub season: Season,
    /// Draw trees and bushes covered in snow instead of with seasonal foliage.
    pub snow: bool,
    /// Add flowers to bushes and ground cover.
    pub flower: bool,
    /// Use the large bush sprites.
    pub large_bush: bool,
    /// The size of ordinary trees, from 0 to 3. Values outside that range are clamped.
    pub tree_size: i32,
    /// The size of the map's large ("jumbo") trees, from `tree_size` to 5. Values outside that
    /// range are clamped. Sizes 4 and 5 use the game's larger `JUMBO` tree sprites.
    pub jumbo_tree_size: i32,
    /// The species of the large trees, from 0 to 10. Larger values give the last species.
    pub jumbo_tree_type: usize,
    /// Leave bushes and grass out.
    pub no_ground_cover: bool,
    /// Draw every tree as this species, numbered as for `jumbo_tree_type`.
    pub unify_tree_type: Option<usize>,
}

impl Default for PlantsConfig {
    fn default() -> Self {
        PlantsConfig {
            season: Season::Summer2,
            snow: false,
            flower: false,
            large_bush: false,
            tree_size: 2,
            jumbo_tree_size: 3,
            jumbo_tree_type: 3,
            no_ground_cover: false,
            unify_tree_type: None,
        }
    }
}

/// Returns the sprites for one tree: the trunk first, then the seasonal leaves.
fn tree_textures(prefix: &str, config: &PlantsConfig, size: i32, evergreen: bool) -> Vec<String> {
    let is_jumbo = size >= 4;
    let idx = size % 4;
    let stem = format!("{prefix}{}", if is_jumbo { "JUMBO_1_" } else { "_1_" });
    let step = if is_jumbo { 2 } else { 4 };

    if config.snow {
        return vec![format!("{stem}{}", idx + step)];
    }

    let mut out = vec![format!("{stem}{idx}")];

    if !evergreen {
        let overlay = match config.season {
            Season::Spring => Some(idx + step * 2),
            Season::Summer => Some(idx + step * 3),
            Season::Summer2 => Some(idx + step * 4),
            Season::Autumn => Some(idx + step * 5),
            Season::Winter => None,
        };
        if let Some(overlay) = overlay {
            out.push(format!("{stem}{overlay}"));
        }
    }

    out
}

/// Returns each `vegetation_*` and `jumbo_tree_*` tile name with the sprites it is made of,
/// bottom first.
///
/// A name with an empty list draws nothing.
pub(crate) fn plant_mapping(config: &PlantsConfig) -> Vec<(String, Vec<String>)> {
    let mut mapping = Vec::with_capacity(98);

    // Bushes: vegetation_foliage_01_0..15.
    let (offset1, offset2) = if config.large_bush { (8, 32) } else { (0, 0) };

    let mut bushes = Vec::with_capacity(16);
    for i in 0..16_i32 {
        let trunk = i % 8;
        let mut textures = Vec::new();

        if config.snow {
            textures.push(format!("f_bushes_1_{}", trunk + offset1 + 16));
        } else {
            textures.push(format!("f_bushes_1_{}", trunk + offset1));
            match config.season {
                Season::Spring => textures.push(format!("f_bushes_1_{}", trunk + offset1 + 32)),
                Season::Summer | Season::Summer2 => {
                    textures.push(format!("f_bushes_1_{}", i + offset2 + 64))
                }
                Season::Autumn => textures.push(format!("f_bushes_1_{}", trunk + offset1 + 48)),
                Season::Winter => {}
            }
            if config.flower {
                textures.push(format!("f_bushes_1_{}", i + offset2 + 80));
            }
        }

        bushes.push(textures);
    }

    for (i, textures) in bushes.into_iter().enumerate() {
        let textures = if config.no_ground_cover {
            Vec::new()
        } else {
            textures
        };
        mapping.push((format!("vegetation_foliage_01_{i}"), textures));
    }

    // Grass: vegetation_groundcover_01_0..47. The 24 variants repeat twice.
    let mut grass = Vec::with_capacity(24);
    for i in 0..24_i32 {
        let offset = (i / 8) * 16 + 16;
        let imod8 = i % 8;
        let mut textures = Vec::new();

        match config.season {
            Season::Spring => textures.push(format!("d_plants_1_{imod8}")),
            Season::Summer | Season::Summer2 => {
                textures.push(format!("d_plants_1_{}", offset + imod8))
            }
            Season::Autumn => textures.push(format!("d_plants_1_{}", 8 + imod8)),
            Season::Winter => {}
        }
        if config.flower {
            textures.push(format!("d_plants_1_{}", offset + 8 + imod8));
        }

        grass.push(textures);
    }

    // Repeats the 24 variants to fill the 48 names.
    for (i, textures) in grass.iter().cycle().take(48).enumerate() {
        let textures = if config.no_ground_cover {
            Vec::new()
        } else {
            textures.clone()
        };
        mapping.push((format!("vegetation_groundcover_01_{i}"), textures));
    }

    // Trees: vegetation_trees_01_0..32.
    let tree_size = config.tree_size.clamp(0, 3);

    for i in 0..33_usize {
        let pick = config.unify_tree_type.unwrap_or(i % TREE_DEF.len());
        let (prefix, evergreen) = tree_def(pick);
        mapping.push((
            format!("vegetation_trees_01_{i}"),
            tree_textures(prefix, config, tree_size, evergreen),
        ));
    }

    // The single jumbo tree.
    let jumbo_size = config.jumbo_tree_size.min(5).max(tree_size);
    let jumbo_type = config.unify_tree_type.unwrap_or(config.jumbo_tree_type);
    let (prefix, evergreen) = tree_def(jumbo_type);
    mapping.push((
        "jumbo_tree_01_0".to_string(),
        tree_textures(prefix, config, jumbo_size, evergreen),
    ));

    mapping
}

/// Compares [`plant_mapping`] with pzmap2dzi's output. The fixtures in `tests/fixtures/` were
/// generated by pzmap2dzi's `plants.py`.
#[cfg(test)]
mod tests {
    use super::*;

    fn reference(label: &str) -> Vec<(String, Vec<String>)> {
        let path = format!(
            "{}/tests/fixtures/plants_{label}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));

        let mut out = Vec::new();
        for line in text.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let (key, sources) = line.split_once('\t').expect("fixture line needs a tab");
            let sources = if sources.is_empty() {
                Vec::new()
            } else {
                sources.split(',').map(str::to_string).collect()
            };
            out.push((key.to_string(), sources));
        }
        out
    }

    fn check(label: &str, config: &PlantsConfig) {
        let mut ours = plant_mapping(config);
        ours.sort_by(|a, b| a.0.cmp(&b.0));

        let theirs = reference(label);

        assert_eq!(ours.len(), theirs.len(), "{label}: key count");
        for (ours, theirs) in ours.iter().zip(&theirs) {
            assert_eq!(ours.0, theirs.0, "{label}: key order");
            assert_eq!(ours.1, theirs.1, "{label}: sources for {}", ours.0);
        }
    }

    #[test]
    fn matches_the_python_reference_for_every_config() {
        check("default", &PlantsConfig::default());

        check(
            "snow",
            &PlantsConfig {
                snow: true,
                ..Default::default()
            },
        );
        check(
            "jumbo5",
            &PlantsConfig {
                jumbo_tree_size: 5,
                ..Default::default()
            },
        );
        check(
            "spring_flower",
            &PlantsConfig {
                season: Season::Spring,
                flower: true,
                ..Default::default()
            },
        );
        check(
            "large_bush_autumn",
            &PlantsConfig {
                season: Season::Autumn,
                large_bush: true,
                ..Default::default()
            },
        );
        check(
            "winter",
            &PlantsConfig {
                season: Season::Winter,
                ..Default::default()
            },
        );
        check(
            "no_ground_cover",
            &PlantsConfig {
                no_ground_cover: true,
                ..Default::default()
            },
        );
        check(
            "unify7",
            &PlantsConfig {
                unify_tree_type: Some(7),
                ..Default::default()
            },
        );
    }
}
