/*
 * libc symbols required by WAMR WASI on bare-metal Stick.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_vmcore.h"

#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <reent.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

static int stick_errno;
static struct _reent stick_reent;

int *
__errno(void)
{
    return &stick_errno;
}

struct _reent *
__getreent(void)
{
    return &stick_reent;
}

size_t
strcspn(const char *s, const char *reject)
{
    const char *p;
    size_t count = 0;

    if (!s || !reject) {
        return 0;
    }
    for (; *s; s++, count++) {
        for (p = reject; *p; p++) {
            if (*s == *p) {
                return count;
            }
        }
    }
    return count;
}

size_t
strspn(const char *s, const char *accept)
{
    const char *p;
    size_t count = 0;

    if (!s || !accept) {
        return 0;
    }
    for (; *s; s++, count++) {
        for (p = accept; *p; p++) {
            if (*s == *p) {
                break;
            }
        }
        if (!*p) {
            return count;
        }
    }
    return count;
}

char *
strtok_r(char *str, const char *delim, char **saveptr)
{
    char *start;

    if (!delim || !saveptr) {
        return NULL;
    }
    if (str) {
        start = str;
    }
    else {
        start = *saveptr;
        if (!start) {
            return NULL;
        }
    }

    start += strspn(start, delim);
    if (*start == '\0') {
        *saveptr = NULL;
        return NULL;
    }

    *saveptr = start + strcspn(start, delim);
    if (**saveptr != '\0') {
        *(*saveptr)++ = '\0';
    }
    else {
        *saveptr = NULL;
    }
    return start;
}

int
fputs(const char *s, FILE *stream)
{
    (void)s;
    (void)stream;
    return 0;
}

static int
stick_digit_value(char c, int base)
{
    if (c >= '0' && c <= '9') {
        int v = c - '0';
        return v < base ? v : -1;
    }
    if (base > 10) {
        if (c >= 'a' && c <= 'z') {
            int v = c - 'a' + 10;
            return v < base ? v : -1;
        }
        if (c >= 'A' && c <= 'Z') {
            int v = c - 'A' + 10;
            return v < base ? v : -1;
        }
    }
    return -1;
}

long
strtol(const char *nptr, char **endptr, int base)
{
    const char *s = nptr;
    int sign = 1;
    long acc = 0;
    int any = 0;

    if (!nptr) {
        errno = EINVAL;
        return 0;
    }
    if (base == 0) {
        if (*s == '0') {
            if (s[1] == 'x' || s[1] == 'X') {
                base = 16;
                s += 2;
            }
            else {
                base = 8;
                s += 1;
            }
        }
        else {
            base = 10;
        }
    }
    else if (base == 16 && s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
        s += 2;
    }

    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') {
        s++;
    }
    if (*s == '-') {
        sign = -1;
        s++;
    }
    else if (*s == '+') {
        s++;
    }

    while (1) {
        int d = stick_digit_value(*s, base);
        if (d < 0) {
            break;
        }
        any = 1;
        if (acc > (LONG_MAX - d) / base) {
            errno = ERANGE;
            acc = sign < 0 ? LONG_MIN : LONG_MAX;
            s++;
            break;
        }
        acc = acc * base + d;
        s++;
    }

    if (endptr) {
        *endptr = any ? (char *)s : (char *)nptr;
    }
    return sign < 0 ? -acc : acc;
}

unsigned long
strtoul(const char *nptr, char **endptr, int base)
{
    long v = strtol(nptr, endptr, base);
    if (v < 0) {
        return 0;
    }
    return (unsigned long)v;
}

int
clock_gettime(clockid_t clk_id, struct timespec *tp)
{
    uint64_t boot_us;

    (void)clk_id;
    if (!tp) {
        errno = EINVAL;
        return -1;
    }
    boot_us = os_time_get_boot_us();
    tp->tv_sec = (time_t)(boot_us / 1000000ULL);
    tp->tv_nsec = (long)((boot_us % 1000000ULL) * 1000ULL);
    return 0;
}

int
nanosleep(const struct timespec *req, struct timespec *rem)
{
    (void)req;
    if (rem) {
        rem->tv_sec = 0;
        rem->tv_nsec = 0;
    }
    return 0;
}

int
sched_yield(void)
{
    return 0;
}

int
pthread_cond_timedwait(pthread_cond_t *cond, pthread_mutex_t *mutex,
                       const struct timespec *abstime)
{
    (void)cond;
    (void)mutex;
    (void)abstime;
    return 0;
}
