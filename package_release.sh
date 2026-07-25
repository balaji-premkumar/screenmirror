#!/bin/bash
set -e

# Mirror Core Release Packager
# This script bundles the Desktop App, Mobile App, and OBS Plugin into a single release folder.

VERSION="1.0.0"
RELEASE_DIR="releases/v$VERSION"
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')

echo "Creating release directory: $RELEASE_DIR"
mkdir -p "$RELEASE_DIR/desktop"
mkdir -p "$RELEASE_DIR/mobile"
mkdir -p "$RELEASE_DIR/obs_plugin"

# 1. Build Desktop
echo "Building Desktop App..."
cd desktopApp
bun install
bun run build:all
# npx electrobun build  # Uncomment if electrobun build is configured for local packaging
cd ..

# 2. Build Mobile
echo "Building Mobile App..."
cd mobileApp
flutter build apk --release
cd ..

# 3. Build OBS Plugin (CMake handles Linux/macOS/Windows)
echo "Building OBS Plugin..."
cd desktopApp/obs_plugin
cmake -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release
cd ../..

# 4. Bundle everything
echo "Bundling artifacts..."
cp mobileApp/build/app/outputs/flutter-apk/app-release.apk "$RELEASE_DIR/mobile/mirror-companion.apk"

# The desktop app installs the plugin from desktopApp/bin/, so stage it there too.
mkdir -p desktopApp/bin
PLUGIN_BIN=$(find desktopApp/obs_plugin/build -maxdepth 2 -name 'mirror-source.*' \
    \( -name '*.so' -o -name '*.dll' -o -name '*.dylib' \) | head -1)
if [ -n "$PLUGIN_BIN" ]; then
    cp "$PLUGIN_BIN" "$RELEASE_DIR/obs_plugin/"
    cp "$PLUGIN_BIN" desktopApp/bin/
else
    echo "WARNING: OBS plugin binary not found — skipping plugin bundling"
fi

# Copy desktop binaries (platform specific)
if [ "$PLATFORM" == "linux" ]; then
    cp desktopApp/mirror_backend/target/release/libmirror_backend.so "$RELEASE_DIR/desktop/"
    # Copy Electrobun app if built
elif [ "$PLATFORM" == "darwin" ]; then
    cp desktopApp/mirror_backend/target/release/libmirror_backend.dylib "$RELEASE_DIR/desktop/"
fi

echo "Release v$VERSION created successfully in $RELEASE_DIR"
