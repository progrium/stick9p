/*
 * Minimal WAMR platform for esp-hal / Xtensa newlib (no semaphore.h).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#ifndef _PLATFORM_INTERNAL_H
#define _PLATFORM_INTERNAL_H

#include <stdint.h>
#include <stdarg.h>
#include <stdbool.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <limits.h>
#include <time.h>
#include <pthread.h>

/* WAMR WASI init keeps resolved_path[PATH_MAX] on the stack; newlib's 4096
 * blows the AppCpu worker stack during wasm_runtime_init_wasi. */
#ifdef PATH_MAX
#undef PATH_MAX
#endif
#define PATH_MAX 256

#ifdef __cplusplus
extern "C" {
#endif

#ifndef BH_PLATFORM_STICK
#define BH_PLATFORM_STICK
#endif

typedef pthread_t korp_tid;
typedef pthread_mutex_t korp_mutex;
typedef pthread_cond_t korp_cond;
typedef pthread_t korp_thread;
typedef pthread_rwlock_t korp_rwlock;
typedef unsigned int korp_sem;

#define OS_THREAD_MUTEX_INITIALIZER PTHREAD_MUTEX_INITIALIZER
#define BH_APPLET_PRESERVED_STACK_SIZE (2 * BH_KB)
#define BH_THREAD_DEFAULT_PRIORITY 5

static inline int
os_getpagesize(void)
{
    return 4096;
}

typedef int os_file_handle;
typedef void *os_dir_stream;
typedef int os_raw_file_handle;

struct stick_pollfd {
    int fd;
    short events;
    short revents;
};
typedef struct stick_pollfd os_poll_file_handle;
typedef unsigned int os_nfds_t;
typedef struct timespec os_timespec;

#ifndef POLLIN
#define POLLIN 0x0001
#define POLLOUT 0x0004
#define POLLERR 0x0008
#define POLLHUP 0x0010
#define POLLNVAL 0x0020
#endif

#ifndef FIONREAD
#define FIONREAD 0x541B
#endif

static inline uint16_t
stick_htons(uint16_t x)
{
    return (uint16_t)(((x & 0xffu) << 8) | ((x >> 8) & 0xffu));
}

static inline uint32_t
stick_htonl(uint32_t x)
{
    return ((x & 0xffu) << 24) | ((x & 0xff00u) << 8) | ((x & 0xff0000u) >> 8)
           | ((x >> 24) & 0xffu);
}

#define htons stick_htons
#define htonl stick_htonl

static inline os_file_handle
os_get_invalid_handle(void)
{
    return -1;
}

#ifndef STDIN_FILENO
#define STDIN_FILENO 0
#endif
#ifndef STDOUT_FILENO
#define STDOUT_FILENO 1
#endif
#ifndef STDERR_FILENO
#define STDERR_FILENO 2
#endif

#if WASM_ENABLE_LIBC_WASI != 0
#define CONFIG_HAS_D_INO 0
#define CONFIG_HAS_ISATTY 1
#define CONFIG_HAS_PWRITEV 0
#define CONFIG_HAS_PREADV 0
#define CONFIG_HAS_POSIX_FALLOCATE 0
#define CONFIG_HAS_FDATASYNC 0
#endif

#ifdef __cplusplus
}
#endif

#endif /* _PLATFORM_INTERNAL_H */
