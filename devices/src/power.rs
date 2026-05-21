//! Power / battery (Plus2 Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

struct PowerState {
    vbat_mv: u32,
    hold_on: bool,
}

static STATE: Mutex<RefCell<PowerState>> = Mutex::new(RefCell::new(PowerState {
    vbat_mv: 0,
    hold_on: true,
}));

pub fn set_vbat_mv(mv: u32) {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().vbat_mv = mv;
    });
}

pub fn battery_line() -> String<64> {
    critical_section::with(|cs| {
        let mv = STATE.borrow(cs).borrow().vbat_mv;
        let mut s = String::new();
        let _ = s.push_str("vbat_mv=");
        let _ = s.push_str(u32_to_str(mv).as_str());
        let _ = s.push_str(" charging=0 source=BAT\n");
        s
    })
}

pub fn vbat_line() -> String<16> {
    critical_section::with(|cs| {
        let mut s = String::new();
        let _ = s.push_str(u32_to_str(STATE.borrow(cs).borrow().vbat_mv).as_str());
        let _ = s.push('\n');
        s
    })
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let cmd = s.trim();
    match cmd {
        "hold on" => {
            critical_section::with(|cs| STATE.borrow(cs).borrow_mut().hold_on = true);
            Ok(())
        }
        "hold off" | "shutdown" => {
            critical_section::with(|cs| STATE.borrow(cs).borrow_mut().hold_on = false);
            Ok(())
        }
        _ => Err("unknown ctl"),
    }
}

pub fn hold_should_be_high() -> bool {
    critical_section::with(|cs| STATE.borrow(cs).borrow().hold_on)
}

fn u32_to_str(mut n: u32) -> String<12> {
    let mut s = String::new();
    if n == 0 {
        let _ = s.push('0');
        return s;
    }
    let mut digits = heapless::Vec::<u8, 10>::new();
    while n > 0 {
        let _ = digits.push((n % 10) as u8 + b'0');
        n /= 10;
    }
    while let Some(d) = digits.pop() {
        let _ = s.push(d as char);
    }
    s
}
