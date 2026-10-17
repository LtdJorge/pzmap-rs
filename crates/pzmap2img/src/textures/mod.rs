//! Loading sprites from the game's texture packs.
//!
//! A texture pack (`.pack` file) holds pages. A page is one PNG image with many sprites packed
//! into it, plus a table giving each sprite's name and position. Each tile name in a map is the
//! name of the sprite that draws it.
//!
//! The build reads the packs' tables, decodes only the pages that hold sprites the map uses,
//! and copies those sprites out. It also combines vegetation sprites as [`PlantsConfig`]
//! describes. The result is a [`TextureLibrary`], which looks up a sprite by tile id.

mod build;
#[cfg(test)]
mod game_tests;
mod library;
mod plants;

pub use build::TextureStats;
pub(crate) use build::{
    LoadedPack,
    build_texture_library,
};
pub use library::{
    FLAG_EMPTY,
    FLAG_OPAQUE,
    Sprite,
    TexMeta,
    TextureLibrary,
};
pub use plants::{
    PlantsConfig,
    Season,
};
