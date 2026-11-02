#!/bin/sh
set -eu
repo="__REPO__"
tag="__TAG__"
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) asset="swmm-x86_64-linux.tar.xz" ;;
    Linux-aarch64 | Linux-arm64) asset="swmm-aarch64-linux.tar.xz" ;;
    *) echo "SWMM releases are built for Windows and Linux (x86-64 and ARM64). Build it from source on this system." >&2; exit 1 ;;
esac
url="https://github.com/$repo/releases/download/$tag/$asset"
install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/swmm"
bin_dir="$HOME/.local/bin"
apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"

if command -v curl >/dev/null 2>&1; then
    fetch() { curl --proto '=https' --tlsv1.2 -LsSf "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    echo "Install curl or wget and run this again." >&2
    exit 1
fi

temp="$(mktemp -d)"
trap 'rm -rf "$temp"' EXIT
echo "Downloading SWMM $tag..."
fetch "$url" "$temp/$asset"
mkdir -p "$install_dir" "$bin_dir" "$apps_dir"
tar -xJf "$temp/$asset" -C "$install_dir"
chmod +x "$install_dir/swmm"
ln -sf "$install_dir/swmm" "$bin_dir/swmm"

cat > "$apps_dir/swmm.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=SWMM
Comment=Savage Wind Mod Manager
Exec=$install_dir/swmm
Icon=$install_dir/swmm.png
Terminal=false
Categories=Game;Utility;
DESKTOP

echo "SWMM $tag is installed in $install_dir"
echo "Start it from your app menu or run: swmm"
