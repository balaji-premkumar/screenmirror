#!/bin/bash
# ─── Mirror OBS Plugin — Build & Install Script ─────────────
# Compiles the shared-memory OBS source plugin and installs it
# into the user's OBS Studio plugin directory.
#
# Requirements:
#   - gcc
#   - libobs-dev (or OBS headers at /usr/include/obs)
#
# Usage:
#   chmod +x install_plugin.sh && ./install_plugin.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE="$SCRIPT_DIR/mirror_source.c"
BUILD_DIR="$SCRIPT_DIR/build"
OUTPUT="$BUILD_DIR/mirror-source.so"

echo "╔════════════════════════════════════════════════╗"
echo "║   Mirror Stream — OBS Plugin Installer         ║"
echo "╚════════════════════════════════════════════════╝"
echo ""

# ── 1. Check for gcc ──────────────────────────────────────
if ! command -v gcc &> /dev/null; then
    echo "✗ gcc not found. Install with: sudo apt install build-essential"
    exit 1
fi
echo "✓ gcc found"

# ── 2. Check for OBS headers ─────────────────────────────
HAS_LIBOBS=false
if pkg-config --exists libobs 2>/dev/null; then
    HAS_LIBOBS=true
    echo "✓ libobs-dev found via pkg-config"
elif [ -f /usr/include/obs/obs-module.h ]; then
    echo "✓ OBS headers found at /usr/include/obs"
else
    echo "✗ OBS development headers not found."
    echo "  Install with: sudo apt install libobs-dev"
    echo "  (or install OBS Studio from source)"
    exit 1
fi

echo ""
echo "→ Checking for pre-compiled plugin..."

if [ ! -f "$OUTPUT" ]; then
    echo "✗ Pre-compiled plugin not found at $OUTPUT."
    echo "  Please compile or copy mirror-source.so to build/ first."
    exit 1
fi

echo "✓ Found: $OUTPUT"

# ── 4. Find OBS plugin directory ──────────────────────────
PLUGIN_DIR=""

# Flatpak OBS
FLATPAK_DIR="$HOME/.var/app/com.obsproject.Studio/config/obs-studio/plugins"
# Native OBS (modern)
CONFIG_DIR="$HOME/.config/obs-studio/plugins"
# Legacy OBS
LEGACY_DIR="$HOME/.obs-studio/plugins"

if [ -d "$(dirname "$FLATPAK_DIR")" ]; then
    PLUGIN_DIR="$FLATPAK_DIR"
    echo "✓ Detected Flatpak OBS install"
elif [ -d "$(dirname "$CONFIG_DIR")" ]; then
    PLUGIN_DIR="$CONFIG_DIR"
    echo "✓ Detected native OBS install (modern)"
elif [ -d "$(dirname "$LEGACY_DIR")" ]; then
    PLUGIN_DIR="$LEGACY_DIR"
    echo "✓ Detected native OBS install (legacy)"
else
    # Default to modern path
    PLUGIN_DIR="$CONFIG_DIR"
    echo "→ OBS config directory not found, using default: $PLUGIN_DIR"
fi

# ── 5. Install ─────────────────────────────────────────────
INSTALL_DIR="$PLUGIN_DIR/mirror-source/bin/64bit"
mkdir -p "$INSTALL_DIR"

cp "$OUTPUT" "$INSTALL_DIR/mirror-source.so"

echo ""
echo "════════════════════════════════════════════════"
echo "  ✓ Plugin installed successfully!"
echo ""
echo "  Location: $INSTALL_DIR/mirror-source.so"
echo ""
echo "  Next steps:"
echo "  1. (Re)start OBS Studio"
echo "  2. Add Source → 'Mirror Stream (USB)'"
echo "  3. Start streaming in the Mirror desktop app"
echo "  4. Enable 'Direct to OBS' toggle"
echo "════════════════════════════════════════════════"
