//! The currently supported builds, [`B41`] and [`B42`].
//!
//! Both are uninhabited types, only to be used as type parameters.

use crate::build::{
    VersionMetadata,
    sealed::{
        self,
        LayerEncoding,
    },
};
use std::fmt::{
    Display,
    Formatter,
};

/// Project Zomboid Build 41.
///
/// Cells are 30×30 blocks of 10×10 squares, with layers 0 to 7.
#[derive(Copy, Clone, Debug)]
pub enum B41 {}

impl Display for B41 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(Self::NAME)
    }
}

impl sealed::Sealed for B41 {
    const NULL_AFTER_STRING_TABLE: bool = true;
    const LAYER_ENCODING: LayerEncoding = LayerEncoding::MaxOnly;
}

impl VersionMetadata for B41 {
    const VERSION_NUMBER: u32 = 0;
    const CELL_SIZE_IN_BLOCKS: usize = 30;
    const BLOCK_SIZE_IN_SQUARES: usize = 10;
    const CELL_SIZE_IN_SQUARES: i32 = 300;
    const MIN_LAYER: i32 = 0;
    const MAX_LAYER: i32 = 8;
    const NAME: &'static str = "Build 41";
}

/// Project Zomboid Build 42.
///
/// Cells are 32×32 blocks of 8×8 squares, with layers -32 to 31. Negative layers are basements.
#[derive(Copy, Clone, Debug)]
pub enum B42 {}

impl Display for B42 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(Self::NAME)
    }
}

impl sealed::Sealed for B42 {
    const NULL_AFTER_STRING_TABLE: bool = false;
    const LAYER_ENCODING: LayerEncoding = LayerEncoding::MinAndMax;
}

impl VersionMetadata for B42 {
    const VERSION_NUMBER: u32 = 1;
    const CELL_SIZE_IN_BLOCKS: usize = 32;
    const BLOCK_SIZE_IN_SQUARES: usize = 8;
    const CELL_SIZE_IN_SQUARES: i32 = 256;
    const MIN_LAYER: i32 = -32;
    const MAX_LAYER: i32 = 32;
    const NAME: &'static str = "Build 42";
}
