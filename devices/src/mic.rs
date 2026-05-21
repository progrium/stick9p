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
}

static STATE: Mutex<RefCell<MicState>> = Mutex::new(RefCell::new(MicState {
    ring: [0; PCM_RING_CAP],
    head: 0,
    len: 0,
    rate_hz: 44_100,
    running: false,
}));

pub fn rate_hz() -> u32 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().rate_hz)
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

pub fn status_line() -> heapless::String<48> {
    let mut s = heapless::String::new();
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let _ = s.push_str(if st.running { "running=1" } else { "running=0" });
        let _ = s.push_str(" rate=");
        let _ = push_u32(&mut s, st.rate_hz);
        let _ = s.push_str(" queued=");
        let _ = push_u32(&mut s, st.len as u32);
        let _ = s.push_str(" fmt=s16le\n");
    });
    s
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
