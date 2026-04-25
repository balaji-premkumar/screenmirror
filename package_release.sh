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

# 3. Build OBS Plugin
echo "Building OBS Plugin..."
cd desktopApp/obs_plugin
mkdir -p build
gcc -shared -fPIC -o build/mirror-source.so mirror_source.c -I/usr/include/obs -lobs -lrt
cd ../..

# 4. Bundle everything
echo "Bundling artifacts..."
cp mobileApp/build/app/outputs/flutter-apk/app-release.apk "$RELEASE_DIR/mobile/mirror-companion.apk"
cp desktopApp/obs_plugin/build/mirror-source.so "$RELEASE_DIR/obs_plugin/"

# Copy desktop binaries (platform specific)
if [ "$PLATFORM" == "linux" ]; then
    cp desktopApp/mirror_backend/target/release/libmirror_backend.so "$RELEASE_DIR/desktop/"
    # Copy Electrobun app if built
elif [ "$PLATFORM" == "darwin" ]; then
    cp desktopApp/mirror_backend/target/release/libmirror_backend.dylib "$RELEASE_DIR/desktop/"
fi

echo "Release v$VERSION created successfully in $RELEASE_DIR"
