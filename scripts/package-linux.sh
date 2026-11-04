#!/bin/sh
# Collects the Linux packages into ./linux-packages. Run from the repository root after
# `pnpm tauri build --bundles deb,appimage`. On Linux the product is called "Reclaim Cleaner", so
# that the deb package gets the name reclaim-cleaner; the files keep the name of the other
# downloads. The Windows release job adds these to the release and writes the checksums.
set -eu
out=linux-packages
rm -rf "$out"
mkdir -p "$out"
for file in target/release/bundle/deb/*.deb target/release/bundle/appimage/*.AppImage; do
  cp "$file" "$out/$(basename "$file" | sed 's/^Reclaim Cleaner/ReClaim-Cleaner/')"
done
ls -l "$out"
