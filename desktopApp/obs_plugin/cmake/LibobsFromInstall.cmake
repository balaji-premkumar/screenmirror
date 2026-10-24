# Finds libobs from an installed OBS Studio, for Windows and macOS builds that
# have no OBS source build to point at.
#
# OBS ships no development package on either platform, but everything the
# plugin needs is either already on a machine that has OBS installed or one
# download away:
#   - headers: libobs/ (and, on Windows, deps/w32-pthreads/) from the
#     obs-studio source at the tag that matches the installed OBS, so the
#     plugin compiles against the same API it will be loaded into;
#   - libraries: on Windows, import libraries generated from the installed
#     obs.dll and w32-pthreads.dll with MSVC's dumpbin and lib; on macOS, the
#     libobs.framework inside OBS.app, linked directly.
#
# Inputs (cache variables, all optional):
#   OBS_INSTALL_DIR  Windows: OBS install root (default %ProgramFiles%/obs-studio)
#                    macOS:   OBS.app           (default /Applications/OBS.app)
#   OBS_VERSION      Version to fetch headers for. Default: the installed one.
#
# Sets LIBOBS_INCLUDE_DIR and LIBOBS_LIB (both lists).

if(WIN32)
    set(_default_install "$ENV{ProgramFiles}/obs-studio")
else()
    set(_default_install "/Applications/OBS.app")
endif()
set(OBS_INSTALL_DIR "${_default_install}" CACHE PATH "OBS Studio install location")
file(TO_CMAKE_PATH "${OBS_INSTALL_DIR}" _obs_root)

if(WIN32)
    set(_obs_bin "${_obs_root}/bin/64bit")
    set(_obs_marker "${_obs_bin}/obs.dll")
else()
    set(_obs_framework "${_obs_root}/Contents/Frameworks/libobs.framework")
    set(_obs_marker "${_obs_framework}/libobs")
endif()

if(NOT EXISTS "${_obs_marker}")
    message(FATAL_ERROR
        "OBS Studio not found: ${_obs_marker} does not exist.\n"
        "Install OBS Studio, pass -DOBS_INSTALL_DIR=<install location>, or pass "
        "-DLIBOBS_INCLUDE_DIR and -DLIBOBS_LIB from an OBS source build.")
endif()

# ── Version ──────────────────────────────────────────────────────────────────
if(NOT OBS_VERSION)
    if(WIN32)
        execute_process(
            COMMAND powershell -NoProfile -NonInteractive -Command
                "(Get-Item -LiteralPath '${_obs_marker}').VersionInfo.ProductVersion"
            OUTPUT_VARIABLE _obs_ver
            OUTPUT_STRIP_TRAILING_WHITESPACE)
    else()
        execute_process(
            COMMAND /usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString"
                "${_obs_root}/Contents/Info.plist"
            OUTPUT_VARIABLE _obs_ver
            OUTPUT_STRIP_TRAILING_WHITESPACE)
    endif()
    string(REGEX MATCH "^[0-9]+\\.[0-9]+\\.[0-9]+" _obs_ver "${_obs_ver}")
    if(NOT _obs_ver)
        message(FATAL_ERROR
            "Could not read the installed OBS version from ${_obs_root}. "
            "Pass -DOBS_VERSION=<x.y.z> to say which headers to use.")
    endif()
    set(OBS_VERSION "${_obs_ver}" CACHE STRING "OBS version to fetch libobs headers for")
endif()
message(STATUS "Using OBS Studio ${OBS_VERSION} from ${_obs_root}")

# ── Headers ──────────────────────────────────────────────────────────────────
# Only the header directories are extracted; the rest of the tree is not
# needed. libobs carries its own simde under util/, which is what Apple
# Silicon builds need in place of the x86-only stub next to this file.
set(_obs_src "${CMAKE_BINARY_DIR}/obs-studio-${OBS_VERSION}")
if(NOT EXISTS "${_obs_src}/libobs/obs-module.h"
        OR NOT EXISTS "${_obs_src}/deps/w32-pthreads/pthread.h")
    set(_tarball "${CMAKE_BINARY_DIR}/obs-studio-${OBS_VERSION}.tar.gz")
    set(_url "https://github.com/obsproject/obs-studio/archive/refs/tags/${OBS_VERSION}.tar.gz")
    message(STATUS "Downloading libobs ${OBS_VERSION} headers")
    file(DOWNLOAD "${_url}" "${_tarball}" STATUS _dl_status)
    list(GET _dl_status 0 _dl_code)
    if(NOT _dl_code EQUAL 0)
        list(GET _dl_status 1 _dl_msg)
        file(REMOVE "${_tarball}")
        message(FATAL_ERROR "Downloading ${_url} failed: ${_dl_msg}")
    endif()
    file(ARCHIVE_EXTRACT
        INPUT "${_tarball}"
        DESTINATION "${CMAKE_BINARY_DIR}"
        PATTERNS
            "obs-studio-${OBS_VERSION}/libobs"
            "obs-studio-${OBS_VERSION}/deps/w32-pthreads")
    file(REMOVE "${_tarball}")
endif()

# obs-config.h includes obsconfig.h, which OBS generates during its own build.
# Plugins do not rely on the paths, and the release-candidate/beta flags are 0
# on tags.
set(OBS_DATA_PATH "../../data")
set(OBS_PLUGIN_PATH "../../obs-plugins/64bit")
set(OBS_PLUGIN_DESTINATION "obs-plugins/64bit")
set(OBS_RELEASE_CANDIDATE 0)
set(OBS_BETA 0)
configure_file("${_obs_src}/libobs/obsconfig.h.in" "${_obs_src}/libobs/obsconfig.h")

# ── Libraries ────────────────────────────────────────────────────────────────
if(APPLE)
    set(LIBOBS_INCLUDE_DIR "${_obs_src}/libobs")
    set(LIBOBS_LIB "${_obs_marker}")
    return()
endif()

set(LIBOBS_INCLUDE_DIR "${_obs_src}/libobs" "${_obs_src}/deps/w32-pthreads")

# dumpbin and lib sit next to cl.exe. NO_DEFAULT_PATH keeps an unrelated `lib`
# or `link` on PATH (Git for Windows ships coreutils `link`) from being picked.
get_filename_component(_msvc_bin "${CMAKE_C_COMPILER}" DIRECTORY)
find_program(OBS_DUMPBIN dumpbin PATHS "${_msvc_bin}" NO_DEFAULT_PATH REQUIRED)
find_program(OBS_LIB_TOOL lib PATHS "${_msvc_bin}" NO_DEFAULT_PATH REQUIRED)

# Writes <name>.lib for the installed <name>.dll and sets <out_var> to its path.
function(obs_import_lib name out_var)
    set(_dll "${_obs_bin}/${name}.dll")
    set(_lib "${CMAKE_BINARY_DIR}/${name}.lib")
    set(${out_var} "${_lib}" PARENT_SCOPE)
    if(EXISTS "${_lib}" AND NOT "${_dll}" IS_NEWER_THAN "${_lib}")
        return()
    endif()

    execute_process(
        COMMAND "${OBS_DUMPBIN}" /NOLOGO /EXPORTS "${_dll}"
        OUTPUT_VARIABLE _exports
        RESULT_VARIABLE _rc)
    if(NOT _rc EQUAL 0)
        message(FATAL_ERROR "dumpbin /EXPORTS ${_dll} failed (${_rc})")
    endif()

    # Export rows read "ordinal hint RVA name"; the name is the last column.
    string(REGEX MATCHALL "\n +[0-9]+ +[0-9A-Fa-f]+ +[0-9A-Fa-f]+ +[A-Za-z_][A-Za-z0-9_@?$]*"
        _rows "${_exports}")
    if(NOT _rows)
        message(FATAL_ERROR "No exports found in ${_dll}")
    endif()

    set(_def "LIBRARY ${name}\nEXPORTS\n")
    foreach(_row IN LISTS _rows)
        string(REGEX REPLACE ".* ([^ ]+)$" "\\1" _sym "${_row}")
        string(APPEND _def "    ${_sym}\n")
    endforeach()
    file(WRITE "${CMAKE_BINARY_DIR}/${name}.def" "${_def}")

    execute_process(
        COMMAND "${OBS_LIB_TOOL}" /NOLOGO /MACHINE:X64
            "/DEF:${CMAKE_BINARY_DIR}/${name}.def" "/OUT:${_lib}"
        OUTPUT_QUIET
        RESULT_VARIABLE _rc)
    if(NOT _rc EQUAL 0)
        message(FATAL_ERROR "Generating ${_lib} from ${_dll} failed (${_rc})")
    endif()
endfunction()

obs_import_lib(obs _obs_lib)
obs_import_lib(w32-pthreads _pthreads_lib)
set(LIBOBS_LIB "${_obs_lib}" "${_pthreads_lib}")
