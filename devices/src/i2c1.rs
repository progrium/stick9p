//! External I²C bus (`/dev/i2c/1`).
//!
//! Exposes the Grove/HAT2 I²C bus to 9P clients. Three files:
//!
//! - `ctl` — read: status line (`freq=`, `last=ok|err …`). Write: `freq N`.
//! - `scan` — read: probes all 7-bit addresses on the bus and returns a
//!   newline-separated list of hex addresses that ACK'd. Each open of
//!   `scan` re-runs the probe (so plugging/unplugging units between reads
//!   gives fresh results).
//! - `data` — write: command line describing a transaction
//!   (`r <addr> <count>`, `w <addr> <hex …>`, `rw <addr> <hex …> <count>`).
//!   Read: bytes returned by the most recent read/rw transaction.
//!
//! The actual bus hardware lives in the firmware crate (board-specific
//! pins, esp-hal types). This module owns:
//!   - the parsed-command state machine,
//!   - the last-result ring (binary bytes for `data`, formatted text for
//!     `ctl` and `scan`),
//!   - and the synchronization primitives the firmware task uses to know
//!     when the 9P side has queued a fresh transaction.
//!
//! See `firmware/src/dev/sticks3.rs::i2c1_*` for the hardware glue.

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::{String, Vec};

/// Maximum bytes accepted in a single read or write transaction. Sized so
/// command lines fit in 9P Tmsg payload comfortably and the result buffer
/// fits in `MSG_CAP - header`.
pub const MAX_XFER: usize = 64;

/// Bus configuration knobs settable via `ctl`. `freq` is what the firmware
/// re-applies when it next opens the bus; runtime reconfiguration is not
/// yet supported (the I²C peripheral on esp-hal needs to be torn down and
/// rebuilt to change clock), so changing `freq` only takes effect at next
/// boot.
struct Config {
    freq_hz: u32,
}

/// Outcome of the most recent transaction. `Idle` is the boot state.
#[derive(Clone)]
enum LastStatus {
    Idle,
    Ok,
    Err(String<48>),
}

struct I2c1State {
    cfg: Config,
    last_status: LastStatus,
    /// Bytes returned by the most recent read or rw. Read by `data`.
    last_data: Vec<u8, MAX_XFER>,
    /// Last scan result formatted as ASCII (`0x18\n0x68\n…`). Built lazily
    /// each time `scan` is read.
    last_scan: Vec<u8, 256>,
}

static STATE: Mutex<RefCell<I2c1State>> = Mutex::new(RefCell::new(I2c1State {
    cfg: Config { freq_hz: 100_000 },
    last_status: LastStatus::Idle,
    last_data: Vec::new(),
    last_scan: Vec::new(),
}));

/// Parsed transaction the firmware should execute.
#[derive(Clone, Debug)]
pub enum Xfer {
    /// `r ADDR COUNT` — read `count` bytes from `addr`.
    Read { addr: u8, count: usize },
    /// `w ADDR B1 B2 …` — write bytes to `addr`.
    Write { addr: u8, data: Vec<u8, MAX_XFER> },
    /// `rw ADDR W1 W2 … COUNT` — write-then-restart-read.
    WriteRead {
        addr: u8,
        write: Vec<u8, MAX_XFER>,
        read: usize,
    },
}

pub fn freq_hz() -> u32 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().cfg.freq_hz)
}

/// Update the configured frequency. Returns Err with a usage hint on bad
/// input.
pub fn set_freq_hz(hz: u32) -> Result<(), &'static str> {
    if !(10_000..=1_000_000).contains(&hz) {
        return Err("freq 10000..1000000");
    }
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().cfg.freq_hz = hz);
    Ok(())
}

/// Mark the last transaction as successful and (optionally) stash its read
/// payload for the next `cat /dev/i2c/1/data`.
pub fn record_ok(read_payload: &[u8]) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.last_status = LastStatus::Ok;
        st.last_data.clear();
        let n = read_payload.len().min(st.last_data.capacity());
        st.last_data.extend_from_slice(&read_payload[..n]).ok();
    });
}

pub fn record_err(msg: &str) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let mut buf = String::new();
        let _ = buf.push_str(msg);
        st.last_status = LastStatus::Err(buf);
        st.last_data.clear();
    });
}

/// Replace the cached scan text. The format is one hex address per line,
/// e.g. `0x18\n0x68\n0x6e\n`. Empty if no devices ACK'd.
pub fn record_scan(addrs: &[u8]) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.last_scan.clear();
        for &a in addrs {
            let _ = push_hex_addr(&mut st.last_scan, a);
            let _ = st.last_scan.push(b'\n');
        }
    });
}

/// Copy the cached scan text into `buf` honoring 9P `off`.
pub fn read_scan(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        copy_with_offset(&st.last_scan, off, buf)
    })
}

/// Copy the cached `data` payload into `buf` honoring 9P `off`.
pub fn read_data(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        copy_with_offset(&st.last_data, off, buf)
    })
}

/// `cat /dev/i2c/1/ctl` text: `freq=N last=ok|err:<msg>\n`.
pub fn status_line() -> String<128> {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let mut s = String::new();
        let _ = s.push_str("freq=");
        push_u32(&mut s, st.cfg.freq_hz);
        let _ = s.push_str(" last=");
        match &st.last_status {
            LastStatus::Idle => {
                let _ = s.push_str("idle");
            }
            LastStatus::Ok => {
                let _ = s.push_str("ok");
            }
            LastStatus::Err(e) => {
                let _ = s.push_str("err:");
                let _ = s.push_str(e.as_str());
            }
        }
        let _ = s.push('\n');
        s
    })
}

/// Parse a `ctl`-line command. Returns either a `Xfer` for the firmware
/// to execute or applies inline (freq=) and returns Ok(None).
pub fn parse_ctl(line: &str) -> Result<Option<Xfer>, &'static str> {
    let mut it = line.split_whitespace();
    let head = it.next().ok_or("empty command")?;
    match head {
        "freq" => {
            let n: u32 = it.next().ok_or("freq <hz>")?.parse().map_err(|_| "bad hz")?;
            set_freq_hz(n)?;
            Ok(None)
        }
        _ => parse_xfer(line),
    }
}

/// Parse just the data-line transaction subset (no `freq` etc.).
pub fn parse_xfer(line: &str) -> Result<Option<Xfer>, &'static str> {
    let mut it = line.split_whitespace();
    let head = it.next().ok_or("empty command")?;
    match head {
        "r" | "R" => {
            let addr = parse_byte(it.next().ok_or("r <addr> <count>")?)?;
            let count: usize = it
                .next()
                .ok_or("r <addr> <count>")?
                .parse()
                .map_err(|_| "bad count")?;
            if count == 0 || count > MAX_XFER {
                return Err("count 1..64");
            }
            Ok(Some(Xfer::Read { addr, count }))
        }
        "w" | "W" => {
            let addr = parse_byte(it.next().ok_or("w <addr> <byte>...")?)?;
            let mut bytes: Vec<u8, MAX_XFER> = Vec::new();
            for tok in it {
                bytes
                    .push(parse_byte(tok)?)
                    .map_err(|_| "too many bytes (max 64)")?;
            }
            if bytes.is_empty() {
                return Err("w needs at least one byte");
            }
            Ok(Some(Xfer::Write { addr, data: bytes }))
        }
        "rw" | "wr" | "RW" | "WR" => {
            // Last token is the read count; preceding tokens after addr are write bytes.
            let addr = parse_byte(it.next().ok_or("rw <addr> <wbyte>... <count>")?)?;
            let toks: heapless::Vec<&str, 32> = it.collect();
            if toks.len() < 2 {
                return Err("rw <addr> <wbyte>... <count>");
            }
            let (write_toks, count_tok) = toks.split_at(toks.len() - 1);
            let count: usize = count_tok[0].parse().map_err(|_| "bad count")?;
            if count == 0 || count > MAX_XFER {
                return Err("count 1..64");
            }
            let mut write: Vec<u8, MAX_XFER> = Vec::new();
            for t in write_toks {
                write
                    .push(parse_byte(t)?)
                    .map_err(|_| "too many write bytes (max 64)")?;
            }
            if write.is_empty() {
                return Err("rw needs at least one write byte");
            }
            Ok(Some(Xfer::WriteRead {
                addr,
                write,
                read: count,
            }))
        }
        _ => Err("usage: r ADDR COUNT | w ADDR B... | rw ADDR W... COUNT | freq HZ"),
    }
}

// ----- helpers ------------------------------------------------------------

fn parse_byte(tok: &str) -> Result<u8, &'static str> {
    let s = tok.trim();
    let (radix, body) = if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, rest)
    } else {
        (10, s)
    };
    u8::from_str_radix(body, radix).map_err(|_| "bad byte (0..255, hex via 0x.. or decimal)")
}

fn copy_with_offset(src: &[u8], off: u64, dst: &mut [u8]) -> usize {
    if off as usize >= src.len() {
        return 0;
    }
    let start = off as usize;
    let n = (src.len() - start).min(dst.len());
    dst[..n].copy_from_slice(&src[start..start + n]);
    n
}

fn push_hex_addr<const N: usize>(buf: &mut Vec<u8, N>, addr: u8) -> Result<(), ()> {
    buf.push(b'0').map_err(|_| ())?;
    buf.push(b'x').map_err(|_| ())?;
    let hi = (addr >> 4) & 0xF;
    let lo = addr & 0xF;
    buf.push(hex_digit(hi)).map_err(|_| ())?;
    buf.push(hex_digit(lo)).map_err(|_| ())?;
    Ok(())
}

fn hex_digit(n: u8) -> u8 {
    if n < 10 {
        b'0' + n
    } else {
        b'a' + (n - 10)
    }
}

fn push_u32(s: &mut String<128>, mut n: u32) {
    let mut tmp: String<12> = String::new();
    if n == 0 {
        let _ = tmp.push('0');
    } else {
        while n > 0 {
            let _ = tmp.push((b'0' + (n % 10) as u8) as char);
            n /= 10;
        }
    }
    while let Some(c) = tmp.pop() {
        let _ = s.push(c);
    }
}
