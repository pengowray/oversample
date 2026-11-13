#!/usr/bin/env bash
# Build for GitHub Pages (serves from /oversample/)
# Strip function names from the wasm, as the deploy build in release.yml does.
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="-C link-arg=--strip-all -C link-arg=--keep-section=target_features" \
  trunk build --release --public-url /oversample/ --dist docs
