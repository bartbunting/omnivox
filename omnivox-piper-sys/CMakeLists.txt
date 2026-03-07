# omnivox-piper-sys build system
#
# Builds piper's C++ code as a static library so Rust can link against it.
# The official piper CMakeLists only produces executables; this file reuses the
# same ExternalProject downloads but wraps piper.cpp + piper_bridge.cpp in a
# static library target.
#
# Dependencies downloaded on first build (requires network access):
#   - fmt 10.0.0       (~1 MB)
#   - spdlog 1.12.0    (~1 MB)
#   - piper-phonemize  (~10 MB + downloads onnxruntime ~100 MB during its build)

cmake_minimum_required(VERSION 3.13)
project(omnivox_piper_bridge C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(MSVC)
  add_compile_options("$<$<C_COMPILER_ID:MSVC>:/utf-8>")
  add_compile_options("$<$<CXX_COMPILER_ID:MSVC>:/utf-8>")
endif()

# Piper source directory (cloned by build.rs)
set(PIPER_SRC "${CMAKE_CURRENT_SOURCE_DIR}/piper/src/cpp")

# Install prefix for all deps built here
set(INSTALL_PREFIX "${CMAKE_CURRENT_BINARY_DIR}/install")
file(MAKE_DIRECTORY "${INSTALL_PREFIX}/lib")
file(MAKE_DIRECTORY "${INSTALL_PREFIX}/include")

include(ExternalProject)

# ---- fmt ---------------------------------------------------------------
set(FMT_VERSION "10.0.0")
set(FMT_DIR "${INSTALL_PREFIX}")

ExternalProject_Add(
  fmt_external
  PREFIX      "${CMAKE_CURRENT_BINARY_DIR}/fmt"
  URL         "https://github.com/fmtlib/fmt/archive/refs/tags/${FMT_VERSION}.zip"
  CMAKE_ARGS  -DCMAKE_INSTALL_PREFIX:PATH=${FMT_DIR}
              -DFMT_TEST:BOOL=OFF
              -DCMAKE_BUILD_TYPE=Release
)

# ---- spdlog ------------------------------------------------------------
set(SPDLOG_VERSION "1.12.0")
set(SPDLOG_DIR "${INSTALL_PREFIX}")

ExternalProject_Add(
  spdlog_external
  PREFIX      "${CMAKE_CURRENT_BINARY_DIR}/spdlog"
  URL         "https://github.com/gabime/spdlog/archive/refs/tags/v${SPDLOG_VERSION}.zip"
  CMAKE_ARGS  -DCMAKE_INSTALL_PREFIX:PATH=${SPDLOG_DIR}
              -DCMAKE_BUILD_TYPE=Release
)

# ---- piper-phonemize (also downloads onnxruntime) ----------------------
set(PIPER_PHONEMIZE_DIR "${INSTALL_PREFIX}")

ExternalProject_Add(
  piper_phonemize_external
  PREFIX      "${CMAKE_CURRENT_BINARY_DIR}/phonemize"
  URL         "https://github.com/rhasspy/piper-phonemize/archive/refs/heads/master.zip"
  CMAKE_ARGS  -DCMAKE_INSTALL_PREFIX:PATH=${PIPER_PHONEMIZE_DIR}
              -DCMAKE_BUILD_TYPE=Release
)

# ---- Static library: piper.cpp + piper_bridge.cpp ----------------------

add_library(piper_bridge STATIC
  "${PIPER_SRC}/piper.cpp"
  "${CMAKE_CURRENT_SOURCE_DIR}/piper_bridge.cpp"
)

add_dependencies(piper_bridge
  fmt_external
  spdlog_external
  piper_phonemize_external
)

target_compile_features(piper_bridge PUBLIC cxx_std_17)

target_compile_definitions(piper_bridge PUBLIC
  _PIPER_VERSION="omnivox-embedded"
  PIPER_LIB_DIR="${INSTALL_PREFIX}/lib"
)

target_include_directories(piper_bridge PUBLIC
  "${PIPER_SRC}"
  "${CMAKE_CURRENT_SOURCE_DIR}"
  "${INSTALL_PREFIX}/include"
)

target_link_directories(piper_bridge PUBLIC
  "${INSTALL_PREFIX}/lib"
)

# Install the static library where build.rs can find it
install(TARGETS piper_bridge
  ARCHIVE DESTINATION "${INSTALL_PREFIX}/lib"
)
