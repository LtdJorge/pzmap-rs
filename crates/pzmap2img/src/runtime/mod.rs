//! The output of the build phase.
//!
//! There are two independent outputs. [`raster::Map`] holds every square's tiles and the
//! sprites to draw them, for the bitmap tiles. [`vector::Annotations`] holds rooms, buildings
//! and zombie spawn density, for vector tiles and overlays. A build that asks for only one of
//! them doesn't spend time or memory on the other.

pub mod raster;
pub mod size;
pub mod vector;

/// Which outputs a build should produce.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BuildTargets {
    /// Build the [`raster::Map`] for bitmap tiles.
    pub raster: bool,
    /// Build the [`vector::Annotations`].
    pub vector: bool,
    /// Also count the containers in each room. This reads the lotpacks and needs the tile
    /// definitions. It implies `vector`.
    pub contents: bool,
}

impl BuildTargets {
    /// Only the bitmap map.
    pub const RASTER: BuildTargets = BuildTargets {
        raster: true,
        vector: false,
        contents: false,
    };
    /// Only the annotations, without room contents. No lotpack is read.
    pub const VECTOR: BuildTargets = BuildTargets {
        raster: false,
        vector: true,
        contents: false,
    };
    /// The bitmap map and the annotations, without room contents.
    pub const BOTH: BuildTargets = BuildTargets {
        raster: true,
        vector: true,
        contents: false,
    };
    /// Everything, including room contents.
    pub const ALL: BuildTargets = BuildTargets {
        raster: true,
        vector: true,
        contents: true,
    };

    /// Returns the same targets with room contents added.
    #[must_use]
    pub const fn with_contents(self) -> Self {
        BuildTargets {
            vector: true,
            contents: true,
            ..self
        }
    }

    /// Returns true if the build produces [`vector::Annotations`].
    #[must_use]
    pub const fn wants_vector(self) -> bool {
        self.vector || self.contents
    }
}

impl Default for BuildTargets {
    fn default() -> Self {
        BuildTargets::RASTER
    }
}
