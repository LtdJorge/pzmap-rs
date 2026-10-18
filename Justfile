# Commands for working on pzmap-rs. Run `just` to list them.
#
# The release commands check each step before taking it and print the next one. CONTRIBUTING.md
# describes the release process.

set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# List the commands.
default:
    @just --list --unsorted

# Run everything that CI's lint and test jobs check.
check: fmt-check clippy test doc notices-check deny

# Format the code. rustfmt.toml uses options that only nightly rustfmt applies.
fmt:
    cargo +nightly fmt --all

# Fail if any code is not formatted.
fmt-check:
    cargo +nightly fmt --all --check

# Run clippy on every feature set that CI checks. Keep this in step with ci.yml.
clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo clippy -p pzmap2img --no-default-features --all-targets --locked -- -D warnings
    cargo clippy -p pzmap2img --no-default-features --features cli --all-targets --locked -- -D warnings
    cargo clippy -p pzmap-tools --no-default-features --features tui --all-targets --locked -- -D warnings
    cargo clippy -p pzmap-tools --no-default-features --features gui --all-targets --locked -- -D warnings

# Run the tests that need no game files.
test:
    cargo test --workspace --locked

# Build the docs, failing on any warning.
[env("RUSTDOCFLAGS", "-D warnings")]
doc:
    cargo doc --workspace --no-deps --locked

# Regenerate the THIRD-PARTY-NOTICES.md files. Needs cargo-about.
notices:
    cargo run -q -p xtask --locked -- notices

# Fail if a THIRD-PARTY-NOTICES.md file lacks a crate or license its binary has. Needs cargo-about.
notices-check:
    cargo run -q -p xtask --locked -- notices --check

# Check the dependencies for advisories and license problems. Needs cargo-deny.
deny:
    cargo deny check

# Show where each crate's release stands, and the next step.
release-status crate="":
    cargo run -q -p xtask --locked -- release status {{ crate }}

# Bump a crate to a new version on a release branch, date its changelog section, and run every check.
release-prepare crate version:
    cargo run -q -p xtask --locked -- release prepare {{ crate }} {{ version }}
    just check
    @echo ""
    @echo "Every check passed. Next:"
    @echo "  1. Review the changes with git diff, and commit them as 'Release {{ crate }} v{{ version }}'."
    @echo "  2. Push the branch with git push -u origin HEAD, and open a pull request with gh pr create --fill."
    @echo "  3. Once CI passes, merge the pull request, then run git switch master and git pull."
    @echo "  4. Wait for CI to pass on master, then run just release-tag {{ crate }}."

# Tag a crate's version and push the tag, which publishes it. Asks before pushing.
release-tag crate:
    cargo run -q -p xtask --locked -- release tag {{ crate }}

# Publish a crate to crates.io for the first time, by hand. Asks before publishing.
release-first crate:
    cargo run -q -p xtask --locked -- release first {{ crate }}
