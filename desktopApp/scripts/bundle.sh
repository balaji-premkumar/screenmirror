#!/bin/bash
set -e

# Bundle Native Libraries for Mirror Core

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
TARGET_DIR="$( cd "$SCRIPT_DIR/.." && pwd )/bin"
mkdir -p "$TARGET_DIR"

PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')

echo "Bundling native libraries for $PLATFORM..."

if [ -f "obs_plugin/build/mirror-source.so" ]; then
    echo "Bundling pre-built OBS plugin..."
    cp obs_plugin/build/mirror-source.so "$TARGET_DIR/"
fi

echo "Bundling Rust backend library..."
if [ "$PLATFORM" == "linux" ]; then
    cp mirror_backend/target/release/libmirror_backend.so "$TARGET_DIR/" 2>/dev/null || echo "Backend lib not found yet, skipping"
elif [ "$PLATFORM" == "darwin" ]; then
    cp mirror_backend/target/release/libmirror_backend.dylib "$TARGET_DIR/" 2>/dev/null || echo "Backend lib not found yet, skipping"
else
    cp mirror_backend/target/release/mirror_backend.dll "$TARGET_DIR/" 2>/dev/null || echo "Backend lib not found yet, skipping"
fi

echo "Libraries bundled successfully in $TARGET_DIR"
