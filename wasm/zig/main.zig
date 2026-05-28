const std = @import("std");
const wasi = std.os.wasi;

// FBA backing lives in .bss (linear memory data section), not on the wasm
// stack. With `--stack 65536` (see Makefile) we only have a 64 KiB wasm
// stack budget; a 64 KiB local array here would underflow the stack pointer
// past __data_end on entry to main() and WAMR would trap with
// "Exception: out of bounds memory access".
var fba_backing: [65536]u8 = undefined;

// Write bytes to a file descriptor using the WASI fd_write syscall.
fn fdWrite(fd: wasi.fd_t, str: []const u8) void {
    const iov = [1]wasi.ciovec_t{.{ .base = str.ptr, .len = str.len }};
    var nwritten: usize = undefined;
    _ = wasi.fd_write(fd, &iov, 1, &nwritten);
}

// Scratch buffer used by `print`. Also `.bss` so per-call stack frames stay
// small (the Args/Env/Root loops below call `print` once per entry).
var print_buf: [4096]u8 = undefined;

// Scratch for `fd_readdir`; 2 KiB is roughly one page of entries.
var dirent_scratch: [2048]u8 = undefined;

// Write formatted text to stdout.
fn print(comptime fmt: []const u8, args: anytype) void {
    const s = std.fmt.bufPrint(&print_buf, fmt, args) catch return;
    fdWrite(1, s);
}

pub fn main() !void {
    var fba = std.heap.FixedBufferAllocator.init(&fba_backing);
    const allocator = fba.allocator();

    // Print current directory (not available without Io object)
    fdWrite(1, "Dir: n/a\n");

    // Print arguments via WASI args_get
    fdWrite(1, "Args:");
    {
        var count: usize = undefined;
        var buf_size: usize = undefined;
        switch (wasi.args_sizes_get(&count, &buf_size)) {
            .SUCCESS => {},
            else => count = 0,
        }
        if (count > 0) {
            const buf = try allocator.alloc(u8, buf_size);
            defer allocator.free(buf);
            const ptrs = try allocator.alloc([*:0]u8, count);
            defer allocator.free(ptrs);
            switch (wasi.args_get(ptrs.ptr, buf.ptr)) {
                .SUCCESS => {},
                else => {},
            }
            for (ptrs) |ptr| {
                const arg = std.mem.sliceTo(ptr, 0);
                print(" {s}", .{arg});
            }
        }
    }
    fdWrite(1, "\n");

    // Print environment variables via WASI environ_get
    fdWrite(1, "Env:\n");
    {
        var count: usize = undefined;
        var buf_size: usize = undefined;
        switch (wasi.environ_sizes_get(&count, &buf_size)) {
            .SUCCESS => {},
            else => count = 0,
        }
        if (count > 0) {
            const buf = try allocator.alloc(u8, buf_size);
            defer allocator.free(buf);
            const ptrs = try allocator.alloc([*:0]u8, count);
            defer allocator.free(ptrs);
            switch (wasi.environ_get(ptrs.ptr, buf.ptr)) {
                .SUCCESS => {},
                else => {},
            }
            for (ptrs) |ptr| {
                const entry = std.mem.sliceTo(ptr, 0);
                print(" {s}\n", .{entry});
            }
        }
    }
    fdWrite(1, "\n");

    // Print root directory contents by reading fd 3 (first preopen, typically "/")
    fdWrite(1, "Root:");
    {
        // `dirent_scratch` (module-level `.bss`) keeps main()'s wasm-stack
        // frame small enough to fit under the --stack budget.
        var cookie: wasi.dircookie_t = wasi.DIRCOOKIE_START;
        while (true) {
            var nread: usize = undefined;
            const rc = wasi.fd_readdir(3, &dirent_scratch, dirent_scratch.len, cookie, &nread);
            if (rc != .SUCCESS or nread == 0) break;
            var offset: usize = 0;
            while (offset + @sizeOf(wasi.dirent_t) <= nread) {
                const dirent: *const wasi.dirent_t = @ptrCast(@alignCast(&dirent_scratch[offset]));
                const name_start = offset + @sizeOf(wasi.dirent_t);
                const name_len = dirent.namlen;
                if (name_start + name_len > nread) break;
                const name = dirent_scratch[name_start .. name_start + name_len];
                if (dirent.type == .DIRECTORY) {
                    print(" {s}/", .{name});
                } else {
                    print(" {s}", .{name});
                }
                cookie = dirent.next;
                offset = name_start + name_len;
            }
            if (nread < dirent_scratch.len) break;
        }
    }
    fdWrite(1, "\n");
}
