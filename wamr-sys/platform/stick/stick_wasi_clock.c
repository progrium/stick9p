/*
 * Minimal WASI clock stubs for bare-metal Stick.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_extension.h"
#include "platform_api_vmcore.h"
#include "libc_errno.h"

#define NANOSECONDS_PER_SECOND 1000000000ULL

__wasi_errno_t
os_clock_res_get(__wasi_clockid_t clock_id, __wasi_timestamp_t *resolution)
{
    if (!resolution) {
        return __WASI_EINVAL;
    }
    switch (clock_id) {
        case __WASI_CLOCK_PROCESS_CPUTIME_ID:
        case __WASI_CLOCK_THREAD_CPUTIME_ID:
            return __WASI_ENOTSUP;
        case __WASI_CLOCK_REALTIME:
        case __WASI_CLOCK_MONOTONIC:
            *resolution = NANOSECONDS_PER_SECOND / 1000000;
            return __WASI_ESUCCESS;
        default:
            return __WASI_EINVAL;
    }
}

__wasi_errno_t
os_clock_time_get(__wasi_clockid_t clock_id, __wasi_timestamp_t precision,
                  __wasi_timestamp_t *time)
{
    uint64_t boot_us;

    (void)precision;
    if (!time) {
        return __WASI_EINVAL;
    }
    switch (clock_id) {
        case __WASI_CLOCK_PROCESS_CPUTIME_ID:
        case __WASI_CLOCK_THREAD_CPUTIME_ID:
            return __WASI_ENOTSUP;
        case __WASI_CLOCK_REALTIME:
        case __WASI_CLOCK_MONOTONIC:
            boot_us = os_time_get_boot_us();
            *time = boot_us * 1000;
            return __WASI_ESUCCESS;
        default:
            return __WASI_EINVAL;
    }
}
