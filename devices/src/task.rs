//! WASM task slots exposed as `/task/<rid>/…` (configure + `ctl start`).
//!
//! [`exec_configure`] (from `/ctl exec …`) allocates a task and sets `cmd` without
//! starting. The core-1 worker runs [`run_pending`] when `ctl` receives `start`.

extern crate alloc;

use alloc::vec::Vec;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU8, Ordering};
use critical_section::Mutex;
use heapless::String;

use crate::memfs;

pub const POOL_BYTES: usize = wamr_sys::RUNTIME_HEAP_BYTES;
const DATA_CAP: usize = 8192;
const CMD_CAP: usize = 256;
const ENV_CAP: usize = 512;
const DIR_CAP: usize = 64;
const MAX_TASKS: usize = 8;
const MAX_ARGV: usize = 16;
const MAX_ENV_LINES: usize = 16;
const ENV_LINE_CAP: usize = 64;

const PH_FREE: u8 = 0;
const PH_CONFIGURED: u8 = 1;
const PH_PENDING: u8 = 2;
const PH_RUNNING: u8 = 3;
const PH_EXITED: u8 = 4;

struct Task {
    phase: u8,
    started: bool,
    cmd: String<CMD_CAP>,
    env: String<ENV_CAP>,
    dir: String<DIR_CAP>,
    exit_code: i32,
    basename: String<32>,
    /// Allocated from PSRAM on `start` — not kept in `.bss` (8 slots × 16 KiB overflowed SRAM).
    data_out: Option<Vec<u8>>,
    data_in: Option<Vec<u8>>,
}

/// Parsed argv/env for the one pending/running guest (not duplicated per slot).
struct RunnerScratch {
    wasm_ino: u16,
    argv: heapless::Vec<String<64>, MAX_ARGV>,
    env_lines: heapless::Vec<String<ENV_LINE_CAP>, MAX_ENV_LINES>,
}

impl RunnerScratch {
    const INIT: Self = Self {
        wasm_ino: 0,
        argv: heapless::Vec::new(),
        env_lines: heapless::Vec::new(),
    };
}

impl Task {
    const INIT: Self = Self {
        phase: PH_FREE,
        started: false,
        cmd: String::new(),
        env: String::new(),
        dir: String::new(),
        exit_code: 0,
        basename: String::new(),
        data_out: None,
        data_in: None,
    };
}

static RUNNER_SCRATCH: Mutex<RefCell<RunnerScratch>> =
    Mutex::new(RefCell::new(RunnerScratch::INIT));

static TASKS: Mutex<RefCell<[Task; MAX_TASKS]>> =
    Mutex::new(RefCell::new([Task::INIT; MAX_TASKS]));
static RUNNER: AtomicU8 = AtomicU8::new(0);
static RUNTIME_HEAP: Mutex<RefCell<Option<Vec<u8>>>> = Mutex::new(RefCell::new(None));

fn rid_to_idx(rid: u8) -> Option<usize> {
    if rid == 0 || rid as usize > MAX_TASKS {
        None
    } else {
        Some(rid as usize - 1)
    }
}

fn runner_rid() -> u8 {
    RUNNER.load(Ordering::Acquire)
}

fn set_runner(rid: u8) {
    RUNNER.store(rid, Ordering::Release);
}

pub fn exists(rid: u8) -> bool {
    let Some(idx) = rid_to_idx(rid) else {
        return false;
    };
    critical_section::with(|cs| TASKS.borrow(cs).borrow()[idx].phase != PH_FREE)
}

pub fn list_rids(out: &mut heapless::Vec<u8, 32>) {
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        for (i, t) in tasks.iter().enumerate() {
            if t.phase != PH_FREE {
                let rid = (i + 1) as u8;
                let _ = out.push(rid);
            }
        }
    });
}

/// Read `/task/alloc` at offset 0 — allocates a slot and returns `"<rid>\n"`.
pub fn alloc_read(off: u64, buf: &mut [u8]) -> usize {
    if off != 0 {
        return 0;
    }
    let rid = match alloc_slot() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let mut line = heapless::String::<8>::new();
    let _ = push_u8(&mut line, rid);
    let _ = line.push('\n');
    copy_bytes(line.as_bytes(), 0, buf)
}

fn alloc_slot() -> Result<u8, &'static str> {
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        for (i, t) in tasks.iter_mut().enumerate() {
            if t.phase == PH_FREE || t.phase == PH_EXITED {
                *t = Task::INIT;
                t.phase = PH_CONFIGURED;
                let _ = t.dir.push_str(".");
                return Ok((i + 1) as u8);
            }
        }
        Err("task: no free slots")
    })
}

/// `/ctl exec …` — allocate and set `cmd` (does not start).
pub fn exec_configure(line: &str) -> Result<(), &'static str> {
    let rid = alloc_slot()?;
    set_cmd(rid, line)?;
    Ok(())
}

pub fn read_id(rid: u8, off: u64, buf: &mut [u8]) -> usize {
    if !exists(rid) {
        return 0;
    }
    let mut line = heapless::String::<8>::new();
    let _ = push_u8(&mut line, rid);
    let _ = line.push('\n');
    copy_bytes(line.as_bytes(), off, buf)
}

pub fn read_exit(rid: u8, off: u64, buf: &mut [u8]) -> usize {
    let line = critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let Some(idx) = rid_to_idx(rid) else {
            return heapless::String::<16>::new();
        };
        let t = &tasks[idx];
        if t.phase != PH_EXITED {
            return heapless::String::<16>::new();
        }
        exit_line(t.exit_code)
    });
    copy_bytes(line.as_bytes(), off, buf)
}

fn exit_line(code: i32) -> heapless::String<16> {
    let mut s = heapless::String::<16>::new();
    if code < 0 {
        let _ = s.push_str("-1");
    } else {
        let _ = push_i32(&mut s, code);
    }
    let _ = s.push('\n');
    s
}

pub fn read_field(rid: u8, field: TaskField, off: u64, buf: &mut [u8]) -> usize {
    if !exists(rid) {
        return 0;
    }
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let Some(idx) = rid_to_idx(rid) else {
            return 0;
        };
        let t = &tasks[idx];
        match field {
            TaskField::Cmd => copy_field_with_nl(t.cmd.as_str(), off, buf),
            TaskField::Env => copy_field_with_nl(t.env.as_str(), off, buf),
            TaskField::Dir => copy_field_with_nl(t.dir.as_str(), off, buf),
        }
    })
}

pub fn field_len(rid: u8, field: TaskField) -> u64 {
    if !exists(rid) {
        return 0;
    }
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let Some(idx) = rid_to_idx(rid) else {
            return 0;
        };
        let t = &tasks[idx];
        let n = match field {
            TaskField::Cmd => t.cmd.len(),
            TaskField::Env => t.env.len(),
            TaskField::Dir => t.dir.len(),
        };
        (n + 1) as u64
    })
}

pub fn data_out_len(rid: u8) -> u64 {
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        rid_to_idx(rid)
            .and_then(|idx| tasks[idx].data_out.as_ref())
            .map(|data| data.len() as u64)
            .unwrap_or(0)
    })
}

pub fn read_data(rid: u8, off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let Some(idx) = rid_to_idx(rid) else {
            return 0;
        };
        let Some(data) = &tasks[idx].data_out else {
            return 0;
        };
        copy_bytes(data.as_slice(), off, buf)
    })
}

pub fn write_field(
    rid: u8,
    field: TaskField,
    off: u64,
    data: &[u8],
) -> Result<usize, &'static str> {
    if !exists(rid) {
        return Err("task: not found");
    }
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let Some(idx) = rid_to_idx(rid) else {
            return Err("task: not found");
        };
        let t = &mut tasks[idx];
        if t.started {
            return Err("task: already started");
        }
        if t.phase == PH_EXITED {
            return Err("task: exited");
        }
        match field {
            TaskField::Cmd => write_replace(&mut t.cmd, off, data),
            TaskField::Env => write_replace(&mut t.env, off, data),
            TaskField::Dir => write_replace(&mut t.dir, off, data),
        }
    })
}

fn write_replace<const N: usize>(
    dst: &mut String<N>,
    off: u64,
    data: &[u8],
) -> Result<usize, &'static str> {
    if off != 0 {
        return Err("task: write at offset 0 only");
    }
    dst.clear();
    let s = core::str::from_utf8(data).map_err(|_| "bad utf8")?;
    let s = s.trim_end_matches('\0').trim_end_matches('\r').trim_end_matches('\n');
    if dst.push_str(s).is_err() {
        return Err("task: field too long");
    }
    Ok(data.len())
}

fn record_start_error(rid: u8, msg: &'static str) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let Some(idx) = rid_to_idx(rid) else {
            return Err("task: not found");
        };
        let t = &mut tasks[idx];
        if t.started {
            return Ok(());
        }
        t.started = true;
        t.phase = PH_EXITED;
        t.exit_code = -1;
        let mut out = new_data_buf().unwrap_or_default();
        let mut line = heapless::String::<256>::new();
        let _ = line.push_str(msg);
        if !line.ends_with('\n') {
            let _ = line.push('\n');
        }
        append_out(&mut out, line.as_bytes());
        t.data_out = Some(out);
        Ok(())
    })
}

pub fn write_ctl(rid: u8, line: &str) -> Result<(), &'static str> {
    let cmd = line.trim();
    if cmd != "start" {
        return Err("task: bad ctl");
    }
    if let Err(e) = start(rid) {
        record_start_error(rid, e)?;
    }
    Ok(())
}

pub fn write_data(rid: u8, off: u64, data: &[u8]) -> Result<usize, &'static str> {
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let Some(idx) = rid_to_idx(rid) else {
            return Err("task: not found");
        };
        let t = &mut tasks[idx];
        if t.phase == PH_EXITED {
            return Err("task: exited");
        }
        if t.phase != PH_RUNNING && t.phase != PH_PENDING {
            return Err("task: not running");
        }
        let Some(data_in) = &mut t.data_in else {
            return Err("task: not running");
        };
        if off as usize + data.len() > DATA_CAP {
            return Err("task: stdin full");
        }
        if data_in.len() < off as usize + data.len() {
            if data_in.try_reserve(off as usize + data.len()).is_err() {
                return Err("task: stdin full");
            }
            data_in.resize(off as usize + data.len(), 0);
        }
        data_in[off as usize..off as usize + data.len()].copy_from_slice(data);
        Ok(data.len())
    })
}

fn set_cmd(rid: u8, line: &str) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let Some(idx) = rid_to_idx(rid) else {
            return Err("task: not found");
        };
        let t = &mut tasks[idx];
        if t.started {
            return Err("task: already started");
        }
        t.cmd.clear();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err("task: empty cmd");
        }
        if t.cmd.push_str(trimmed).is_err() {
            return Err("task: cmd too long");
        }
        Ok(())
    })
}

fn start(rid: u8) -> Result<(), &'static str> {
    if runner_rid() != 0 {
        return Err("task: runner busy");
    }
    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let Some(idx) = rid_to_idx(rid) else {
            return Err("task: not found");
        };
        let t = &mut tasks[idx];
        if t.phase != PH_CONFIGURED {
            return Err("task: not configured");
        }
        if t.started {
            return Err("task: already started");
        }
        if t.cmd.is_empty() {
            return Err("task: empty cmd");
        }
        parse_cmd(t)?;
        parse_env(t)?;
        t.data_out = Some(new_data_buf()?);
        t.data_in = Some(Vec::new());
        t.started = true;
        t.phase = PH_PENDING;
        set_runner(rid);
        Ok(())
    })
}

fn parse_cmd(t: &mut Task) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut scratch = RUNNER_SCRATCH.borrow(cs).borrow_mut();
        scratch.argv.clear();
        t.basename.clear();
        let tokens = split_cmdline(t.cmd.as_str())?;
        if tokens.is_empty() {
            return Err("task: empty cmd");
        }
        let path = tokens[0].as_str();
        let (ino, base) = resolve_tmp_file(path)?;
        scratch.wasm_ino = ino;
        let _ = t.basename.push_str(base);
        let mut arg0 = String::<64>::new();
        let _ = arg0.push_str(t.basename.as_str());
        let _ = scratch.argv.push(arg0);
        for tok in tokens.iter().skip(1) {
            if scratch.argv.push(tok.clone()).is_err() {
                return Err("task: too many args");
            }
        }
        Ok(())
    })
}

fn parse_env(t: &Task) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut scratch = RUNNER_SCRATCH.borrow(cs).borrow_mut();
        scratch.env_lines.clear();
        if t.env.is_empty() {
            return Ok(());
        }
        for line in t.env.as_str().split('\n') {
            let line = line.trim_end_matches('\r').trim();
            if line.is_empty() {
                continue;
            }
            let mut s = String::<ENV_LINE_CAP>::new();
            if s.push_str(line).is_err() {
                return Err("task: env line too long");
            }
            if scratch.env_lines.push(s).is_err() {
                return Err("task: too many env lines");
            }
        }
        Ok(())
    })
}

fn resolve_tmp_file(path: &str) -> Result<(u16, &str), &'static str> {
    let rest = path
        .strip_prefix("/tmp/")
        .ok_or("task: path must start with /tmp/")?;
    if rest.is_empty() {
        return Err("task: empty path");
    }
    let mut cur = memfs::ROOT_INO;
    let mut last_component = "";
    for part in rest.split('/') {
        if part.is_empty() {
            return Err("task: empty component");
        }
        let next = memfs::walk(cur, part).ok_or("task: not found")?;
        cur = next;
        last_component = part;
    }
    if memfs::is_dir(cur) {
        return Err("task: is a directory");
    }
    let base = last_component
        .strip_suffix(".wasm")
        .unwrap_or(last_component);
    Ok((cur, base))
}

fn split_cmdline(s: &str) -> Result<heapless::Vec<String<64>, MAX_ARGV>, &'static str> {
    let mut out = heapless::Vec::<String<64>, MAX_ARGV>::new();
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let mut tok = String::<64>::new();
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if tok.push(b[i] as char).is_err() {
                    return Err("task: arg too long");
                }
                i += 1;
            }
            if i < b.len() {
                i += 1;
            }
        } else {
            while i < b.len() && b[i] != b' ' {
                if tok.push(b[i] as char).is_err() {
                    return Err("task: arg too long");
                }
                i += 1;
            }
        }
        if tok.is_empty() {
            continue;
        }
        if out.push(tok).is_err() {
            return Err("task: too many args");
        }
    }
    Ok(out)
}

pub fn is_busy() -> bool {
    matches!(runner_rid(), r if r != 0)
        && critical_section::with(|cs| {
            let tasks = TASKS.borrow(cs).borrow();
            runner_rid()
                .checked_sub(1)
                .map(|idx| {
                    let p = tasks[idx as usize].phase;
                    p == PH_PENDING || p == PH_RUNNING
                })
                .unwrap_or(false)
        })
}

pub fn status_line() -> String<200> {
    let mut s = String::new();
    let _ = s.push_str("wasm ");
    let rid = runner_rid();
    if rid != 0 {
        push_runner_status(&mut s, rid);
        return s;
    }
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        for (i, t) in tasks.iter().enumerate() {
            if t.phase == PH_CONFIGURED {
                let _ = s.push_str("configured rid=");
                let _ = push_u8(&mut s, (i + 1) as u8);
                let _ = s.push('\n');
                return;
            }
        }
        let _ = s.push_str("idle\n");
    });
    s
}

fn push_runner_status(s: &mut String<200>, rid: u8) {
    critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let Some(t) = tasks.get(rid as usize - 1) else {
            let _ = s.push_str("idle\n");
            return;
        };
        match t.phase {
            PH_PENDING | PH_RUNNING => {
                let _ = s.push_str("running ");
                let _ = s.push_str(t.basename.as_str());
                let _ = s.push('\n');
            }
            PH_EXITED if t.exit_code == 0 => {
                let _ = s.push_str("done ");
                let _ = s.push_str(t.basename.as_str());
                let _ = s.push('\n');
            }
            PH_EXITED => {
                let _ = s.push_str("failed ");
                let _ = s.push_str(t.basename.as_str());
                let _ = s.push('\n');
            }
            PH_CONFIGURED => {
                let _ = s.push_str("configured rid=");
                let _ = push_u8(s, rid);
                let _ = s.push('\n');
            }
            _ => {
                let _ = s.push_str("idle\n");
            }
        }
    });
}

pub fn run_pending() -> bool {
    let rid = runner_rid();
    if rid == 0 {
        return false;
    }
    let phase = critical_section::with(|cs| {
        TASKS.borrow(cs).borrow()[rid as usize - 1].phase
    });
    if phase != PH_PENDING {
        return false;
    }

    critical_section::with(|cs| {
        TASKS.borrow(cs).borrow_mut()[rid as usize - 1].phase = PH_RUNNING;
    });

    let (wasm_ino, argv, env_lines, dir, basename) = critical_section::with(|cs| {
        let tasks = TASKS.borrow(cs).borrow();
        let scratch = RUNNER_SCRATCH.borrow(cs).borrow();
        let t = &tasks[rid as usize - 1];
        (
            scratch.wasm_ino,
            scratch.argv.clone(),
            scratch.env_lines.clone(),
            t.dir.clone(),
            t.basename.clone(),
        )
    });

    let run_outcome = run_guest(wasm_ino, &argv, &env_lines, dir.as_str());

    critical_section::with(|cs| {
        let mut tasks = TASKS.borrow(cs).borrow_mut();
        let t = &mut tasks[rid as usize - 1];
        match run_outcome {
            RunOutcome::Ok(out) => {
                t.exit_code = 0;
                if let Some(data_out) = &mut t.data_out {
                    append_out(data_out, out.as_bytes());
                }
            }
            RunOutcome::Err(msg) => {
                t.exit_code = -1;
                let mut line = heapless::String::<256>::new();
                let _ = line.push_str("wasm error: ");
                let _ = line.push_str(msg.as_str());
                if !line.ends_with('\n') {
                    let _ = line.push('\n');
                }
                if let Some(data_out) = &mut t.data_out {
                    append_out(data_out, line.as_bytes());
                }
            }
        }
        t.data_in = None;
        t.phase = PH_EXITED;
        set_runner(0);
    });
    let _ = basename;
    true
}

enum RunOutcome {
    Ok(&'static str),
    Err(heapless::String<256>),
}

fn run_err(msg: &'static str) -> RunOutcome {
    let mut s = heapless::String::<256>::new();
    let _ = s.push_str(msg);
    RunOutcome::Err(s)
}

fn run_guest(
    wasm_ino: u16,
    argv: &heapless::Vec<String<64>, MAX_ARGV>,
    env_lines: &heapless::Vec<String<ENV_LINE_CAP>, MAX_ENV_LINES>,
    preopen_dir: &str,
) -> RunOutcome {
    let wasm_bytes = match read_wasm(wasm_ino) {
        Ok(v) => v,
        Err(e) => return run_err(e),
    };
    if let Err(e) = ensure_runtime_heap() {
        return run_err(e);
    }
    if wamr_sys::init_runtime().is_err() {
        return run_err("wamr init failed");
    }

    let mut argv_refs: heapless::Vec<&str, MAX_ARGV> = heapless::Vec::new();
    for a in argv {
        let _ = argv_refs.push(a.as_str());
    }
    let mut env_refs: heapless::Vec<&str, MAX_ENV_LINES> = heapless::Vec::new();
    for e in env_lines {
        let _ = env_refs.push(e.as_str());
    }

    let mut err = [0u8; 256];
    match wamr_sys::run(
        &wasm_bytes,
        argv_refs.as_slice(),
        env_refs.as_slice(),
        preopen_dir,
        &mut err,
    ) {
        Ok(out) => RunOutcome::Ok(out),
        Err(()) => {
            let s = core::str::from_utf8(&err)
                .unwrap_or("wasm failed")
                .trim_end_matches('\0');
            let mut msg = heapless::String::<256>::new();
            let _ = msg.push_str(s);
            RunOutcome::Err(msg)
        }
    }
}

fn read_wasm(ino: u16) -> Result<Vec<u8>, &'static str> {
    let len = memfs::length(ino) as usize;
    let mut out = Vec::new();
    if out.try_reserve_exact(len).is_err() {
        return Err("wasm: out of memory loading file");
    }
    out.resize(len, 0);
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

pub fn preinit_runtime_heap() -> Result<(), &'static str> {
    ensure_runtime_heap()
}

fn ensure_runtime_heap() -> Result<(), &'static str> {
    if critical_section::with(|cs| RUNTIME_HEAP.borrow(cs).borrow().is_some()) {
        return Ok(());
    }
    let mut heap = Vec::new();
    if heap.try_reserve_exact(POOL_BYTES).is_err() {
        return Err("out of memory for wasm pool (see /sys/heap psram free=)");
    }
    heap.resize(POOL_BYTES, 0);
    wamr_sys::set_runtime_heap(heap.as_mut_ptr(), heap.len());
    core::sync::atomic::fence(Ordering::SeqCst);
    critical_section::with(|cs| {
        *RUNTIME_HEAP.borrow(cs).borrow_mut() = Some(heap);
    });
    Ok(())
}

pub enum TaskField {
    Cmd,
    Dir,
    Env,
}

fn new_data_buf() -> Result<Vec<u8>, &'static str> {
    let mut v = Vec::new();
    if v.try_reserve_exact(DATA_CAP).is_err() {
        return Err("task: out of memory for output");
    }
    Ok(v)
}

fn append_out(out: &mut Vec<u8>, data: &[u8]) {
    for &b in data {
        if out.len() >= DATA_CAP {
            break;
        }
        out.push(b);
    }
}

fn copy_bytes(src: &[u8], off: u64, buf: &mut [u8]) -> usize {
    if off >= src.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (src.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&src[start..start + n]);
    n
}

/// Read a text field plus a trailing `\n` (virtual; not stored in the slot).
fn copy_field_with_nl(s: &str, off: u64, buf: &mut [u8]) -> usize {
    let total = s.len() + 1;
    if off >= total as u64 {
        return 0;
    }
    let mut pos = off as usize;
    let mut out = 0usize;
    if pos < s.len() {
        let n = (s.len() - pos).min(buf.len());
        buf[..n].copy_from_slice(&s.as_bytes()[pos..pos + n]);
        out += n;
        pos += n;
    }
    if out < buf.len() && pos >= s.len() {
        buf[out] = b'\n';
        out += 1;
    }
    out
}

fn push_u8<const N: usize>(s: &mut String<N>, n: u8) -> Result<(), ()> {
    if n >= 10 {
        let _ = s.push((b'0' + n / 10) as char);
        let _ = s.push((b'0' + n % 10) as char);
    } else {
        let _ = s.push((b'0' + n) as char);
    }
    Ok(())
}

fn push_i32(s: &mut heapless::String<16>, mut n: i32) -> Result<(), ()> {
    if n == 0 {
        let _ = s.push('0');
        return Ok(());
    }
    if n < 0 {
        let _ = s.push('-');
        n = -n;
    }
    let mut digits = heapless::Vec::<u8, 12>::new();
    while n > 0 {
        let _ = digits.push((n % 10) as u8);
        n /= 10;
    }
    while let Some(d) = digits.pop() {
        let _ = s.push((b'0' + d) as char);
    }
    Ok(())
}
