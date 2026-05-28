//! WAMR runner driven by `/ctl exec <path>`.
//!
//! The wasm bytes live in the `/tmp` ramfs ([`crate::memfs`]); the root `/ctl`
//! handler calls [`exec`] with a path like `/tmp/zigcheck.wasm`, the core-1
//! worker ([`run_pending`]) reads those bytes, invokes WAMR, and writes
//! captured stdout (or an error message) to `/tmp/exec.log`.

extern crate alloc;

use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU8, Ordering};
use critical_section::Mutex;
use heapless::String;

use crate::memfs;

/// Captured-stdout buffer length (matches `CAPTURE_MAX` in `stick_wamr.c`).
const OUT_CAP: usize = 8192;
/// `/tmp` filename that receives stdout / error after each run.
const LOG_NAME: &str = "exec.log";

/// WAMR interpreter pool size ([`wamr_sys::RUNTIME_HEAP_BYTES`]).
pub const POOL_BYTES: usize = wamr_sys::RUNTIME_HEAP_BYTES;

const ST_IDLE: u8 = 0;
const ST_PENDING: u8 = 1;
const ST_RUNNING: u8 = 2;
const ST_DONE: u8 = 3;
const ST_FAILED: u8 = 4;

/// argv passed to the guest (`args_get`). argv[0] is replaced with the wasm
/// basename on each exec; the remaining slots are static defaults.
static GUEST_EXTRA_ARGV: &[&str] = &["m5stick"];
/// env pairs passed to the guest (`environ_get`).
static GUEST_ENV: &[&str] = &[
    "PATH=/",
    "HOME=/",
    "USER=stick",
    "ZIG_TARGET=m5stick",
];

static RUN_STATE: AtomicU8 = AtomicU8::new(ST_IDLE);

struct Job {
    /// memfs inode of the wasm file to execute.
    wasm_ino: u16,
    /// argv[0] for the guest — wasm basename, NUL-trimmed.
    basename: String<32>,
    /// Trailing error message after a failed run (truncated).
    err: String<128>,
}

impl Job {
    const INIT: Self = Self {
        wasm_ino: 0,
        basename: String::new(),
        err: String::new(),
    };
}

static JOB: Mutex<RefCell<Job>> = Mutex::new(RefCell::new(Job::INIT));
static RUNTIME_HEAP: Mutex<RefCell<Option<Vec<u8>>>> = Mutex::new(RefCell::new(None));

fn load_state() -> u8 {
    RUN_STATE.load(Ordering::Acquire)
}

fn store_state(state: u8) {
    RUN_STATE.store(state, Ordering::Release);
}

/// True while a guest is running on core 1 (9P should defer the Rwrite reply).
pub fn is_busy() -> bool {
    matches!(load_state(), ST_PENDING | ST_RUNNING)
}

/// Status line for `cat /ctl` — one of `idle`, `running <basename>`,
/// `done <basename>`, `failed <basename>: <msg>`.
pub fn status_line() -> String<200> {
    let mut s = String::new();
    let st = load_state();
    let _ = s.push_str("wasm ");
    match st {
        ST_IDLE => {
            let _ = s.push_str("idle\n");
        }
        ST_PENDING | ST_RUNNING => {
            let _ = s.push_str("running ");
            critical_section::with(|cs| {
                let job = JOB.borrow(cs).borrow();
                let _ = s.push_str(job.basename.as_str());
            });
            let _ = s.push('\n');
        }
        ST_DONE => {
            let _ = s.push_str("done ");
            critical_section::with(|cs| {
                let job = JOB.borrow(cs).borrow();
                let _ = s.push_str(job.basename.as_str());
            });
            let _ = s.push('\n');
        }
        ST_FAILED => {
            let _ = s.push_str("failed ");
            critical_section::with(|cs| {
                let job = JOB.borrow(cs).borrow();
                let _ = s.push_str(job.basename.as_str());
                let _ = s.push_str(": ");
                let _ = s.push_str(job.err.as_str());
            });
            let _ = s.push('\n');
        }
        _ => {
            let _ = s.push_str("idle\n");
        }
    }
    s
}

/// Queue an exec of `path` (must resolve under `/tmp`). Returns once the
/// worker has accepted the job; the caller is expected to poll [`is_busy`]
/// and read `/tmp/exec.log` for output.
pub fn exec(path: &str) -> Result<(), &'static str> {
    if is_busy() {
        return Err("wasm: already running");
    }
    if !memfs::is_ready() {
        return Err("tmp not available");
    }
    let (ino, base) = resolve_tmp_file(path)?;
    let len = memfs::length(ino);
    if len == 0 {
        return Err("wasm: file is empty");
    }
    if len > u32::MAX as u64 {
        return Err("wasm: file too large");
    }

    critical_section::with(|cs| {
        let mut job = JOB.borrow(cs).borrow_mut();
        job.wasm_ino = ino;
        job.basename.clear();
        let _ = job.basename.push_str(base);
        job.err.clear();
    });
    let _ = path; // surfaced via the user's own ctl write log; basename is the public handle
    store_state(ST_PENDING);
    Ok(())
}

fn resolve_tmp_file(path: &str) -> Result<(u16, &str), &'static str> {
    let rest = path
        .strip_prefix("/tmp/")
        .ok_or("wasm: path must start with /tmp/")?;
    if rest.is_empty() {
        return Err("wasm: empty path");
    }
    let mut cur = memfs::ROOT_INO;
    let mut last_component = "";
    for part in rest.split('/') {
        if part.is_empty() {
            return Err("wasm: empty component");
        }
        let next = memfs::walk(cur, part).ok_or("wasm: not found")?;
        cur = next;
        last_component = part;
    }
    if memfs::is_dir(cur) {
        return Err("wasm: is a directory");
    }
    let base = last_component
        .strip_suffix(".wasm")
        .unwrap_or(last_component);
    Ok((cur, base))
}

/// Core-1 worker entry: run guest if armed. Returns true when work was done.
pub fn run_pending() -> bool {
    if load_state() != ST_PENDING {
        return false;
    }
    store_state(ST_RUNNING);

    let (wasm_ino, basename) = critical_section::with(|cs| {
        let job = JOB.borrow(cs).borrow();
        (job.wasm_ino, job.basename.clone())
    });

    let wasm_bytes = match read_wasm(wasm_ino) {
        Ok(v) => v,
        Err(msg) => {
            finish_failure(msg);
            return true;
        }
    };

    if let Err(msg) = ensure_runtime_heap() {
        finish_failure(msg);
        return true;
    }
    if wamr_sys::init_runtime().is_err() {
        finish_failure("wamr init failed");
        return true;
    }

    // argv[0] is the wasm basename; tack on the static defaults so guests get
    // a familiar `<prog> m5stick` invocation regardless of which file ran.
    let mut argv_buf: heapless::Vec<&str, 8> = heapless::Vec::new();
    let _ = argv_buf.push(basename.as_str());
    for s in GUEST_EXTRA_ARGV {
        let _ = argv_buf.push(*s);
    }

    let mut err = [0u8; 256];
    match wamr_sys::run(&wasm_bytes, argv_buf.as_slice(), GUEST_ENV, &mut err) {
        Ok(stdout) => finish_success(stdout),
        Err(()) => {
            let msg = core::str::from_utf8(&err)
                .unwrap_or("wasm failed")
                .trim_end_matches('\0');
            finish_failure_owned(msg);
        }
    }
    true
}

fn finish_success(stdout: &str) {
    let mut buf: heapless::Vec<u8, OUT_CAP> = heapless::Vec::new();
    let bytes = stdout.as_bytes();
    let take = bytes.len().min(buf.capacity());
    let _ = buf.extend_from_slice(&bytes[..take]);
    if !stdout.ends_with('\n') && buf.len() < buf.capacity() {
        let _ = buf.push(b'\n');
    }
    write_log(&buf);
    store_state(ST_DONE);
}

fn finish_failure(msg: &'static str) {
    finish_failure_owned(msg);
}

fn finish_failure_owned(msg: &str) {
    critical_section::with(|cs| {
        let mut job = JOB.borrow(cs).borrow_mut();
        job.err.clear();
        let _ = job.err.push_str(msg);
    });
    let mut buf: heapless::Vec<u8, OUT_CAP> = heapless::Vec::new();
    let _ = buf.extend_from_slice(b"wasm error: ");
    let take = msg.as_bytes().len().min(buf.capacity() - buf.len());
    let _ = buf.extend_from_slice(&msg.as_bytes()[..take]);
    if buf.len() < buf.capacity() {
        let _ = buf.push(b'\n');
    }
    write_log(&buf);
    store_state(ST_FAILED);
}

fn read_wasm(ino: u16) -> Result<Vec<u8>, &'static str> {
    let len = memfs::length(ino) as usize;
    let mut out = Vec::new();
    if out.try_reserve_exact(len).is_err() {
        return Err("wasm: out of memory loading file");
    }
    out.resize(len, 0);
    // Chunk reads so the critical_section inside memfs::read stays short.
    const CHUNK: usize = 4096;
    let mut off = 0usize;
    while off < len {
        let end = (off + CHUNK).min(len);
        let got = memfs::read(ino, off as u64, &mut out[off..end]);
        if got == 0 {
            return Err("wasm: short read");
        }
        off += got;
    }
    Ok(out)
}

fn write_log(data: &[u8]) {
    let ino = match ensure_log_inode() {
        Ok(i) => i,
        Err(_) => return,
    };
    memfs::truncate(ino);
    let mut off = 0usize;
    while off < data.len() {
        match memfs::write(ino, off as u64, &data[off..]) {
            Ok(0) => break,
            Ok(n) => off += n,
            Err(_) => break,
        }
    }
}

fn ensure_log_inode() -> Result<u16, &'static str> {
    if let Some(ino) = memfs::walk(memfs::ROOT_INO, LOG_NAME) {
        return Ok(ino);
    }
    memfs::create(memfs::ROOT_INO, LOG_NAME, 0o100_666)
}

/// Reserve the WAMR pool once PSRAM is up (call from firmware boot).
///
/// WAMR runtime init runs on AppCpu in [`run_pending`] (same core as `stick_wamr_run`).
pub fn preinit_runtime_heap() -> Result<(), &'static str> {
    ensure_runtime_heap()
}

fn heap_is_ready() -> bool {
    critical_section::with(|cs| RUNTIME_HEAP.borrow(cs).borrow().is_some())
}

fn ensure_runtime_heap() -> Result<(), &'static str> {
    if heap_is_ready() {
        return Ok(());
    }
    let mut heap = Vec::new();
    if heap
        .try_reserve_exact(wamr_sys::RUNTIME_HEAP_BYTES)
        .is_err()
    {
        return Err("out of memory for wasm pool (see /sys/heap psram free=)");
    }
    // Do not zero or register inside `critical_section` — a multi-MiB `resize`
    // would mask interrupts on both CPUs and starve I²S DMA on core 0.
    heap.resize(wamr_sys::RUNTIME_HEAP_BYTES, 0);
    wamr_sys::set_runtime_heap(heap.as_mut_ptr(), heap.len());
    core::sync::atomic::fence(Ordering::SeqCst);
    critical_section::with(|cs| {
        *RUNTIME_HEAP.borrow(cs).borrow_mut() = Some(heap);
    });
    Ok(())
}

/// Reset cached state (for testing / future ctl).
pub fn reset() {
    store_state(ST_IDLE);
    critical_section::with(|cs| {
        let mut job = JOB.borrow(cs).borrow_mut();
        job.basename.clear();
        job.err.clear();
    });
}
