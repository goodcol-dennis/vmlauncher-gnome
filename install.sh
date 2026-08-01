#!/bin/bash
set -euo pipefail

echo "Building vmlaunch..."
cargo build --release

echo "Installing binary..."
install -Dm755 target/release/vmlaunch ~/.local/bin/vmlaunch

echo "Installing icon..."
install -Dm644 assets/vmlaunch.svg ~/.local/share/icons/hicolor/scalable/apps/vmlaunch.svg

echo "Installing desktop entry..."
install -Dm644 tech.goodcol.vmlaunch.desktop ~/.local/share/applications/tech.goodcol.vmlaunch.desktop

echo "Updating caches..."
gtk-update-icon-cache -f ~/.local/share/icons/hicolor/ 2>/dev/null || true
update-desktop-database ~/.local/share/applications/ 2>/dev/null || true

echo "Done. Launch 'vmlaunch' from your app menu or run ~/.local/bin/vmlaunch"
