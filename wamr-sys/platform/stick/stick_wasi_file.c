/*
 * Minimal WASI host file stubs for bare-metal Stick (no host filesystem).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_extension.h"
#include "libc_errno.h"

#include <string.h>

#define STICK_PREOPEN_FD 100

/*
 * Size of the resolved-path scratch buffer that WAMR's wasm_runtime_init_wasi
 * stack-allocates and passes to `os_realpath` on this platform. Must match
 * the same #define in core/iwasm/common/wasm_runtime_common.c (compile-time
 * checked there, but kept here to keep `os_realpath`'s bound visibly correct).
 */
#define STICK_WASI_RESOLVED_PATH_BYTES 256

typedef struct {
    bool active;
    uint64_t cookie;
} stick_dir_t;

__wasi_errno_t
os_open_preopendir(const char *path, os_file_handle *out)
{
    (void)path;
    if (!out) {
        return __WASI_EINVAL;
    }
    *out = STICK_PREOPEN_FD;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_fstat(os_file_handle handle, struct __wasi_filestat_t *buf)
{
    if (!buf) {
        return __WASI_EINVAL;
    }
    if (handle == STICK_PREOPEN_FD) {
        memset(buf, 0, sizeof(*buf));
        buf->st_filetype = __WASI_FILETYPE_DIRECTORY;
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_fstatat(os_file_handle handle, const char *path,
           struct __wasi_filestat_t *buf, __wasi_lookupflags_t flags)
{
    (void)path;
    (void)flags;
    return os_fstat(handle, buf);
}

__wasi_errno_t
os_file_get_fdflags(os_file_handle handle, __wasi_fdflags_t *flags)
{
    if (!flags) {
        return __WASI_EINVAL;
    }
    if (handle == STICK_PREOPEN_FD) {
        *flags = 0;
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_file_set_fdflags(os_file_handle handle, __wasi_fdflags_t flags)
{
    (void)handle;
    (void)flags;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_fdatasync(os_file_handle handle)
{
    (void)handle;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_fsync(os_file_handle handle)
{
    (void)handle;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_openat(os_file_handle handle, const char *path, __wasi_oflags_t oflags,
          __wasi_fdflags_t fd_flags, __wasi_lookupflags_t lookup_flags,
          wasi_libc_file_access_mode access_mode, os_file_handle *out)
{
    (void)handle;
    (void)path;
    (void)oflags;
    (void)fd_flags;
    (void)lookup_flags;
    (void)access_mode;
    if (!out) {
        return __WASI_EINVAL;
    }
    return __WASI_ENOENT;
}

__wasi_errno_t
os_file_get_access_mode(os_file_handle handle,
                        wasi_libc_file_access_mode *access_mode)
{
    if (!access_mode) {
        return __WASI_EINVAL;
    }
    if (handle == STICK_PREOPEN_FD) {
        *access_mode = WASI_LIBC_ACCESS_MODE_READ_ONLY;
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_close(os_file_handle handle, bool is_stdio)
{
    (void)handle;
    (void)is_stdio;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_preadv(os_file_handle handle, const struct __wasi_iovec_t *iov, int iovcnt,
          __wasi_filesize_t offset, size_t *nread)
{
    (void)handle;
    (void)iov;
    (void)iovcnt;
    (void)offset;
    (void)nread;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_pwritev(os_file_handle handle, const struct __wasi_ciovec_t *iov, int iovcnt,
           __wasi_filesize_t offset, size_t *nwritten)
{
    (void)handle;
    (void)iov;
    (void)iovcnt;
    (void)offset;
    (void)nwritten;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_readv(os_file_handle handle, const struct __wasi_iovec_t *iov, int iovcnt,
         size_t *nread)
{
    (void)handle;
    (void)iov;
    (void)iovcnt;
    (void)nread;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_writev(os_file_handle handle, const struct __wasi_ciovec_t *iov, int iovcnt,
          size_t *nwritten)
{
    (void)handle;
    (void)iov;
    (void)iovcnt;
    (void)nwritten;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_fallocate(os_file_handle handle, __wasi_filesize_t offset,
             __wasi_filesize_t len)
{
    (void)handle;
    (void)offset;
    (void)len;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_ftruncate(os_file_handle handle, __wasi_filesize_t size)
{
    (void)handle;
    (void)size;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_futimens(os_file_handle handle, __wasi_timestamp_t access_time,
            __wasi_timestamp_t modification_time, __wasi_fstflags_t fstflags)
{
    (void)handle;
    (void)access_time;
    (void)modification_time;
    (void)fstflags;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_utimensat(os_file_handle handle, const char *path,
             __wasi_timestamp_t access_time,
             __wasi_timestamp_t modification_time, __wasi_fstflags_t fstflags,
             __wasi_lookupflags_t lookup_flags)
{
    (void)handle;
    (void)path;
    (void)access_time;
    (void)modification_time;
    (void)fstflags;
    (void)lookup_flags;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_readlinkat(os_file_handle handle, const char *path, char *buf,
              size_t bufsize, size_t *nread)
{
    (void)handle;
    (void)path;
    (void)buf;
    (void)bufsize;
    (void)nread;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_linkat(os_file_handle from_handle, const char *from_path,
          os_file_handle to_handle, const char *to_path,
          __wasi_lookupflags_t flags)
{
    (void)from_handle;
    (void)from_path;
    (void)to_handle;
    (void)to_path;
    (void)flags;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_symlinkat(const char *old_path, os_file_handle handle, const char *new_path)
{
    (void)old_path;
    (void)handle;
    (void)new_path;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_mkdirat(os_file_handle handle, const char *path)
{
    (void)handle;
    (void)path;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_renameat(os_file_handle old_handle, const char *old_path,
            os_file_handle new_handle, const char *new_path)
{
    (void)old_handle;
    (void)old_path;
    (void)new_handle;
    (void)new_path;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_unlinkat(os_file_handle handle, const char *path, bool is_dir)
{
    (void)handle;
    (void)path;
    (void)is_dir;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_lseek(os_file_handle handle, __wasi_filedelta_t offset,
         __wasi_whence_t whence, __wasi_filesize_t *new_offset)
{
    (void)handle;
    (void)offset;
    (void)whence;
    (void)new_offset;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_fadvise(os_file_handle handle, __wasi_filesize_t offset,
           __wasi_filesize_t len, __wasi_advice_t advice)
{
    (void)handle;
    (void)offset;
    (void)len;
    (void)advice;
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_isatty(os_file_handle handle)
{
    if (handle == STDIN_FILENO || handle == STDOUT_FILENO
        || handle == STDERR_FILENO) {
        return __WASI_ESUCCESS;
    }
    return __WASI_ENOTTY;
}

bool
os_is_stdin_handle(os_file_handle fd)
{
    return fd == STDIN_FILENO;
}

bool
os_is_stdout_handle(os_file_handle fd)
{
    return fd == STDOUT_FILENO;
}

bool
os_is_stderr_handle(os_file_handle fd)
{
    return fd == STDERR_FILENO;
}

os_file_handle
os_convert_stdin_handle(os_raw_file_handle raw_stdin)
{
    return raw_stdin >= 0 ? raw_stdin : STDIN_FILENO;
}

os_file_handle
os_convert_stdout_handle(os_raw_file_handle raw_stdout)
{
    return raw_stdout >= 0 ? raw_stdout : STDOUT_FILENO;
}

os_file_handle
os_convert_stderr_handle(os_raw_file_handle raw_stderr)
{
    return raw_stderr >= 0 ? raw_stderr : STDERR_FILENO;
}

__wasi_errno_t
os_fdopendir(os_file_handle handle, os_dir_stream *dir_stream)
{
    stick_dir_t *dir;

    if (!dir_stream || handle != STICK_PREOPEN_FD) {
        return __WASI_EBADF;
    }
    dir = (stick_dir_t *)BH_MALLOC(sizeof(*dir));
    if (!dir) {
        return __WASI_ENOMEM;
    }
    dir->active = true;
    dir->cookie = 0;
    *dir_stream = dir;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_rewinddir(os_dir_stream dir_stream)
{
    stick_dir_t *dir = (stick_dir_t *)dir_stream;
    if (!dir) {
        return __WASI_EBADF;
    }
    dir->cookie = 0;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_seekdir(os_dir_stream dir_stream, __wasi_dircookie_t position)
{
    stick_dir_t *dir = (stick_dir_t *)dir_stream;
    if (!dir) {
        return __WASI_EBADF;
    }
    dir->cookie = position;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_readdir(os_dir_stream dir_stream, __wasi_dirent_t *entry,
           const char **d_name)
{
    stick_dir_t *dir = (stick_dir_t *)dir_stream;

    if (!dir || !entry || !d_name) {
        return __WASI_EINVAL;
    }
    *d_name = NULL;
    memset(entry, 0, sizeof(*entry));
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_closedir(os_dir_stream dir_stream)
{
    if (dir_stream) {
        BH_FREE(dir_stream);
    }
    return __WASI_ESUCCESS;
}

os_dir_stream
os_get_invalid_dir_stream()
{
    return NULL;
}

bool
os_is_dir_stream_valid(os_dir_stream *dir_stream)
{
    return dir_stream != NULL && *dir_stream != NULL;
}

bool
os_is_handle_valid(os_file_handle *handle)
{
    return handle != NULL && *handle >= 0;
}

char *
os_realpath(const char *path, char *resolved_path)
{
    /*
     * Stick has no real filesystem; WAMR only feeds us the WASI preopen dir
     * (".") so this is just a copy with a NUL terminator.
     *
     * Do NOT use strncpy(resolved_path, path, N) — POSIX strncpy is required
     * to NUL-pad the *entire* N-byte destination, so strncpy(dst, ".", 4095)
     * writes 4095 bytes. The caller in wasm_runtime_init_wasi only allocates
     * STICK_WASI_RESOLVED_PATH_BYTES (256) on the stack, so a strncpy with a
     * 4 KiB bound smashes ~3.8 KiB of locals (wasi_ctx, argv_list, env_list,
     * …) and later trips a NULL %s in vsnprintf during the WASI error path.
     */
    if (!path || !resolved_path) {
        return NULL;
    }
    size_t len = strlen(path);
    if (len >= STICK_WASI_RESOLVED_PATH_BYTES) {
        len = STICK_WASI_RESOLVED_PATH_BYTES - 1;
    }
    memcpy(resolved_path, path, len);
    resolved_path[len] = '\0';
    return resolved_path;
}

os_raw_file_handle
os_invalid_raw_handle(void)
{
    return -1;
}

int
os_ioctl(os_file_handle handle, int request, ...)
{
    (void)handle;
    (void)request;
    return -1;
}

int
os_poll(os_poll_file_handle *fds, os_nfds_t nfs, int timeout)
{
    (void)fds;
    (void)nfs;
    (void)timeout;
    return -1;
}

bool
os_compare_file_handle(os_file_handle handle1, os_file_handle handle2)
{
    return handle1 == handle2;
}
