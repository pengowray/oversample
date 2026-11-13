#!/usr/bin/env bash
# Build for GitHub Pages (serves from /oversample/)
bash "$(dirname "$0")/scripts/trunk-release.sh" --public-url /oversample/ --dist docs
