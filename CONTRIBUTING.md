# Contributing

Thank you for helping. This file explains how to build and test the project and what a PR
will need before getting merged.

## Building and testing the Rust crates

Stable Rust is enough. `rust-toolchain.toml` automatically selects it if you use rustup.

```sh
cargo build
cargo test
```

To build or test one crate, add `-p` (package) and its name, such as `cargo test -p libpzmap`.

## Building and testing the Java bindings

The Java bindings need Java 17 or newer. Run the Gradle wrapper from `crates/pzmap-java`. It
downloads the right Gradle version on first use, so you don't need to install Gradle. On Windows,
use `gradlew.bat`.

```sh
./gradlew build
./gradlew test
```

Gradle compiles the native library with cargo and puts it inside the jar. The
[pzmap-java README](crates/pzmap-java/README.md) has the details.

## Building and testing the Python bindings

The Python bindings build with [maturin](https://www.maturin.rs/). From `crates/pzmap-python`, in
a virtual environment:

```sh
pip install maturin
maturin develop --extras test
pytest
```

`maturin develop` compiles the extension and installs it into the virtual environment. The
`test` extra installs pytest.

## Formatting and lints

Format with nightly rustfmt. `rustfmt.toml` uses options that stable rustfmt ignores, so
`cargo fmt` on stable leaves the imports unformatted.

```sh
cargo +nightly fmt
```

Run clippy on every target, including tests and examples, and fix every warning:

```sh
cargo clippy --all-targets
```

## Third-party notices

Each crate that ships a binary has a `THIRD-PARTY-NOTICES.md` file listing the licenses of the
code compiled into it. When you add, remove or update a dependency, regenerate these files. The
task needs cargo-about, which you can install with `cargo install cargo-about`.

```sh
cargo run -p xtask -- notices
```

Add `--check` to only check that the files are up to date.

## Panics

The parsers read files from anyone, such as maps and mods downloaded from the internet. A bad
file must never crash the program.

- Malformed input returns a typed error, never a panic.
- Assertions are the only allowed panics. They check the code's own invariants, so a failed
  assertion points to a bug in this project, not to a bad file.
- Assertions must stay on in release builds.

The workspace lints enforce this. They deny `unwrap`, `expect`, `panic!`, slice indexing and
casts that can truncate or change sign. Test modules may opt out with a module-level `#![allow]`.

## Tests that need the game

Tests that read real game files are marked `#[ignore]`, so `cargo test` passes without the game.
To run them, set `PZ_INSTALL_DIR` to the `projectzomboid` folder of a Build 42 install, the one
that contains `media`:

```sh
PZ_INSTALL_DIR=/path/to/projectzomboid cargo test -- --include-ignored
```

One libpzmap test also needs `PZ_B41_INSTALL_DIR`, set the same way for a Build 41 install. The
Java and Python tests read `PZ_INSTALL_DIR` too, and skip the game tests when it is not set.

Game tests should check facts that hold for any Build 42 install, not counts from one version of
the game.

Never commit game files, or text copied from them, to this repository. That includes small
excerpts used in tests. Build test inputs by hand in the test, or read them from
`PZ_INSTALL_DIR` at test time.

## Adding a parser for a new file format

1. Put the parser in libpzmap. Files that sit next to a map's cells (mainly the .lua files) go in the `sidecar` module.
2. Take the file's contents as `&[u8]` and borrow from it instead of copying where you can.
3. Add a variant to `error::Format`, so errors name the new format.
4. Return `ParseError` for every malformed input, with the byte offset where reading failed.
   Check every count read from the file against the bytes that remain before you allocate.
5. Add unit tests with small inputs built in the test, including truncated and malformed ones.
   Add an `#[ignore]` test that parses the real files from `PZ_INSTALL_DIR`.
6. Document the format in the module's docs, and update the crate's README and the changelog.

## Proposing changes

For a bug or a feature idea, open an issue first, so we can agree on the approach before you
write the code. Small fixes can go straight to a pull request.

A pull request should:

- pass `cargo +nightly fmt --check`, `cargo clippy --all-targets` and `cargo test`;
- add tests for the new behavior;
- add a line to the `[Unreleased]` section of [CHANGELOG.md](CHANGELOG.md) when users would
  notice the change.

Report security problems privately, as described in [SECURITY.md](SECURITY.md), not in a public
issue.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in
the work, as defined in the Apache-2.0 license, shall be dual licensed under MIT OR Apache-2.0,
without any additional terms or conditions.
