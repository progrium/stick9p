//! Speaker PCM ring + ctl (Stage 3, StickS3).
//!
//! The 9P surface is `/dev/spk/{ctl,pcm,info}`:
//!
//! - `ctl` — `start | stop | flush | rate N | bits 16 | gain N | fanfare`.
//!   `gain` is a software multiplier in Q8 (256 = unity, 0 = mute, 512 = +6 dB
//!   clipped at i16 fullscale). `rate` is accepted but the codec stays at
//!   16 kHz today; the value is only stored so clients see it round-trip.
//! - `pcm` — write-only stream of s16le **mono** samples. Bytes are pushed
//!   into a ring buffer that the firmware's `audio_task` drains into the
//!   ES8311 over I²S. Reads return 0 (it's a pipe, not a file).
//! - `info` — read-only static string with the on-wire format.
//!
//! Backpressure: when the ring fills, `write_pcm` returns a short count
//! (possibly 0). 9P clients handle this naturally — the Twrite reply tells
//! them how many bytes were accepted, and they retry the remainder. The ring
//! is sized for ~1 s of audio at 16 kHz, which comfortably absorbs WiFi
//! jitter without dropping samples.

extern crate alloc;

use alloc::boxed::Box;
use core::cell::RefCell;
use critical_section::Mutex;

/// Ring capacity — 32 KiB ≈ 1.0 s of mono s16le @ 16 kHz.
///
/// Lives on the **heap** (not BSS). Tried bumping to 64-128 KiB but the
/// StickS3 has very limited internal SRAM headroom: enlarging either BSS
/// or the heap pool overflows into the embassy task stack region and
/// trips the stack guard at runtime. 32 KiB is the largest size that
/// fits comfortably alongside the framebuffer + fanfare buffer + 9P
/// scratch in the StickS3 heap budget. Click smoothing in the audio task
/// (see `firmware/src/dev/sticks3.rs`) covers the rest.
pub const PCM_RING_CAP: usize = 32_768;

/// On-wire format string returned by `/dev/spk/info`.
pub const INFO_TEXT: &str = "fmt=s16le ch=1 rate=16000\n";

struct SpkState {
    ring: Option<&'static mut [u8]>,
    head: usize,
    len: usize,
    rate_hz: u32,
    gain_q8: u16,
    running: bool,
    underruns: u32,
}

static STATE: Mutex<RefCell<SpkState>> = Mutex::new(RefCell::new(SpkState {
    ring: None,
    head: 0,
    len: 0,
    rate_hz: 16_000,
    gain_q8: 256,
    running: false,
    underruns: 0,
}));

/// Allocate the PCM ring on the heap. Must be called once at boot, before
/// any 9P traffic touches `/dev/spk/pcm`. We keep the buffer on the heap
/// instead of in BSS because a 128 KiB static would overrun the StickS3
/// task stack guard region; the heap allocator has plenty of headroom.
pub fn init() {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if st.ring.is_none() {
            let buf = alloc::vec![0u8; PCM_RING_CAP].into_boxed_slice();
            st.ring = Some(Box::leak(buf));
        }
    });
}

pub fn rate_hz() -> u32 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().rate_hz)
}

pub fn is_running() -> bool {
    critical_section::with(|cs| STATE.borrow(cs).borrow().running)
}

pub fn set_running(on: bool) {
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().running = on);
}

pub fn gain_q8() -> u16 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().gain_q8)
}

pub fn queued_bytes() -> usize {
    critical_section::with(|cs| STATE.borrow(cs).borrow().len)
}

pub fn note_underrun() {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.underruns = st.underruns.saturating_add(1);
    });
}

pub fn flush() {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.head = 0;
        st.len = 0;
    });
}

/// Producer side: push raw PCM bytes from the 9P `Twrite` handler. Returns
/// the number of bytes accepted (≤ `data.len()`). The 9P server uses the
/// short-write as backpressure — clients retry the unwritten remainder.
///
/// `off` is ignored: `/dev/spk/pcm` is a stream, not a seekable file.
pub fn write_pcm(_off: u64, data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let head = st.head;
        let queued = st.len;
        let ring = match st.ring.as_deref_mut() {
            Some(r) => r,
            None => return 0,
        };
        let cap = ring.len();
        let free = cap - queued;
        let n = free.min(data.len());
        if n == 0 {
            return 0;
        }
        // Bulk copy via at most two slice ranges (before and after wrap)
        // rather than byte-by-byte with a modulo per byte. Keeps the
        // critical section short so the 9P task and audio task aren't
        // contending unnecessarily.
        let tail = (head + queued) % cap;
        let first = (cap - tail).min(n);
        ring[tail..tail + first].copy_from_slice(&data[..first]);
        if first < n {
            ring[..n - first].copy_from_slice(&data[first..n]);
        }
        st.len = queued + n;
        n
    })
}

/// Consumer side: drain up to `buf.len()` bytes from the ring. Called by the
/// audio task between DMA pushes.
pub fn try_drain(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let head = st.head;
        let avail = st.len;
        let ring = match st.ring.as_deref() {
            Some(r) => r,
            None => return 0,
        };
        if avail == 0 {
            return 0;
        }
        let cap = ring.len();
        let n = avail.min(buf.len());
        let first = (cap - head).min(n);
        buf[..first].copy_from_slice(&ring[head..head + first]);
        if first < n {
            buf[first..n].copy_from_slice(&ring[..n - first]);
        }
        st.head = (head + n) % cap;
        st.len -= n;
        n
    })
}

/// `cat /dev/spk/ctl` response.
pub fn status_line() -> heapless::String<96> {
    let mut s = heapless::String::new();
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let _ = s.push_str(if st.running { "running=1" } else { "running=0" });
        let _ = s.push_str(" rate=");
        push_u32(&mut s, st.rate_hz);
        let _ = s.push_str(" gain=");
        push_u32(&mut s, st.gain_q8 as u32);
        let _ = s.push_str(" queued=");
        push_u32(&mut s, st.len as u32);
        let _ = s.push_str(" cap=");
        let cap = st.ring.as_deref().map(|r| r.len()).unwrap_or(0) as u32;
        push_u32(&mut s, cap);
        let _ = s.push_str(" under=");
        push_u32(&mut s, st.underruns);
        let _ = s.push_str(" fmt=s16le ch=1\n");
    });
    s
}

/// Optional: schedule the boot fanfare via `/dev/spk/ctl fanfare`. Returns
/// true if accepted. The actual audio is queued through the same `buzzer`
/// signal the boot path uses so the code stays in one place.
pub fn request_fanfare() {
    // re-export via devices::buzzer so callers don't need both crates.
    crate::buzzer::request_stage2_done();
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let parts: heapless::Vec<&str, 4> = s.split_whitespace().collect();
    match parts.as_slice() {
        ["start"] => {
            set_running(true);
            Ok(())
        }
        ["stop"] => {
            set_running(false);
            Ok(())
        }
        ["flush"] => {
            flush();
            Ok(())
        }
        ["fanfare"] => {
            request_fanfare();
            Ok(())
        }
        ["rate", hz] => {
            let v: u32 = hz.parse().map_err(|_| "bad rate")?;
            match v {
                8000 | 16000 | 22050 | 32000 | 44100 | 48000 => {
                    critical_section::with(|cs| {
                        STATE.borrow(cs).borrow_mut().rate_hz = v;
                    });
                    Ok(())
                }
                _ => Err("rate 8000|16000|22050|32000|44100|48000 (hw: 16000)"),
            }
        }
        ["bits", b] => {
            let v: u32 = b.parse().map_err(|_| "bad bits")?;
            if v == 16 {
                Ok(())
            } else {
                Err("bits 16 only")
            }
        }
        ["gain", g] => {
            let v: u32 = g.parse().map_err(|_| "bad gain")?;
            if v > 512 {
                return Err("gain 0..512 (Q8, 256=unity)");
            }
            critical_section::with(|cs| {
                STATE.borrow(cs).borrow_mut().gain_q8 = v as u16;
            });
            Ok(())
        }
        _ => Err("usage: start|stop|flush|fanfare|rate N|bits 16|gain N"),
    }
}

fn push_u32(s: &mut heapless::String<96>, mut n: u32) {
    let mut tmp = heapless::String::<12>::new();
    if n == 0 {
        let _ = tmp.push('0');
    } else {
        while n > 0 {
            let d = (n % 10) as u8;
            let _ = tmp.push((b'0' + d) as char);
            n /= 10;
        }
    }
    while let Some(c) = tmp.pop() {
        let _ = s.push(c);
    }
}
