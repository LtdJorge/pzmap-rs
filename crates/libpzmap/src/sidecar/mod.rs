//! Readers for the small files that aren't `.lotheader` or `.lotpack`.
//!
//! A map is stored as cells: square areas of the map, each in a `.lotheader` file and a
//! `world_*.lotpack` file. Next to them, the map folder under `media/maps` holds smaller files
//! that describe the map as a whole. A few related files live elsewhere in the game's `media`
//! folder. The modules are:
//!
//! - [`zones`] reads `objects.lua`, `regions.lua` and `roomtones.lua`, which mark areas of the
//!   map such as forests, parking stalls, and towns.
//! - [`streets`] reads `streets.xml`, the named streets.
//! - [`annotations`] reads `worldmap-annotations.lua`, the text labels on the world map.
//! - [`map_info`] reads `map.info`, a map folder's title and the folders for its cells.
//! - [`tiledefs`] reads `media/newtiledefinitions.tiles`, the properties of every sprite.
//! - [`translation`] reads the game's translation files, which hold the label texts.
//! - [`markup`] splits the rich-text markup in translated texts such as town descriptions.
//! - [`lua`] reads Lua files that hold a single table of data. The other readers use it.
//!
//! The Lua files are treated as data, never run as programs. [`lua`] reads table literals and
//! [`annotations`] looks for known calls among the tokens. This makes it safe to read maps from
//! untrusted sources.
//!
//!
//! ## Warning
//! This module is written by AI and has had almost no review yet, apart from testing that it works
//! on every file supported.

pub mod annotations;
pub mod lua;
pub mod map_info;
pub mod markup;
pub mod streets;
pub mod tiledefs;
pub mod translation;
pub mod zones;
