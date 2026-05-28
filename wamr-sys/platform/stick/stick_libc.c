/*
 * Minimal libc helpers for bare-metal esp-hal (no full newlib link).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include <stddef.h>

void *
bsearch(const void *key, const void *base0, size_t nmemb, size_t size,
        int (*compar)(const void *, const void *))
{
    const char *base = (const char *)base0;
    size_t lo = 0;
    size_t hi = nmemb;

    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const void *p = base + mid * size;
        int cmp = compar(key, p);

        if (cmp == 0) {
            return (void *)p;
        }
        if (cmp < 0) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    return NULL;
}

static void
swap_bytes(char *a, char *b, size_t size)
{
    char tmp;
    while (size--) {
        tmp = *a;
        *a++ = *b;
        *b++ = tmp;
    }
}

static int
partition(char *base, size_t nel, size_t size,
          int (*compar)(const void *, const void *))
{
    char *pivot = base;
    size_t i = 1;

    for (size_t j = 1; j < nel; j++) {
        char *elem = base + j * size;
        if (compar(elem, pivot) <= 0) {
            swap_bytes(base + i * size, elem, size);
            i++;
        }
    }
    swap_bytes(base, base + (i - 1) * size, size);
    return (int)(i - 1);
}

static void
qsort_impl(char *base, size_t nel, size_t size,
           int (*compar)(const void *, const void *))
{
    if (nel < 2) {
        return;
    }
    int p = partition(base, nel, size, compar);
    qsort_impl(base, (size_t)p, size, compar);
    qsort_impl(base + ((size_t)p + 1) * size, nel - (size_t)p - 1, size, compar);
}

void
qsort(void *base, size_t nel, size_t size,
      int (*compar)(const void *, const void *))
{
    if (!base || !compar || size == 0) {
        return;
    }
    qsort_impl((char *)base, nel, size, compar);
}
