#!/usr/bin/env bash
# `trunk build --release` with function names stripped from the wasm.
# Extra arguments go to trunk, e.g. scripts/trunk-release.sh --dist docs
# Runs from the repo root, so relative paths in them are from there.
#
# Used by .github/workflows/release.yml, gh-release.sh and Tauri's
# beforeBuildCommand (desktop and Android bundles). `trunk serve --release`
# for local work keeps the names.
#
# --strip-all drops the wasm `name` section: 48 MB of the app's 60 MB wasm and
# 6.6 MB of the BirdNET worker's 23 MB. Panic messages keep their file:line,
# but stack frames show as wasm-function[N]. To find the function behind N,
# build the same commit with plain `trunk build --release`: it has the same
# functions in the same order, with names.
#
# --strip-all also drops `target_features`, which wasm-bindgen reads to turn on
# externref and multivalue, so --keep-section keeps it.
#
# The flags go in CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS, which Cargo
# adds to the flags in .cargo/config.toml (simd128); RUSTFLAGS would replace
# them. CARGO_PROFILE_RELEASE_STRIP would not do: Cargo ignores the base
# profile's strip for birdnet-worker, which has its own profile override, and
# a profile change also changes the compiled code.
set -euo pipefail

strip_flags="-C link-arg=--strip-all -C link-arg=--keep-section=target_features"
var=CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS
export "$var=${!var:+${!var} }$strip_flags"

cd "$(dirname "$0")/.."
exec trunk build --release "$@"
