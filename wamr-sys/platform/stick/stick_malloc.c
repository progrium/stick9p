/*
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include <string.h>

#include "platform_api_vmcore.h"
#include "wasm_export.h"

void *
os_malloc(unsigned size)
{
    return wasm_runtime_malloc(size);
}

void *
os_realloc(void *ptr, unsigned size)
{
    void *new_ptr = wasm_runtime_malloc(size);
    if (!new_ptr) {
        return NULL;
    }
    if (ptr) {
        memcpy(new_ptr, ptr, size);
        wasm_runtime_free(ptr);
    }
    return new_ptr;
}

void
os_free(void *ptr)
{
    wasm_runtime_free(ptr);
}

int
os_dumps_proc_mem_info(char *out, unsigned int size)
{
    (void)out;
    (void)size;
    return -1;
}
