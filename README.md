# pzmap-rs

Rust tools and libraries for reading Project Zomboid's map files. They parse the game
files for the maps, render the maps to images, and let you browse the game's texture packs. Bindings
make the parsers available from Java and Python too. Everything reads Build 41 and Build 42 files.

## Crates

| Crate                                         | What it is for                                                                                                                                                                                                   | Published as                        |
|-----------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------|
| [libpzmap](crates/libpzmap/README.md)         | Rust parsers for the map files: cells (`.lotheader` and `.lotpack`), texture packs, and the map's other files such as zones, streets and tile definitions.                                                       | `libpzmap` on crates.io             |
| [pzmap2img](crates/pzmap2img/README.md)       | Renders a map to Deep Zoom image pyramids, the same way as [pzmap2dzi](https://github.com/cff29546/pzmap2dzi), and it's inspired by it but with heavily improved performance. A command-line tool and a library. | `pzmap2img` on crates.io            |
| [pzmap-tools](crates/pzmap-tools/README.md)   | `pack-view` only for now, a visual browser for the sprites in texture packs, in a window or a terminal.                                                                                                          | `pzmap-tools` on crates.io          |
| [pzmap-java](crates/pzmap-java/README.md)     | Java bindings for libpzmap.                                                                                                                                                                                      | `io.github.ltdjorge:pzmap` on Maven |
| [pzmap-python](crates/pzmap-python/README.md) | Python bindings for libpzmap.                                                                                                                                                                                    | `pzmap` on PyPI                     |

## Which one do I want?

- To make an image or a zoomable web map of a map, use the `pzmap2img` command-line tool.
- To look at the sprites inside the game's `.pack` files, use `pack-view` from pzmap-tools. This is particularly useful
  for making mods, and it allows viewing your own `.pack` files.
- To read map files in your own Rust program, use `libpzmap`.
- To read map files from Java or Python, use `pzmap-java` or `pzmap-python`.
- To render maps from your own Rust program, use `pzmap2img` as a library.

## Building from source

Stable Rust is enough. The `rust-toolchain.toml` file selects it for you if you use rustup. The
workspace needs Rust 1.95 or later. `libpzmap` on its own needs only Rust 1.88.

```sh
cargo build --release
```

This builds every Rust crate in the workspace. The binaries end up in `target/release`.

The code is formatted with nightly rustfmt, because `rustfmt.toml` uses options that stable
rustfmt ignores:

```sh
cargo +nightly fmt
```

## Running tests

```sh
cargo test
```

Most tests need nothing but this repository. Tests that read real game files are marked
`#[ignore]`. To run them, set `PZ_INSTALL_DIR` to the `projectzomboid` folder of a Build 42
install, the one that contains `media`. With Steam on Linux, it is usually
`~/.local/share/Steam/steamapps/common/ProjectZomboid/projectzomboid`.

```sh
PZ_INSTALL_DIR=/path/to/projectzomboid cargo test -- --include-ignored
```

One libpzmap test reads Build 41 files and also needs `PZ_B41_INSTALL_DIR`, set the same way for
a Build 41 install. Without it, that test fails when ignored tests are included.

The Java bindings build and test with Gradle, as described in the
[pzmap-java README](crates/pzmap-java/README.md). The Python bindings build with maturin and test
with pytest, as described in [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and propose changes. To report a
security problem, see [SECURITY.md](SECURITY.md). Each crate has its own changelog, listed in
[CHANGELOG.md](CHANGELOG.md).

## Thanks

- The Indie Stone for creating one of the most amazing games ever developed
- Min Xiang's [pzmap2dzi](https://github.com/cff29546/pzmap2dzi/) for the invaluable help in decoding the game's files
- Blindcoder for the original PZ → DZI tooling

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT), at your option.

Some crates include work under other licenses:

- pzmap-tools bundles the Fira Code font, which is under the SIL Open Font License 1.1.
- Parts of pzmap2img follow pzmap2dzi, which is under the MIT license. Its
  [NOTICE](crates/pzmap2img/NOTICE) file gives the details.

The `THIRD-PARTY-NOTICES` files list the licenses of the dependencies compiled into each binary.
Run `cargo run -p xtask -- notices` to generate them, or add `--check` to check that they are
up to date.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
the work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

Project Zomboid is a trademark of The Indie Stone. This project is not affiliated with or endorsed
by The Indie Stone, and it does not include any game files.
