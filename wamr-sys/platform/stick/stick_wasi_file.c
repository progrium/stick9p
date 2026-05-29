/*
 * WASI file hooks backed by the stick9p VFS (Rust `stick_vfs_*` FFI).
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

#include "platform_api_extension.h"
#include "libc_errno.h"

#include <string.h>

extern void stick_wamr_stream_output(const uint8_t *data, size_t len);
extern bool stick_task_should_terminate(void);

#define STICK_WASI_RESOLVED_PATH_BYTES 256

#define STICK_TAG_DIR 0x10000000
#define STICK_TAG_FILE 0x20000000
#define STICK_TAG_MASK 0xF0000000
#define STICK_ID_MASK 0x0FFFFFFF
#define STICK_MAX_OPEN_FILES 16
#define STICK_DIRENT_NAME_CAP 256

typedef struct {
    bool active;
    uint32_t vfs_id;
    __wasi_dircookie_t cookie;
} stick_dir_t;

typedef struct {
    uint8_t used;
    uint32_t vfs_id;
    uint64_t offset;
} stick_open_file_t;

static stick_open_file_t stick_files[STICK_MAX_OPEN_FILES];
static char stick_dirent_name[STICK_DIRENT_NAME_CAP];

extern bool stick_vfs_ready(void);
extern int stick_vfs_preopen_ino(const char *path);
extern int stick_vfs_walk(uint32_t parent, const char *name);
extern bool stick_vfs_is_dir(uint32_t id);
extern uint64_t stick_vfs_length(uint32_t id);
extern int stick_vfs_read(uint32_t id, uint64_t off, uint8_t *buf, size_t len);
extern int stick_vfs_write(uint32_t id, uint64_t off, const uint8_t *buf,
                            size_t len);
extern int stick_vfs_child_at(uint32_t parent, uint32_t index);
extern int stick_vfs_name(uint32_t id, char *buf, size_t buflen);

static bool
stick_handle_is_dir(os_file_handle handle)
{
    return (handle & STICK_TAG_MASK) == STICK_TAG_DIR;
}

static bool
stick_handle_is_file(os_file_handle handle)
{
    return (handle & STICK_TAG_MASK) == STICK_TAG_FILE;
}

static uint32_t
stick_handle_id(os_file_handle handle)
{
    return (uint32_t)(handle & STICK_ID_MASK);
}

static os_file_handle
stick_dir_handle_id(uint32_t id)
{
    return (os_file_handle)(STICK_TAG_DIR | (id & STICK_ID_MASK));
}

static int
stick_handle_file_slot(os_file_handle handle)
{
    if (!stick_handle_is_file(handle)) {
        return -1;
    }
    int slot = handle & 0xFF;
    if (slot < 0 || slot >= STICK_MAX_OPEN_FILES) {
        return -1;
    }
    return slot;
}

static os_file_handle
stick_dir_handle(uint32_t id)
{
    return stick_dir_handle_id(id);
}

static os_file_handle
stick_file_handle(int slot)
{
    return (os_file_handle)(STICK_TAG_FILE | slot);
}

static bool
stick_is_managed_handle(os_file_handle handle)
{
    return stick_handle_is_dir(handle) || stick_handle_is_file(handle);
}

static void
stick_fill_stat(uint32_t id, struct __wasi_filestat_t *buf)
{
    memset(buf, 0, sizeof(*buf));
    buf->st_dev = 1;
    buf->st_ino = id;
    buf->st_nlink = 1;
    if (stick_vfs_is_dir(id)) {
        buf->st_filetype = __WASI_FILETYPE_DIRECTORY;
        buf->st_size = 0;
    }
    else {
        buf->st_filetype = __WASI_FILETYPE_REGULAR_FILE;
        buf->st_size = stick_vfs_length(id);
    }
}

static int
stick_resolve_at(uint32_t dir_id, const char *path, uint32_t *out_id)
{
    uint32_t cur;
    const char *p;

    if (!out_id || !stick_vfs_ready()) {
        return 0;
    }
    if (!path || path[0] == '\0') {
        *out_id = dir_id;
        return 1;
    }

    if (path[0] == '/') {
        cur = (uint32_t)stick_vfs_preopen_ino(NULL);
        p = path;
        while (*p == '/') {
            p++;
        }
    }
    else {
        cur = dir_id;
        p = path;
    }

    if (*p == '\0') {
        *out_id = cur;
        return 1;
    }

    while (*p != '\0') {
        char component[256];
        size_t i = 0;
        int next;

        while (*p == '/') {
            p++;
        }
        if (*p == '\0') {
            break;
        }
        while (*p != '\0' && *p != '/') {
            if (i + 1 >= sizeof(component)) {
                return 0;
            }
            component[i++] = *p++;
        }
        component[i] = '\0';
        if (component[0] == '\0') {
            continue;
        }
        next = stick_vfs_walk(cur, component);
        if (next < 0) {
            return 0;
        }
        cur = (uint32_t)next;
    }

    *out_id = cur;
    return 1;
}

static int
stick_file_slot_alloc(uint32_t vfs_id)
{
    int i;
    for (i = 0; i < STICK_MAX_OPEN_FILES; i++) {
        if (!stick_files[i].used) {
            stick_files[i].used = 1;
            stick_files[i].vfs_id = vfs_id;
            stick_files[i].offset = 0;
            return i;
        }
    }
    return -1;
}

static void
stick_file_slot_free(int slot)
{
    if (slot >= 0 && slot < STICK_MAX_OPEN_FILES) {
        stick_files[slot].used = 0;
        stick_files[slot].vfs_id = 0;
        stick_files[slot].offset = 0;
    }
}

static size_t
stick_iov_total(const struct __wasi_iovec_t *iov, int iovcnt)
{
    size_t total = 0;
    int i;
    for (i = 0; i < iovcnt; i++) {
        total += iov[i].buf_len;
    }
    return total;
}

static size_t
stick_ciov_total(const struct __wasi_ciovec_t *iov, int iovcnt)
{
    size_t total = 0;
    int i;
    for (i = 0; i < iovcnt; i++) {
        total += iov[i].buf_len;
    }
    return total;
}

static __wasi_errno_t
stick_stream_writev(const struct __wasi_ciovec_t *iov, int iovcnt, size_t *nwritten)
{
    if (stick_task_should_terminate()) {
        return __WASI_EINTR;
    }
    uint8_t stack_buf[512];
    size_t cap;
    uint8_t *tmp = stack_buf;
    size_t written = 0;
    int i;

    if (!nwritten) {
        return __WASI_EINVAL;
    }
    cap = stick_ciov_total(iov, iovcnt);
    if (cap == 0) {
        *nwritten = 0;
        return __WASI_ESUCCESS;
    }
    if (cap > sizeof(stack_buf)) {
        tmp = (uint8_t *)BH_MALLOC(cap);
        if (!tmp) {
            return __WASI_ENOMEM;
        }
    }
    for (i = 0; i < iovcnt; i++) {
        if (iov[i].buf_len > 0) {
            memcpy(tmp + written, iov[i].buf, iov[i].buf_len);
            written += iov[i].buf_len;
        }
    }
    stick_wamr_stream_output(tmp, written);
    if (tmp != stack_buf) {
        BH_FREE(tmp);
    }
    *nwritten = written;
    return __WASI_ESUCCESS;
}

static __wasi_errno_t
stick_read_into_iov(uint32_t id, uint64_t offset, const struct __wasi_iovec_t *iov,
                    int iovcnt, size_t *nread)
{
    if (stick_task_should_terminate()) {
        return __WASI_EINTR;
    }
    uint8_t stack_buf[512];
    size_t cap = stick_iov_total(iov, iovcnt);
    uint8_t *tmp = stack_buf;
    size_t got;
    size_t copied = 0;
    int i;

    if (!nread) {
        return __WASI_EINVAL;
    }
    *nread = 0;
    if (cap == 0) {
        return __WASI_ESUCCESS;
    }
    if (cap > sizeof(stack_buf)) {
        tmp = (uint8_t *)BH_MALLOC(cap);
        if (!tmp) {
            return __WASI_ENOMEM;
        }
    }

    got = (size_t)stick_vfs_read(id, offset, tmp, cap);
    for (i = 0; i < iovcnt && copied < got; i++) {
        size_t n = iov[i].buf_len;
        if (n > got - copied) {
            n = got - copied;
        }
        if (n > 0) {
            memcpy(iov[i].buf, tmp + copied, n);
            copied += n;
        }
    }
    if (tmp != stack_buf) {
        BH_FREE(tmp);
    }
    *nread = copied;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_open_preopendir(const char *path, os_file_handle *out)
{
    int id;

    if (!out) {
        return __WASI_EINVAL;
    }
    if (!stick_vfs_ready()) {
        return __WASI_ENOENT;
    }
    id = stick_vfs_preopen_ino(path);
    if (id < 0 || !stick_vfs_is_dir((uint32_t)id)) {
        return __WASI_ENOENT;
    }
    *out = stick_dir_handle((uint32_t)id);
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_fstat(os_file_handle handle, struct __wasi_filestat_t *buf)
{
    int slot;

    if (!buf) {
        return __WASI_EINVAL;
    }
    if (stick_handle_is_dir(handle)) {
        stick_fill_stat(stick_handle_id(handle), buf);
        return __WASI_ESUCCESS;
    }
    slot = stick_handle_file_slot(handle);
    if (slot >= 0 && stick_files[slot].used) {
        stick_fill_stat(stick_files[slot].vfs_id, buf);
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_fstatat(os_file_handle handle, const char *path,
           struct __wasi_filestat_t *buf, __wasi_lookupflags_t flags)
{
    uint32_t id;

    (void)flags;
    if (!stick_handle_is_dir(handle)) {
        return __WASI_EBADF;
    }
    if (!stick_resolve_at(stick_handle_id(handle), path, &id)) {
        return __WASI_ENOENT;
    }
    stick_fill_stat(id, buf);
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_file_get_fdflags(os_file_handle handle, __wasi_fdflags_t *flags)
{
    if (!flags) {
        return __WASI_EINVAL;
    }
    if (stick_is_managed_handle(handle)) {
        *flags = 0;
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_file_set_fdflags(os_file_handle handle, __wasi_fdflags_t flags)
{
    if (stick_is_managed_handle(handle)) {
        (void)flags;
        return __WASI_ESUCCESS;
    }
    return __WASI_ENOTSUP;
}

__wasi_errno_t
os_fdatasync(os_file_handle handle)
{
    (void)handle;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_fsync(os_file_handle handle)
{
    (void)handle;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_openat(os_file_handle handle, const char *path, __wasi_oflags_t oflags,
          __wasi_fdflags_t fd_flags, __wasi_lookupflags_t lookup_flags,
          wasi_libc_file_access_mode access_mode, os_file_handle *out)
{
    uint32_t id;
    int slot;
    bool want_dir = (oflags & __WASI_O_DIRECTORY) != 0;

    (void)fd_flags;
    (void)lookup_flags;
    (void)access_mode;
    if (!out || !stick_handle_is_dir(handle)) {
        return __WASI_EINVAL;
    }
    if (!stick_resolve_at(stick_handle_id(handle), path, &id)) {
        return __WASI_ENOENT;
    }
    /* O_CREAT / O_EXCL / O_TRUNC are accepted but not implemented. */
    (void)oflags;
    if (stick_vfs_is_dir(id)) {
        if (want_dir) {
            *out = stick_dir_handle(id);
            return __WASI_ESUCCESS;
        }
        /* TinyGo opens "." for ReadDir without O_DIRECTORY. */
        *out = stick_dir_handle(id);
        return __WASI_ESUCCESS;
    }
    if (want_dir) {
        return __WASI_ENOTDIR;
    }
    slot = stick_file_slot_alloc(id);
    if (slot < 0) {
        return __WASI_ENOMEM;
    }
    *out = stick_file_handle(slot);
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_file_get_access_mode(os_file_handle handle,
                        wasi_libc_file_access_mode *access_mode)
{
    if (!access_mode) {
        return __WASI_EINVAL;
    }
    if (stick_is_managed_handle(handle)) {
        *access_mode = WASI_LIBC_ACCESS_MODE_READ_WRITE;
        return __WASI_ESUCCESS;
    }
    return __WASI_EBADF;
}

__wasi_errno_t
os_close(os_file_handle handle, bool is_stdio)
{
    int slot;

    (void)is_stdio;
    slot = stick_handle_file_slot(handle);
    if (slot >= 0) {
        stick_file_slot_free(slot);
    }
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_preadv(os_file_handle handle, const struct __wasi_iovec_t *iov, int iovcnt,
          __wasi_filesize_t offset, size_t *nread)
{
    int slot;

    slot = stick_handle_file_slot(handle);
    if (slot < 0 || !stick_files[slot].used) {
        return __WASI_EBADF;
    }
    return stick_read_into_iov(stick_files[slot].vfs_id, offset, iov, iovcnt, nread);
}

__wasi_errno_t
os_pwritev(os_file_handle handle, const struct __wasi_ciovec_t *iov, int iovcnt,
           __wasi_filesize_t offset, size_t *nwritten)
{
    if (os_is_stdout_handle(handle) || os_is_stderr_handle(handle)) {
        (void)offset;
        return stick_stream_writev(iov, iovcnt, nwritten);
    }
    int slot;
    uint8_t stack_buf[512];
    size_t cap;
    uint8_t *tmp = stack_buf;
    size_t written = 0;
    int i;

    if (!nwritten) {
        return __WASI_EINVAL;
    }
    if (stick_task_should_terminate()) {
        return __WASI_EINTR;
    }
    slot = stick_handle_file_slot(handle);
    if (slot < 0 || !stick_files[slot].used) {
        return __WASI_EBADF;
    }
    cap = stick_ciov_total(iov, iovcnt);
    if (cap == 0) {
        *nwritten = 0;
        return __WASI_ESUCCESS;
    }
    if (cap > sizeof(stack_buf)) {
        tmp = (uint8_t *)BH_MALLOC(cap);
        if (!tmp) {
            return __WASI_ENOMEM;
        }
    }
    for (i = 0; i < iovcnt; i++) {
        if (iov[i].buf_len > 0) {
            memcpy(tmp + written, iov[i].buf, iov[i].buf_len);
            written += iov[i].buf_len;
        }
    }
    if (stick_vfs_write(stick_files[slot].vfs_id, offset, tmp, written) < 0) {
        if (tmp != stack_buf) {
            BH_FREE(tmp);
        }
        return __WASI_EIO;
    }
    if (tmp != stack_buf) {
        BH_FREE(tmp);
    }
    *nwritten = written;
    return __WASI_ESUCCESS;
}

__wasi_errno_t
os_readv(os_file_handle handle, const struct __wasi_iovec_t *iov, int iovcnt,
         size_t *nread)
{
    int slot;
    __wasi_errno_t err;
    uint64_t offset;

    slot = stick_handle_file_slot(handle);
    if (slot < 0 || !stick_files[slot].used) {
        return __WASI_EBADF;
    }
    offset = stick_files[slot].offset;
    err = stick_read_into_iov(stick_files[slot].vfs_id, offset, iov, iovcnt, nread);
    if (err == __WASI_ESUCCESS && nread) {
        stick_files[slot].offset += *nread;
    }
    return err;
}

__wasi_errno_t
os_writev(os_file_handle handle, const struct __wasi_ciovec_t *iov, int iovcnt,
          size_t *nwritten)
{
    int slot;
    __wasi_errno_t err;
    uint64_t offset;

    if (os_is_stdout_handle(handle) || os_is_stderr_handle(handle)) {
        return stick_stream_writev(iov, iovcnt, nwritten);
    }
    slot = stick_handle_file_slot(handle);
    if (slot < 0 || !stick_files[slot].used) {
        return __WASI_EBADF;
    }
    offset = stick_files[slot].offset;
    err = os_pwritev(handle, iov, iovcnt, offset, nwritten);
    if (err == __WASI_ESUCCESS && nwritten) {
        stick_files[slot].offset += *nwritten;
    }
    return err;
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
    if (nread) {
        *nread = 0;
    }
    /* No symlinks on stick — path_get treats EINVAL as "not a symlink". */
    return __WASI_EINVAL;
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
    int slot;
    uint64_t base;
    int64_t delta = (int64_t)offset;

    if (!new_offset) {
        return __WASI_EINVAL;
    }
    slot = stick_handle_file_slot(handle);
    if (slot < 0 || !stick_files[slot].used) {
        return __WASI_EBADF;
    }
    base = stick_files[slot].offset;
    switch (whence) {
        case __WASI_WHENCE_SET:
            if (delta < 0) {
                return __WASI_EINVAL;
            }
            base = (uint64_t)delta;
            break;
        case __WASI_WHENCE_CUR:
            if (delta < 0 && (uint64_t)(-delta) > base) {
                return __WASI_EINVAL;
            }
            base = (uint64_t)((int64_t)base + delta);
            break;
        case __WASI_WHENCE_END:
        {
            uint64_t end = stick_vfs_length(stick_files[slot].vfs_id);
            if (delta < 0 && (uint64_t)(-delta) > end) {
                return __WASI_EINVAL;
            }
            base = (uint64_t)((int64_t)end + delta);
            break;
        }
        default:
            return __WASI_EINVAL;
    }
    stick_files[slot].offset = base;
    *new_offset = base;
    return __WASI_ESUCCESS;
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

    if (!dir_stream || !stick_handle_is_dir(handle)) {
        return __WASI_EBADF;
    }
    dir = (stick_dir_t *)BH_MALLOC(sizeof(*dir));
    if (!dir) {
        return __WASI_ENOMEM;
    }
    dir->active = true;
    dir->vfs_id = stick_handle_id(handle);
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
    int child_id;
    int namelen;

    if (!dir || !entry || !d_name) {
        return __WASI_EINVAL;
    }
    child_id = stick_vfs_child_at(dir->vfs_id, (uint32_t)dir->cookie);
    if (child_id < 0) {
        *d_name = NULL;
        memset(entry, 0, sizeof(*entry));
        return __WASI_ESUCCESS;
    }
    namelen = stick_vfs_name((uint32_t)child_id, stick_dirent_name,
                             sizeof(stick_dirent_name));
    if (namelen < 0) {
        *d_name = NULL;
        return __WASI_EIO;
    }
    memset(entry, 0, sizeof(*entry));
    entry->d_ino = (uint64_t)child_id;
    entry->d_namlen = (uint32_t)namelen;
    entry->d_type = stick_vfs_is_dir((uint32_t)child_id)
                          ? __WASI_FILETYPE_DIRECTORY
                          : __WASI_FILETYPE_REGULAR_FILE;
    dir->cookie++;
    entry->d_next = dir->cookie;
    *d_name = stick_dirent_name;
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
