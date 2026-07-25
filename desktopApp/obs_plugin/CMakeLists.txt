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

cmake_minimum_required(VERSION 3.16)
project(mirror-source C)

set(CMAKE_C_STANDARD 11)

add_library(mirror-source MODULE mirror_source.c)
set_target_properties(mirror-source PROPERTIES PREFIX "")

# Prefer an installed obs package (obs-studio dev packages export libobs)
find_package(libobs QUIET)

if(TARGET OBS::libobs)
    target_link_libraries(mirror-source PRIVATE OBS::libobs)
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
else()
    target_link_libraries(mirror-source PRIVATE rt pthread)
endif()
