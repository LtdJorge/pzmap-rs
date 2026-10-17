# Changelog

All notable changes to libpzmap are listed in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and libpzmap
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-17

The first public release.

### Added

- Parsers for a map's cells: `.lotheader` files and `world_*.lotpack` files.
- A parser for `.pack` texture packs.
- Parsers for the map's other files: zones in `objects.lua` and `regions.lua`, `streets.xml`,
  world map labels, `newtiledefinitions.tiles`, translations, `map.info`, and Lua data files.
- Support for Build 41 and Build 42 files, chosen at compile time or at runtime.
- Typed errors with the file format and byte offset for every malformed file, instead of panics.

[Unreleased]: https://github.com/LtdJorge/pzmap-rs/compare/v0.1.0...master
[0.1.0]: https://github.com/LtdJorge/pzmap-rs/releases/tag/v0.1.0
