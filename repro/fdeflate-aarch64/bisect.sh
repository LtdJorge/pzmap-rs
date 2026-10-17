#!/bin/sh
# The test that cargo-bisect-rustc runs for each compiler, from the repository root. It sets
# RUSTUP_TOOLCHAIN to the compiler being tested.
#
# Builds pzmap2img's `decode_tile` doctest with overflow checks on in fdeflate, which fails on
# Apple silicon with Rust 1.99 and passes with Rust 1.95. Exits 1 only for that failure, so a
# compiler that fails for another reason does not count as the regression.

out=$(cargo test -p pzmap2img --doc --locked --ignore-rust-version \
    --config 'profile.test.package.fdeflate.overflow-checks=true' -- decode_tile 2>&1)
status=$?
printf '%s\n' "$out" | tail -n 15

if printf '%s\n' "$out" | grep -q CorruptFlateStream; then
    echo "bisect: $RUSTUP_TOOLCHAIN has the regression"
    exit 1
fi
if [ "$status" -ne 0 ]; then
    echo "bisect: $RUSTUP_TOOLCHAIN failed for another reason, counted as without the regression"
fi
exit 0
