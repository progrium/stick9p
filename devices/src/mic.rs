//! Microphone PCM ring + ctl (Stage 3).

use core::cell::RefCell;
use critical_section::Mutex;

/// PCM capture buffer (~0.5 s at 16 kHz mono s16le).
pub const PCM_RING_CAP: usize = 16_384;

struct MicState {
    ring: [u8; PCM_RING_CAP],
    head: usize,
    len: usize,
    rate_hz: u32,
    running: bool,
    /// Cumulative bytes the firmware audio task has lifted off the I²S
    /// RX DMA — independent of `running`. Exposed in `cat /dev/mic/ctl`
    /// as `rx_seen=` so we can tell hardware-mute from start/stop bugs.
    rx_seen: u64,
    /// How many times the audio task entered the RX drain branch (i.e.
    /// `available().await` returned). Distinguishes "DMA never produced"
    /// (polls=0) from "executor never scheduled rx_fut" (polls<1).
    rx_polls: u32,
}

static STATE: Mutex<RefCell<MicState>> = Mutex::new(RefCell::new(MicState {
    ring: [0; PCM_RING_CAP],
    head: 0,
    len: 0,
    rate_hz: 44_100,
    running: false,
    rx_seen: 0,
    rx_polls: 0,
}));

pub fn rate_hz() -> u32 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().rate_hz)
}

/// Override the reported sample rate. The audio task on each board calls
/// this once at startup so `cat /dev/mic/ctl` reflects the actual
/// hardware-locked rate (16 kHz on StickS3 via the ES8311 codec, 44.1
/// kHz on Plus2 via the SPM1423 PDM mic) instead of the static-init
/// default. Clients can still `echo rate N > ctl` but the hardware
/// itself is fixed — writes are accepted for forward-compat only.
pub fn set_rate_hz(hz: u32) {
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().rate_hz = hz);
}

pub fn is_running() -> bool {
    critical_section::with(|cs| STATE.borrow(cs).borrow().running)
}

pub fn set_running(on: bool) {
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().running = on);
}

pub fn flush_pcm() {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.head = 0;
        st.len = 0;
    });
}

/// Note that the firmware audio task popped `n` bytes from the I²S RX
/// DMA, regardless of whether they ended up in the ring (capture might
/// be stopped). Use to distinguish "DMA dead" (`rx_seen=0`) from
/// "capture stopped" (`rx_seen>0` but `queued=0`).
pub fn note_rx_seen(n: usize) {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().rx_seen += n as u64;
    });
}

/// Bump the per-iteration counter for the I²S RX drain loop. Distinct
/// from `note_rx_seen` so we can tell "audio task isn't being scheduled"
/// (polls=0) from "audio task is alive but DMA never completes" (polls=1
/// then stuck inside `available().await`).
pub fn note_rx_poll() {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.rx_polls = st.rx_polls.wrapping_add(1);
    });
}

/// Push raw PCM bytes from the I2S/PDM DMA path.
pub fn push_pcm(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        for &b in data {
            if st.len >= PCM_RING_CAP {
                // drop oldest
                st.head = (st.head + 1) % PCM_RING_CAP;
                st.len -= 1;
            }
            let tail = (st.head + st.len) % PCM_RING_CAP;
            st.ring[tail] = b;
            st.len += 1;
        }
    });
}

/// Non-blocking read from the front of the ring (`off` ignored — pcm is a pipe).
pub fn try_read_pcm(_off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if st.len == 0 {
            return 0;
        }
        let n = st.len.min(buf.len());
        for i in 0..n {
            buf[i] = st.ring[(st.head + i) % PCM_RING_CAP];
        }
        st.head = (st.head + n) % PCM_RING_CAP;
        st.len -= n;
        n
    })
}

pub fn status_line() -> heapless::String<96> {
    let mut s = heapless::String::new();
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let _ = s.push_str(if st.running { "running=1" } else { "running=0" });
        let _ = s.push_str(" rate=");
        let _ = push_u32_s(&mut s, st.rate_hz);
        let _ = s.push_str(" queued=");
        let _ = push_u32_s(&mut s, st.len as u32);
        let _ = s.push_str(" rx_seen=");
        let _ = push_u64_s(&mut s, st.rx_seen);
        let _ = s.push_str(" polls=");
        let _ = push_u32_s(&mut s, st.rx_polls);
        let _ = s.push_str(" fmt=s16le ch=1\n");
    });
    s
}

fn push_u32_s(s: &mut heapless::String<96>, mut n: u32) {
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

fn push_u64_s(s: &mut heapless::String<96>, mut n: u64) {
    let mut tmp = heapless::String::<24>::new();
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

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let parts: heapless::Vec<&str, 4> = s.split_whitespace().collect();
    match parts.as_slice() {
        ["rate", hz] => {
            let v: u32 = hz.parse().map_err(|_| "bad rate")?;
            match v {
                8000 | 16000 | 32000 | 44100 | 48000 => {
                    critical_section::with(|cs| {
                        STATE.borrow(cs).borrow_mut().rate_hz = v;
                    });
                    Ok(())
                }
                _ => Err("rate 8000|16000|32000|44100|48000 (hw: 44100)"),
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
        ["gain", _g] => Ok(()),
        ["start"] => {
            flush_pcm();
            set_running(true);
            Ok(())
        }
        ["stop"] => {
            set_running(false);
            Ok(())
        }
        ["flush"] => {
            flush_pcm();
            Ok(())
        }
        _ => Err("usage: rate N|bits 16|gain N|start|stop|flush"),
    }
}

#[allow(dead_code)]
fn push_u32(s: &mut heapless::String<48>, mut n: u32) {
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
