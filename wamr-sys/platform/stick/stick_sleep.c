/*
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_extension.h"

int
os_usleep(uint32 usec)
{
    (void)usec;
    return 0;
}
