cmake_minimum_required(VERSION 3.30)

project(prisma_futex_wait_table_smoke LANGUAGES CXX)

option(
    PRISMA_FUTEX_ENABLE_TSAN
    "Build the futex smoke target with ThreadSanitizer"
    OFF
)

find_package(Threads REQUIRED)

add_executable(prisma_futex_wait_table_smoke main.cpp)
target_compile_features(prisma_futex_wait_table_smoke PRIVATE cxx_std_20)
target_include_directories(
    prisma_futex_wait_table_smoke
    PRIVATE ${CMAKE_CURRENT_LIST_DIR}/../../include
)
target_link_libraries(prisma_futex_wait_table_smoke PRIVATE Threads::Threads)

if(MSVC)
    target_compile_options(
        prisma_futex_wait_table_smoke
        PRIVATE /W4 /WX /permissive- /Zc:__cplusplus
    )
else()
    target_compile_options(
        prisma_futex_wait_table_smoke
        PRIVATE
            -Wall
            -Wextra
            -Wpedantic
            -Wshadow
            -Wconversion
            -Wsign-conversion
            -Wold-style-cast
            -Wnon-virtual-dtor
            -Wcast-align
            -Wunused
            -Woverloaded-virtual
            -Wnull-dereference
            -Wdouble-promotion
            -Werror
    )
endif()

if(PRISMA_FUTEX_ENABLE_TSAN)
    if(MSVC)
        message(FATAL_ERROR "ThreadSanitizer is unavailable with MSVC")
    endif()
    target_compile_options(
        prisma_futex_wait_table_smoke
        PRIVATE -fsanitize=thread -fno-omit-frame-pointer
    )
    target_link_options(
        prisma_futex_wait_table_smoke
        PRIVATE -fsanitize=thread
    )
endif()

enable_testing()
add_test(NAME prisma_futex_wait_table_smoke COMMAND prisma_futex_wait_table_smoke)
set_tests_properties(prisma_futex_wait_table_smoke PROPERTIES TIMEOUT 30)
