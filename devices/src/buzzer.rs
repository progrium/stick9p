//! Buzzer ctl + completion chime request (Plus2 Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;

#[derive(Clone, Copy, Debug, Default)]
pub struct BeepRequest {
    pub freq_hz: u32,
    pub ms: u32,
}

struct BuzzerState {
    pending: Option<BeepRequest>,
    play_done: bool,
}

static STATE: Mutex<RefCell<BuzzerState>> = Mutex::new(RefCell::new(BuzzerState {
    pending: None,
    play_done: false,
}));

pub fn request_beep(freq_hz: u32, ms: u32) {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().pending = Some(BeepRequest { freq_hz, ms });
    });
}

pub fn take_beep() -> Option<BeepRequest> {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().pending.take()
    })
}

/// Queue the Stage 2 completion fanfare (two short beeps).
pub fn request_stage2_done() {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().play_done = true;
    });
}

pub fn take_done_fanfare() -> bool {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        let v = st.play_done;
        st.play_done = false;
        v
    })
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let parts: heapless::Vec<&str, 4> = s.split_whitespace().collect();
    match parts.as_slice() {
        ["beep", freq, ms] => {
            let f: u32 = freq.parse().map_err(|_| "bad freq")?;
            let m: u32 = ms.parse().map_err(|_| "bad ms")?;
            request_beep(f, m);
            Ok(())
        }
        ["stop"] => Ok(()),
        _ => Err("usage: beep <hz> <ms>"),
    }
}
