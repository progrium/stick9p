//! LED control (Plus2: GPIO19 red LED; StickS3: M5PM1 in Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

static LED: Mutex<RefCell<LedState>> = Mutex::new(RefCell::new(LedState::Off));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedState {
    On,
    Off,
    Blink { hi_ms: u32, lo_ms: u32 },
}

impl LedState {
    pub fn as_str(self) -> &'static str {
        match self {
            LedState::On => "on",
            LedState::Off => "off",
            LedState::Blink { .. } => "blink",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LedError {
    BadCtl,
}

pub fn get() -> LedState {
    critical_section::with(|cs| *LED.borrow_ref(cs))
}

pub fn set(state: LedState) {
    critical_section::with(|cs| *LED.borrow_ref_mut(cs) = state);
}

pub fn handle_ctl(cmd: &str) -> Result<(), LedError> {
    let mut parts = cmd.split_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some("on"), None, None) => {
            set(LedState::On);
            Ok(())
        }
        (Some("off"), None, None) => {
            set(LedState::Off);
            Ok(())
        }
        (Some("blink"), Some(hi), Some(lo)) => {
            let hi_ms: u32 = hi.parse().map_err(|_| LedError::BadCtl)?;
            let lo_ms: u32 = lo.parse().map_err(|_| LedError::BadCtl)?;
            set(LedState::Blink { hi_ms, lo_ms });
            Ok(())
        }
        _ => Err(LedError::BadCtl),
    }
}

pub fn state_line() -> String<32> {
    let mut s = String::new();
    match get() {
        LedState::On => {
            let _ = s.push_str("on\n");
        }
        LedState::Off => {
            let _ = s.push_str("off\n");
        }
        LedState::Blink { hi_ms, lo_ms } => {
            let _ = s.push_str("blink ");
            push_u32(hi_ms, &mut s);
            let _ = s.push(' ');
            push_u32(lo_ms, &mut s);
            let _ = s.push('\n');
        }
    }
    s
}

fn push_u32(mut n: u32, s: &mut String<32>) {
    let mut buf = [0u8; 10];
    let mut i = 10;
    if n == 0 {
        let _ = s.push('0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    let _ = s.push_str(core::str::from_utf8(&buf[i..]).unwrap_or("0"));
}
