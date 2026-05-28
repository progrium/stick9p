# Stick / esp-hal bare-metal platform (no POSIX semaphore.h).
# SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

set(PLATFORM_SHARED_DIR ${CMAKE_CURRENT_LIST_DIR})

add_definitions(-DBH_PLATFORM_STICK)
add_definitions(-DWASM_HAVE_MREMAP=0)

include_directories(${PLATFORM_SHARED_DIR})
if (DEFINED WAMR_ROOT_DIR)
    include_directories(${WAMR_ROOT_DIR}/core/shared/platform/include)
else ()
    include_directories(${CMAKE_CURRENT_LIST_DIR}/../../../third_party/wasm-micro-runtime/core/shared/platform/include)
endif ()

set(PLATFORM_SHARED_SOURCE
    ${PLATFORM_SHARED_DIR}/stick_platform.c
    ${PLATFORM_SHARED_DIR}/stick_malloc.c
    ${PLATFORM_SHARED_DIR}/stick_thread_stub.c
    ${PLATFORM_SHARED_DIR}/stick_sleep.c
    ${PLATFORM_SHARED_DIR}/stick_memmap.c
)

if (WAMR_BUILD_LIBC_WASI EQUAL 1)
    if (DEFINED WAMR_ROOT_DIR)
        set(_WAMR_ROOT ${WAMR_ROOT_DIR})
    else ()
        set(_WAMR_ROOT ${CMAKE_CURRENT_LIST_DIR}/../../../third_party/wasm-micro-runtime)
    endif ()
    include(${_WAMR_ROOT}/core/shared/platform/common/libc-util/platform_common_libc_util.cmake)
    list(APPEND PLATFORM_SHARED_SOURCE
        ${PLATFORM_SHARED_DIR}/stick_wasi_file.c
        ${PLATFORM_SHARED_DIR}/stick_wasi_clock.c
        ${PLATFORM_SHARED_DIR}/stick_wasi_socket.c
        ${PLATFORM_SHARED_DIR}/stick_wasi_libc.c
        ${PLATFORM_COMMON_LIBC_UTIL_SOURCE}
    )
endif ()
