#!/usr/bin/env bash
# Build and run the Oversample desktop app (Tauri) on Linux.
#
# Usage:
#   scripts/desktop-run.sh [--release | --dev | --no-build] [app args...]
#
#   (no option)  debug build without bundling, then run it (recommended)
#   --release    release build without bundling, then run it
#   --dev        cargo tauri dev
#   --no-build   run the last built binary (debug, or release with --release)
#   --help       show this text
#   Any other arguments are passed to the app.
#
# Which frontend build is used (from src-tauri/tauri.conf.json):
#   - Builds (default and --release) run beforeBuildCommand,
#     `bash scripts/trunk-release.sh`, a release WASM build. Only the native
#     side is debug in the default mode.
#   - --dev runs beforeDevCommand, `trunk serve --port 21259`, a debug WASM
#     build, which is very slow here. Use the default mode to try things.
#     --dev fails if something else (such as another trunk serve) already
#     uses port 21259.
#
# Google sign-in: src-tauri/build.rs reads the desktop client id and secret
# (OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_ID / _SECRET) from the repo's git-ignored
# .env at build time. Nothing to export by hand.
#
# Needs: cargo install tauri-cli --version "^2"

set -euo pipefail

usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }

cd "$(dirname "$(readlink -f "$0")")/.."

mode=debug
build=1
app_args=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) mode=release ;;
        --dev) mode=dev ;;
        --no-build) build=0 ;;
        -h|--help) usage; exit 0 ;;
        *) app_args+=("$1") ;;
    esac
    shift
done

need_tauri() {
    if ! cargo tauri --version >/dev/null 2>&1; then
        echo "cargo tauri is not installed. Install it with:" >&2
        echo '  cargo install tauri-cli --version "^2"' >&2
        exit 1
    fi
}

if [[ $mode == dev ]]; then
    need_tauri
    if [[ ${#app_args[@]} -gt 0 ]]; then
        exec cargo tauri dev -- -- "${app_args[@]}"
    fi
    exec cargo tauri dev
fi

bin="target/$mode/oversample-desktop"
if [[ $build == 1 ]]; then
    need_tauri
    if [[ $mode == release ]]; then
        cargo tauri build --no-bundle
    else
        cargo tauri build --debug --no-bundle
    fi
fi

if [[ ! -x $bin ]]; then
    echo "No built app at $bin. Run without --no-build first." >&2
    exit 1
fi
exec "$bin" "${app_args[@]}"
