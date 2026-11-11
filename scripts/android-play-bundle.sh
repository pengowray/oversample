#!/usr/bin/env bash
# Build a signed Android App Bundle (.aab) for Google Play, with all four ABIs.
#
# Usage: scripts/android-play-bundle.sh [commit]    (default: HEAD)
#
# Builds from a temporary git worktree of the commit, so uncommitted changes in
# this checkout are not included and a running `trunk serve` here can't
# interfere with the build. The Rust target dir is kept between runs in
# $OVERSAMPLE_PLAY_BUILD_DIR (default ~/.cache/oversample-play-build) to make
# later builds faster.
#
# Output goes to target/play/ in this checkout:
#   oversample-<version>-<versionCode>-<commit>.aab          upload this to Play
#   oversample-<version>-<versionCode>-<commit>-mapping.txt  R8 mapping for Play's crash reports
#
# For a quick arm64-only APK for local testing, use scripts/copyup.sh.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck disable=SC1090
source ~/.profile

rev="${1:-HEAD}"
commit="$(git -C "$repo" rev-parse --verify "$rev^{commit}")"
short="$(git -C "$repo" rev-parse --short "$commit")"

android_dir="$repo/src-tauri/gen/android"
keystore_props="$android_dir/keystore.properties"
if [[ ! -f "$keystore_props" ]]; then
  echo "Missing $keystore_props (release signing details)." >&2
  exit 1
fi
store_file="$(sed -n 's/^storeFile=//p' "$keystore_props")"
if [[ ! -f "$android_dir/$store_file" ]]; then
  echo "Missing keystore $android_dir/$store_file." >&2
  exit 1
fi

if [[ -n "$(git -C "$repo" status --porcelain --untracked-files=no)" ]]; then
  echo "Note: this checkout has uncommitted changes. They are not in the bundle,"
  echo "which is built from commit $short."
fi

build_root="${OVERSAMPLE_PLAY_BUILD_DIR:-$HOME/.cache/oversample-play-build}"
worktree="$build_root/src"
export CARGO_TARGET_DIR="$build_root/target"
mkdir -p "$build_root"

remove_worktree() {
  git -C "$repo" worktree remove --force "$worktree" 2>/dev/null || rm -rf "$worktree"
  git -C "$repo" worktree prune
}
remove_worktree
trap remove_worktree EXIT

git -C "$repo" worktree add --detach "$worktree" "$commit"
cp "$keystore_props" "$android_dir/$store_file" "$worktree/src-tauri/gen/android/"

cd "$worktree"
cargo tauri android build --aab --ci

version="$(sed -n 's/^  "version": "\(.*\)",$/\1/p' src-tauri/tauri.conf.json | head -1)"
version_code="$(sed -n 's/^tauri.android.versionCode=//p' src-tauri/gen/android/app/tauri.properties)"
outputs="src-tauri/gen/android/app/build/outputs"

out_dir="$repo/target/play"
mkdir -p "$out_dir"
name="oversample-$version-$version_code-$short"
cp "$outputs/bundle/universalRelease/app-universal-release.aab" "$out_dir/$name.aab"
cp "$outputs/mapping/universalRelease/mapping.txt" "$out_dir/$name-mapping.txt"

echo
echo "Commit:       $short"
echo "versionName:  $version"
echo "versionCode:  $version_code  (Play needs this higher than any earlier upload)"
echo "Native libs:"
unzip -l "$out_dir/$name.aab" | grep -o 'base/lib/.*\.so' | sed 's/^/  /'
echo "Signed by:"
keytool -printcert -jarfile "$out_dir/$name.aab" | grep -m2 -E 'Owner:|SHA256:' | sed 's/^/  /'
echo
echo "Bundle:  $out_dir/$name.aab"
echo "Mapping: $out_dir/$name-mapping.txt"
