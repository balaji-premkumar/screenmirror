# Builds the Mirror Stream (USB) OBS source plugin on Linux, Windows and macOS.
#
#   cmake -S obs_plugin -B obs_plugin/build
#   cmake --build obs_plugin/build --config Release
#
# libobs is found per platform:
#   Linux:   libobs-dev (falls back to -I/usr/include/obs).
#   Windows: the installed OBS Studio. Headers matching its version are
#            downloaded and import libraries are generated from its DLLs.
#            Needs MSVC.
#   macOS:   the installed OBS.app. Headers matching its version are
#            downloaded and its libobs.framework is linked.
# The Windows and macOS logic is in cmake/LibobsFromInstall.cmake.
#
# Overrides, on any platform:
#   -DLIBOBS_INCLUDE_DIR=<obs-studio>/libobs -DLIBOBS_LIB=<path to libobs>
#   Both may be lists: on Windows, w32-pthreads' headers and import library
#   are needed alongside libobs's.
# macOS, headers only (nothing is linked; OBS supplies libobs when it loads
# the plugin), e.g. for a universal build:
#   -DLIBOBS_INCLUDE_DIR="/path/to/obs-studio/libobs;$(brew --prefix)/include" #   -DCMAKE_OSX_ARCHITECTURES="arm64;x86_64"
#
# .github/actions/build-obs-plugin builds all three the way CI does.
# `bun run build:bundle` copies the result into desktopApp/bin/, where the
# app's "Install OBS plugin" button deploys it from.

cmake_minimum_required(VERSION 3.18)
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
elseif(APPLE AND DEFINED LIBOBS_INCLUDE_DIR AND NOT DEFINED LIBOBS_LIB)
    target_include_directories(mirror-source PRIVATE ${LIBOBS_INCLUDE_DIR})
    target_link_options(mirror-source PRIVATE -undefined dynamic_lookup)
else()
    if(NOT (DEFINED LIBOBS_INCLUDE_DIR AND DEFINED LIBOBS_LIB))
        if(WIN32 OR APPLE)
            include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/LibobsFromInstall.cmake)
        else()
            # Distro default (Debian/Ubuntu: libobs-dev)
            set(LIBOBS_INCLUDE_DIR /usr/include/obs)
            set(LIBOBS_LIB obs)
        endif()
    endif()
    target_include_directories(mirror-source PRIVATE ${LIBOBS_INCLUDE_DIR})
    target_link_libraries(mirror-source PRIVATE ${LIBOBS_LIB})
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "Linux")
    # shm_open lives in librt on older glibc.
    target_link_libraries(mirror-source PRIVATE rt pthread)
endif()
# Windows: OpenFileMapping/MapViewOfFile are in kernel32, linked by default.
# macOS: shm_open and pthreads are in libSystem, linked by default.
