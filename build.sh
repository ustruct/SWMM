#!/bin/sh
set -e
cd "$(dirname "$0")"
cargo build --release --workspace
arch="$(uname -m)"
case "$arch" in
    arm64) arch=aarch64 ;;
esac
mkdir -p dist stage
cp target/release/swmm LICENSE stage/
cp assets/icon.png stage/swmm.png
tar -cJf "dist/swmm-$arch-linux.tar.xz" -C stage .
rm -rf stage
echo "Done: $(pwd)/dist/swmm-$arch-linux.tar.xz"
