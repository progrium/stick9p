//! Button state + edge events (Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

const MAX_EVENTS: usize = 16;
const MAX_LINE: usize = 12;

struct ButtonsState {
    a: bool,
    b: bool,
    events: heapless::Vec<String<MAX_LINE>, MAX_EVENTS>,
}

static STATE: Mutex<RefCell<ButtonsState>> = Mutex::new(RefCell::new(ButtonsState {
    a: true,
    b: true,
    events: heapless::Vec::new(),
}));

pub fn set_a(pressed: bool) {
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().a = pressed);
}

pub fn set_b(pressed: bool) {
    critical_section::with(|cs| STATE.borrow(cs).borrow_mut().b = pressed);
}

/// Queue an edge: `a down`, `b up`, etc.
pub fn push_event(btn: char, pressed: bool) {
    let mut line = String::new();
    let _ = line.push(btn);
    let _ = line.push(' ');
    let _ = line.push_str(if pressed { "down" } else { "up" });
    let _ = line.push('\n');

    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if st.events.is_full() {
            let _ = st.events.remove(0);
        }
        let _ = st.events.push(line);
    });
}

pub fn flush_events() {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().events.clear();
    });
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    match s.trim() {
        "flush" => {
            flush_events();
            Ok(())
        }
        _ => Err("usage: flush"),
    }
}

pub fn read_a(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| read_level(off, buf, STATE.borrow(cs).borrow().a))
}

pub fn read_b(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| read_level(off, buf, STATE.borrow(cs).borrow().b))
}

/// Read one queued event line (non-blocking). Offset is ignored — this is a
/// pipe; v9fs increments offset after each read, but we always pop from the
/// front of the queue regardless.
pub fn try_read_event(_off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        if st.events.is_empty() {
            return 0;
        }
        let ev = st.events.remove(0);
        let bytes = ev.as_bytes();
        if buf.len() < bytes.len() {
            let _ = st.events.insert(0, ev);
            return 0;
        }
        buf[..bytes.len()].copy_from_slice(bytes);
        bytes.len()
    })
}

fn read_level(off: u64, buf: &mut [u8], pressed: bool) -> usize {
    let line: &[u8] = if pressed { b"1\n" } else { b"0\n" };
    if off >= line.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (line.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&line[start..start + n]);
    n
}
