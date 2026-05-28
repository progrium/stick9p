/*
 * Single-threaded WAMR thread/mutex stubs (no pthread on esp-hal firmware link).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_vmcore.h"
#include "platform_api_extension.h"

korp_tid
os_self_thread(void)
{
    return (korp_tid)1;
}

int
os_mutex_init(korp_mutex *mutex)
{
    (void)mutex;
    return BHT_OK;
}

int
os_recursive_mutex_init(korp_mutex *mutex)
{
    return os_mutex_init(mutex);
}

int
os_mutex_destroy(korp_mutex *mutex)
{
    (void)mutex;
    return BHT_OK;
}

int
os_mutex_lock(korp_mutex *mutex)
{
    (void)mutex;
    return BHT_OK;
}

int
os_mutex_unlock(korp_mutex *mutex)
{
    (void)mutex;
    return BHT_OK;
}

int
os_thread_create_with_prio(korp_tid *tid, thread_start_routine_t start, void *arg,
                           unsigned int stack_size, int prio)
{
    (void)tid;
    (void)start;
    (void)arg;
    (void)stack_size;
    (void)prio;
    return BHT_ERROR;
}

int
os_thread_create(korp_tid *tid, thread_start_routine_t start, void *arg,
                 unsigned int stack_size)
{
    return os_thread_create_with_prio(tid, start, arg, stack_size,
                                      BH_THREAD_DEFAULT_PRIORITY);
}

int
os_thread_join(korp_tid thread, void **retval)
{
    (void)thread;
    (void)retval;
    return BHT_ERROR;
}

int
os_thread_detach(korp_tid tid)
{
    (void)tid;
    return BHT_OK;
}

void
os_thread_exit(void *retval)
{
    (void)retval;
}

int
os_cond_init(korp_cond *cond)
{
    (void)cond;
    return BHT_OK;
}

int
os_cond_destroy(korp_cond *cond)
{
    (void)cond;
    return BHT_OK;
}

int
os_cond_wait(korp_cond *cond, korp_mutex *mutex)
{
    (void)cond;
    (void)mutex;
    return BHT_OK;
}

int
os_cond_reltimedwait(korp_cond *cond, korp_mutex *mutex, uint64 useconds)
{
    (void)cond;
    (void)mutex;
    (void)useconds;
    return BHT_OK;
}

int
os_cond_signal(korp_cond *cond)
{
    (void)cond;
    return BHT_OK;
}

int
os_cond_broadcast(korp_cond *cond)
{
    (void)cond;
    return BHT_OK;
}

int
os_rwlock_init(korp_rwlock *lock)
{
    (void)lock;
    return BHT_OK;
}

int
os_rwlock_rdlock(korp_rwlock *lock)
{
    (void)lock;
    return BHT_OK;
}

int
os_rwlock_wrlock(korp_rwlock *lock)
{
    (void)lock;
    return BHT_OK;
}

int
os_rwlock_unlock(korp_rwlock *lock)
{
    (void)lock;
    return BHT_OK;
}

int
os_rwlock_destroy(korp_rwlock *lock)
{
    (void)lock;
    return BHT_OK;
}
