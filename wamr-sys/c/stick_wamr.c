/*
 * Stick host glue: capture guest output and run embedded wasm bytes via WAMR.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "wasm_export.h"

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define CAPTURE_MAX 8192
#define RUNTIME_HEAP_BYTES (192 * 1024)
#define GUEST_STACK_BYTES (64 * 1024)
/* WASI guests manage their own heap in linear memory. */
#define GUEST_HEAP_BYTES 0
#define WASI_DIR_COUNT 1
#define STICK_MAX_ARGV 8
#define STICK_MAX_ENV 16
#define STICK_STR_LEN 64


static char g_capture[CAPTURE_MAX];
static size_t g_capture_len;
static volatile char *g_runtime_heap;
static volatile uint32_t g_runtime_heap_size;
#if !defined(STICK_WAMR_HEAP_EXTERNAL)
static char g_runtime_heap_static[RUNTIME_HEAP_BYTES];
#endif
static volatile bool g_runtime_ready;
#if WASM_ENABLE_LIBC_WASI == 0
static bool g_natives_registered;
#endif


void
stick_wamr_set_runtime_heap(void *heap_buf, uint32_t heap_size)
{
    g_runtime_heap = (char *)heap_buf;
    g_runtime_heap_size = heap_size;
}

void
stick_wamr_capture_reset(void)
{
    g_capture_len = 0;
    g_capture[0] = '\0';
}

const char *
stick_wamr_capture_ptr(void)
{
    return g_capture;
}

size_t
stick_wamr_capture_len(void)
{
    return g_capture_len;
}

static void
capture_append(const char *data, size_t len)
{
    if (!data || len == 0) {
        return;
    }
    size_t room = CAPTURE_MAX - 1 - g_capture_len;
    if (room == 0) {
        return;
    }
    size_t n = len < room ? len : room;
    memcpy(g_capture + g_capture_len, data, n);
    g_capture_len += n;
    g_capture[g_capture_len] = '\0';
}

int
stick_wamr_vprintf(const char *format, va_list ap)
{
    char tmp[256];
    int n = vsnprintf(tmp, sizeof(tmp), format, ap);
    if (n > 0) {
        capture_append(tmp, (size_t)n);
    }
    return n;
}

#if WASM_ENABLE_LIBC_WASI == 0
static void
wasm_env_log(wasm_exec_env_t exec_env, uint32_t ptr, uint32_t len)
{
    wasm_module_inst_t inst = wasm_runtime_get_module_inst(exec_env);
    char *native;

    if (!inst) {
        return;
    }
    native = wasm_runtime_addr_app_to_native(inst, ptr);
    if (!native) {
        return;
    }
    capture_append(native, len);
}

static NativeSymbol native_symbols[] = {
    { "log", (void *)wasm_env_log, "(ii)", NULL },
};
#endif

static bool runtime_init_once(void);

#if WASM_ENABLE_LIBC_WASI != 0
static int
copy_wasi_str_table(char bufs[][STICK_STR_LEN], char *ptrs[], char *inputs[],
                    uint32_t count, uint32_t max_count)
{
    uint32_t i;

    if (count > max_count) {
        return -1;
    }
    for (i = 0; i < count; i++) {
        if (!inputs[i]) {
            return -1;
        }
        snprintf(bufs[i], STICK_STR_LEN, "%s", inputs[i]);
        ptrs[i] = bufs[i];
    }
    return 0;
}
#endif

int
stick_wamr_init_runtime(void)
{
    return runtime_init_once() ? 0 : -1;
}

static bool
runtime_init_once(void)
{
    RuntimeInitArgs init_args;
    char *heap_buf;
    uint32_t heap_size;

    if (g_runtime_ready) {
        return true;
    }

#if defined(STICK_WAMR_HEAP_EXTERNAL)
    if (g_runtime_heap == NULL || g_runtime_heap_size == 0) {
        return false;
    }
    heap_buf = g_runtime_heap;
    heap_size = g_runtime_heap_size;
#else
    heap_buf = g_runtime_heap_static;
    heap_size = sizeof(g_runtime_heap_static);
#endif

    memset(&init_args, 0, sizeof(init_args));
    init_args.mem_alloc_type = Alloc_With_Pool;
    init_args.mem_alloc_option.pool.heap_buf = heap_buf;
    init_args.mem_alloc_option.pool.heap_size = heap_size;

    if (!wasm_runtime_full_init(&init_args)) {
        return false;
    }

#if WASM_ENABLE_LIBC_WASI == 0
    if (!g_natives_registered) {
        if (!wasm_runtime_register_natives(
                "env", native_symbols,
                (uint32_t)(sizeof(native_symbols) / sizeof(native_symbols[0])))) {
            return false;
        }
        g_natives_registered = true;
    }
#endif

    g_runtime_ready = true;
    return true;
}

int
stick_wamr_run(const uint8_t *wasm_bytes, uint32_t wasm_len, char *argv[],
               uint32_t argc, const char *env[], uint32_t env_count, char *err,
               uint32_t err_len)
{
    wasm_module_t module = NULL;
    wasm_module_inst_t module_inst = NULL;
    const char *exception = NULL;
    uint8_t *wasm_copy = NULL;
    LoadArgs load_args;
#if WASM_ENABLE_LIBC_WASI != 0
    const char *dir_list[WASI_DIR_COUNT] = { "." };
    static char argv_bufs[STICK_MAX_ARGV][STICK_STR_LEN];
    static char *argv_copy[STICK_MAX_ARGV];
    static char env_bufs[STICK_MAX_ENV][STICK_STR_LEN];
    static char *env_copy[STICK_MAX_ENV];
#endif

    if (!wasm_bytes || wasm_len == 0) {
        snprintf(err, err_len, "empty wasm");
        return -1;
    }

    stick_wamr_capture_reset();

    if (!runtime_init_once()) {
        snprintf(err, err_len, "wamr init failed");
        return -1;
    }

    /*
     * WAMR rewrites export/import name bytes in-place when loading from a
     * non-freeable buffer (see wasm_const_str_list_insert). Flash/rodata from
     * include_bytes! is not writable — copy to the runtime pool first.
     */
    wasm_copy = (uint8_t *)wasm_runtime_malloc(wasm_len);
    if (!wasm_copy) {
        snprintf(err, err_len, "wasm copy OOM");
        return -1;
    }
    memcpy(wasm_copy, wasm_bytes, wasm_len);

    memset(&load_args, 0, sizeof(load_args));
    load_args.wasm_binary_freeable = true;

    module = wasm_runtime_load_ex(wasm_copy, wasm_len, &load_args, err, err_len);
    if (!module) {
        wasm_runtime_free(wasm_copy);
        return -1;
    }

#if WASM_ENABLE_LIBC_WASI != 0
    if (copy_wasi_str_table(argv_bufs, argv_copy, argv, argc, STICK_MAX_ARGV) != 0
        || copy_wasi_str_table(env_bufs, env_copy, (char **)env, env_count,
                               STICK_MAX_ENV)
               != 0) {
        snprintf(err, err_len, "wasi argv/env too large");
        wasm_runtime_unload(module);
        wasm_runtime_free(wasm_copy);
        return -1;
    }
    wasm_runtime_set_wasi_args(module, dir_list, WASI_DIR_COUNT, NULL, 0,
                               (const char **)env_copy, env_count, argv_copy,
                               (int)argc);
#endif

    module_inst = wasm_runtime_instantiate(module, GUEST_STACK_BYTES,
                                           GUEST_HEAP_BYTES, err, err_len);
    if (!module_inst) {
        wasm_runtime_unload(module);
        wasm_runtime_free(wasm_copy);
        return -1;
    }

    if (!wasm_application_execute_main(module_inst, 0, NULL)) {
        exception = wasm_runtime_get_exception(module_inst);
        snprintf(err, err_len, "%s",
                 exception ? exception : "wasm call failed");
        wasm_runtime_deinstantiate(module_inst);
        wasm_runtime_unload(module);
        wasm_runtime_free(wasm_copy);
        return -1;
    }

    wasm_runtime_deinstantiate(module_inst);
    wasm_runtime_unload(module);
    wasm_runtime_free(wasm_copy);
    return 0;
}
