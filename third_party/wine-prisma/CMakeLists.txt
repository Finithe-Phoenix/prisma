cmake_minimum_required(VERSION 3.24)
project(prisma_wine_arm64 NONE)

find_program(PRISMA_POWERSHELL NAMES pwsh powershell REQUIRED)
get_filename_component(PRISMA_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)

set(PRISMA_WINE_OUTPUT "" CACHE PATH "Optional Wine ARM64 artifact directory under android/app/build")
set(PRISMA_WINE_JOBS "4" CACHE STRING "Parallel jobs inside the ARM64 builder")

set(PRISMA_WINE_BUILD_ARGS
  -NoProfile -ExecutionPolicy Bypass
  -File "${PRISMA_ROOT}/scripts/build-wine-arm64.ps1"
  -Jobs "${PRISMA_WINE_JOBS}")
if(PRISMA_WINE_OUTPUT)
  list(APPEND PRISMA_WINE_BUILD_ARGS -OutputDirectory "${PRISMA_WINE_OUTPUT}")
endif()

add_custom_target(prisma_wine_arm64
  COMMAND "${PRISMA_POWERSHELL}" ${PRISMA_WINE_BUILD_ARGS}
  WORKING_DIRECTORY "${PRISMA_ROOT}"
  USES_TERMINAL
  VERBATIM)
