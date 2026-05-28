/*
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_vmcore.h"

int
bh_platform_init(void)
{
    return 0;
}

void
bh_platform_destroy(void)
{
}

int
os_printf(const char *format, ...)
{
    int ret;
    va_list ap;

    va_start(ap, format);
#ifndef BH_VPRINTF
    ret = vprintf(format, ap);
#else
    ret = BH_VPRINTF(format, ap);
#endif
    va_end(ap);
    return ret;
}

int
os_vprintf(const char *format, va_list ap)
{
#ifndef BH_VPRINTF
    return vprintf(format, ap);
#else
    return BH_VPRINTF(format, ap);
#endif
}

uint64
os_time_get_boot_us(void)
{
    return 0;
}

uint64
os_time_thread_cputime_us(void)
{
    return os_time_get_boot_us();
}

uint8 *
os_thread_get_stack_boundary(void)
{
    return NULL;
}
