//! IMU sample buffer (Stage 2).

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

const LINE_CAP: usize = 48;

struct ImuState {
    rate_hz: u16,
    latest_accel: String<LINE_CAP>,
    latest_gyro: String<LINE_CAP>,
}

static STATE: Mutex<RefCell<ImuState>> = Mutex::new(RefCell::new(ImuState {
    rate_hz: 25,
    latest_accel: String::new(),
    latest_gyro: String::new(),
}));

pub fn set_rate_hz(hz: u16) {
    critical_section::with(|cs| {
        STATE.borrow(cs).borrow_mut().rate_hz = hz.clamp(1, 200);
    });
}

pub fn rate_hz() -> u16 {
    critical_section::with(|cs| STATE.borrow(cs).borrow().rate_hz)
}

pub fn push_accel(ax: i32, ay: i32, az: i32) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.latest_accel.clear();
        let _ = write_i32(&mut st.latest_accel, ax);
        let _ = st.latest_accel.push(' ');
        let _ = write_i32(&mut st.latest_accel, ay);
        let _ = st.latest_accel.push(' ');
        let _ = write_i32(&mut st.latest_accel, az);
        let _ = st.latest_accel.push('\n');
    });
}

pub fn push_gyro(gx: i32, gy: i32, gz: i32) {
    critical_section::with(|cs| {
        let mut st = STATE.borrow(cs).borrow_mut();
        st.latest_gyro.clear();
        let _ = write_i32(&mut st.latest_gyro, gx);
        let _ = st.latest_gyro.push(' ');
        let _ = write_i32(&mut st.latest_gyro, gy);
        let _ = st.latest_gyro.push(' ');
        let _ = write_i32(&mut st.latest_gyro, gz);
        let _ = st.latest_gyro.push('\n');
    });
}

pub fn read_accel(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let line = st.latest_accel.as_bytes();
        if off >= line.len() as u64 {
            return 0;
        }
        let start = off as usize;
        let n = (line.len() - start).min(buf.len());
        buf[..n].copy_from_slice(&line[start..start + n]);
        n
    })
}

pub fn read_gyro(off: u64, buf: &mut [u8]) -> usize {
    critical_section::with(|cs| {
        let st = STATE.borrow(cs).borrow();
        let line = st.latest_gyro.as_bytes();
        if off >= line.len() as u64 {
            return 0;
        }
        let start = off as usize;
        let n = (line.len() - start).min(buf.len());
        buf[..n].copy_from_slice(&line[start..start + n]);
        n
    })
}

pub fn handle_ctl(s: &str) -> Result<(), &'static str> {
    let cmd = s.trim();
    if let Some(hz) = cmd.strip_prefix("rate ") {
        let v: u16 = hz.trim().parse().map_err(|_| "bad rate")?;
        if ![25, 50, 100, 200].contains(&v) {
            set_rate_hz(v.clamp(1, 200));
        } else {
            set_rate_hz(v);
        }
        Ok(())
    } else {
        Err("unknown ctl")
    }
}

fn write_i32(s: &mut String<LINE_CAP>, mut n: i32) -> Result<(), ()> {
    if n == 0 {
        return s.push('0');
    }
    if n < 0 {
        let _ = s.push('-');
        n = -n;
    }
    let mut digits = heapless::Vec::<u8, 12>::new();
    while n > 0 {
        let _ = digits.push((n % 10) as u8 + b'0');
        n /= 10;
    }
    while let Some(d) = digits.pop() {
        let _ = s.push(d as char);
    }
    Ok(())
}
