#!/usr/bin/env bash
# `trunk build --release` with function names stripped from the wasm.
# Extra arguments go to trunk, e.g. scripts/trunk-release.sh --dist docs
# Runs from the repo root, so relative paths in them are from there.
#
# Used by .github/workflows/release.yml, gh-release.sh and Tauri's
# beforeBuildCommand (desktop and Android bundles). `trunk serve --release`
# for local work keeps the names.
#
# Everything here is to make the app's wasm smaller, because a browser or a
# phone's WebView compiles all of it before the app can start.
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

# wasm-bindgen adds a snippets folder per build and never removes old ones.
# Trunk copies every folder into dist and has the page preload every file in
# them, which was 382 extra requests at start-up after a few weeks of builds.
rm -rf "${CARGO_TARGET_DIR:-target}/wasm-bindgen/release/snippets"

# One codegen unit: the app's code comes out 13% smaller (10.30 MB to 8.91 MB),
# and the phone compiles all of it at every start. A full build takes about
# twice as long, so `trunk serve --release` keeps Cargo's default.
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1

# wasm-opt -Os on the app: a further 12% (8.91 MB to 7.87 MB of code). It is
# turned on in a copy of index.html so that `trunk serve --release` skips it:
# it adds time to each rebuild and drops the function names. The copy has to
# sit beside index.html, because Trunk finds the page's files from there. A
# `trunk serve` running in this folder will see it and rebuild.
page=index.release.html
trap 'rm -f "$page"' EXIT
sed 's/data-type="main" data-wasm-opt="0"/data-type="main" data-wasm-opt="s"/' index.html > "$page"
grep -q 'data-type="main" data-wasm-opt="s"' "$page" || {
    echo "trunk-release.sh: index.html has no data-type=\"main\" data-wasm-opt=\"0\" to turn on" >&2
    exit 1
}

trunk build --release "$@" "$page"
