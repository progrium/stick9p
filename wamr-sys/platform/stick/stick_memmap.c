/*
 * malloc-based memmap stubs for esp-hal (no POSIX mmap).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include <string.h>

#include "platform_api_vmcore.h"

void *
os_mmap(void *hint, size_t size, int prot, int flags, os_file_handle file)
{
    void *addr;

    (void)hint;
    (void)prot;
    (void)flags;
    (void)file;

    addr = BH_MALLOC(size);
    if (addr) {
        memset(addr, 0, size);
    }
    return addr;
}

void
os_munmap(void *addr, size_t size)
{
    (void)size;
    BH_FREE(addr);
}

int
os_mprotect(void *addr, size_t size, int prot)
{
    (void)addr;
    (void)size;
    (void)prot;
    return 0;
}

void *
os_mremap(void *old_addr, size_t old_size, size_t new_size)
{
    return os_mremap_slow(old_addr, old_size, new_size);
}
