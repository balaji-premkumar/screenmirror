# Builds the Mirror Stream (USB) OBS source plugin on Linux, Windows and macOS.
#
# Linux:
#   cmake -B build && cmake --build build
#   (needs libobs-dev; falls back to -I/usr/include/obs)
#
# Windows (x64, from a VS developer prompt):
#   cmake -B build -DCMAKE_PREFIX_PATH=C:/path/to/obs-studio-deps ^
#         -DLIBOBS_INCLUDE_DIR=C:/path/to/obs-studio/libobs ^
#         -DLIBOBS_LIB=C:/path/to/obs-studio/build/libobs/RelWithDebInfo/obs.lib
#   cmake --build build --config Release
#   Copy build/Release/mirror-source.dll into desktopApp/bin/ so the app's
#   "Install OBS plugin" button can deploy it.
#   LIBOBS_INCLUDE_DIR and LIBOBS_LIB may be lists: w32-pthreads' headers and
#   import library are needed alongside libobs's.
#
# macOS (headers only; nothing is linked, OBS supplies libobs when it loads
# the plugin):
#   cmake -B build -DLIBOBS_INCLUDE_DIR="/path/to/obs-studio/libobs;$(brew --prefix)/include" \
#         -DCMAKE_OSX_ARCHITECTURES="arm64;x86_64"
#   cmake --build build
#
# .github/actions/build-obs-plugin does all three the way CI does.

cmake_minimum_required(VERSION 3.16)
project(mirror-source C)

set(CMAKE_C_STANDARD 11)

add_library(mirror-source MODULE mirror_source.c)
set_target_properties(mirror-source PROPERTIES PREFIX "")

# The plugin's own directory, for the vendored `simde/x86/sse2.h`. libobs
# headers include it, and distro libobs-dev packages do not always ship simde
# alongside them. The `gcc` line this file replaced passed `-I.` for exactly
# this reason; leaving it out here made the CMake build fail on any machine
# where the header was not already on the system include path.
# Not on macOS: the stub includes <emmintrin.h>, which arm64 lacks, so the
# real SIMDe (Homebrew's) is passed in LIBOBS_INCLUDE_DIR instead.
if(NOT APPLE)
    target_include_directories(mirror-source PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
endif()

# Prefer an installed obs package (obs-studio dev packages export libobs)
find_package(libobs QUIET)

if(TARGET OBS::libobs)
    target_link_libraries(mirror-source PRIVATE OBS::libobs)
elseif(APPLE AND DEFINED LIBOBS_INCLUDE_DIR)
    target_include_directories(mirror-source PRIVATE ${LIBOBS_INCLUDE_DIR})
    target_link_options(mirror-source PRIVATE -undefined dynamic_lookup)
elseif(DEFINED LIBOBS_INCLUDE_DIR AND DEFINED LIBOBS_LIB)
    target_include_directories(mirror-source PRIVATE ${LIBOBS_INCLUDE_DIR})
    target_link_libraries(mirror-source PRIVATE ${LIBOBS_LIB})
else()
    # Distro default (Debian/Ubuntu: libobs-dev)
    target_include_directories(mirror-source PRIVATE /usr/include/obs)
    target_link_libraries(mirror-source PRIVATE obs)
endif()

if(WIN32)
    # OpenFileMapping/MapViewOfFile live in kernel32 (linked by default);
    # w32-pthreads comes in through libobs.
elseif(APPLE)
    # pthread is part of libSystem; there is no librt.
else()
    target_link_libraries(mirror-source PRIVATE rt pthread)
endif()
