# Changelog

All notable changes to pzmap2img are listed in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and pzmap2img
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-17

The first public release.

### Added

- A library that builds a map from the game's files and renders each layer to a Deep Zoom image
  pyramid, matching pzmap2dzi's output.
- The `pzmap2img` command-line tool, with the `build`, `plan` and `render` commands.
- WebP and PNG tiles, lossless or lossy.
- Vegetation options: season, snow, flowers, and tree sizes.
- Memory limits for both phases and a free disk space check before rendering.

[Unreleased]: https://github.com/LtdJorge/pzmap-rs/compare/pzmap2img-v0.1.0...master
[0.1.0]: https://github.com/LtdJorge/pzmap-rs/releases/tag/pzmap2img-v0.1.0
