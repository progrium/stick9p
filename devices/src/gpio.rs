//! User-claimable GPIO pins (`/dev/gpio/<N>`).
//!
//! Per the M5Stack StickS3 schematic v0.6 the Hat2-Bus headers expose
//! G1..G8 as free user GPIOs (G9/G10 are the Grove I²C bus, G43/G44 are
//! the legacy U0RX/U0TX). Plus2 has no spare GPIOs in the v0.6 board map.
//!
//! Each claimable pin has a tiny `PinState` describing its current mode
//! and (for outputs) the last requested level. The firmware glue mirrors
//! that state onto the hardware via esp-hal's `Input` / `Output` drivers,
//! and updates the cached level for input pins on every read.
//!
//! The 9P surface is `/dev/gpio/<N>/{ctl,level}`:
//!   - `ctl` accepts: `in`, `in-pup`, `in-pdn`, `out`, `out-od`. Read
//!     returns one line summarising the current mode.
//!   - `level` accepts `0`/`1` writes (output only); reads return the
//!     current pin level as `0\n` / `1\n` regardless of mode.

use core::cell::RefCell;
use critical_section::Mutex;
use heapless::String;

/// Modes accepted by `/dev/gpio/<N>/ctl`. The hardware glue is what
/// actually applies these; this enum is just the serializable contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Disconnected — input, no pull. Pin floats.
    InFloating,
    /// Input with internal pull-up.
    InPullup,
    /// Input with internal pull-down.
    InPulldown,
    /// Push-pull output.
    PushPull,
    /// Open-drain output (high = Hi-Z, low = pulled to GND).
    OpenDrain,
}

impl Mode {
    pub fn is_output(self) -> bool {
        matches!(self, Mode::PushPull | Mode::OpenDrain)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::InFloating => "in",
            Mode::InPullup => "in-pup",
            Mode::InPulldown => "in-pdn",
            Mode::PushPull => "out",
            Mode::OpenDrain => "out-od",
        }
    }
}

/// Per-pin software state. The hardware-level pin lives in the firmware
/// crate; this struct is only what the 9P layer needs to report and act
/// on. `present=false` means the running board doesn't expose this pin.
#[derive(Clone, Copy)]
pub struct PinState {
    pub present: bool,
    pub mode: Mode,
    /// Last-requested output level. Meaningful only when `mode.is_output()`.
    pub out_level: bool,
    /// Last sampled input level (refreshed by firmware on `level` reads).
    pub in_level: bool,
}

impl PinState {
    const fn absent() -> Self {
        Self {
            present: false,
            mode: Mode::InFloating,
            out_level: false,
            in_level: false,
        }
    }
}

/// Claimable pin numbers exposed by the 9P tree. Pins outside this set
/// always return ENOENT on `walk`. Boards that don't actually wire any
/// of these (e.g. Plus2 in v0.6) simply never call `register`, and reads
/// of `/dev/gpio/<N>/ctl` then return "absent\n".
pub const CLAIMABLE_PINS: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8];

/// Highest GPIO number we serve. Sized to bound the `STATE` table without
/// allocating space for every pin on the chip.
pub const MAX_PIN: u8 = 10;

struct GpioTable {
    pins: [PinState; (MAX_PIN as usize) + 1],
}

static STATE: Mutex<RefCell<GpioTable>> = Mutex::new(RefCell::new(GpioTable {
    pins: [PinState::absent(); (MAX_PIN as usize) + 1],
}));

/// Register a pin as claimable. Called once from board bring-up; without
/// this the pin is invisible to the 9P tree.
pub fn register(pin: u8) {
    if !is_known_pin(pin) {
        return;
    }
    critical_section::with(|cs| {
        let mut t = STATE.borrow(cs).borrow_mut();
        t.pins[pin as usize] = PinState {
            present: true,
            mode: Mode::InFloating,
            out_level: false,
            in_level: false,
        };
    });
}

pub fn is_claimable(pin: u8) -> bool {
    CLAIMABLE_PINS.contains(&pin)
}

fn is_known_pin(pin: u8) -> bool {
    pin <= MAX_PIN
}

/// Snapshot the current state of a pin. Returns `None` if the pin is not
/// exposed on this board.
pub fn snapshot(pin: u8) -> Option<PinState> {
    if !is_known_pin(pin) {
        return None;
    }
    critical_section::with(|cs| {
        let t = STATE.borrow(cs).borrow();
        let s = t.pins[pin as usize];
        if s.present { Some(s) } else { None }
    })
}

/// Parse a `ctl` line into a target `Mode`. The firmware glue applies the
/// mode to hardware and then calls `set_mode` to persist it.
pub fn parse_mode(line: &str) -> Result<Mode, &'static str> {
    match line.trim() {
        "in" | "in-z" | "input" => Ok(Mode::InFloating),
        "in-pup" | "in-up" | "pullup" => Ok(Mode::InPullup),
        "in-pdn" | "in-down" | "pulldown" => Ok(Mode::InPulldown),
        "out" | "out-pp" | "output" => Ok(Mode::PushPull),
        "out-od" | "open-drain" => Ok(Mode::OpenDrain),
        _ => Err("mode: in|in-pup|in-pdn|out|out-od"),
    }
}

pub fn set_mode(pin: u8, mode: Mode) {
    if !is_known_pin(pin) {
        return;
    }
    critical_section::with(|cs| {
        let mut t = STATE.borrow(cs).borrow_mut();
        if t.pins[pin as usize].present {
            t.pins[pin as usize].mode = mode;
        }
    });
}

pub fn set_out_level(pin: u8, level: bool) {
    if !is_known_pin(pin) {
        return;
    }
    critical_section::with(|cs| {
        let mut t = STATE.borrow(cs).borrow_mut();
        if t.pins[pin as usize].present {
            t.pins[pin as usize].out_level = level;
        }
    });
}

pub fn set_in_level(pin: u8, level: bool) {
    if !is_known_pin(pin) {
        return;
    }
    critical_section::with(|cs| {
        let mut t = STATE.borrow(cs).borrow_mut();
        if t.pins[pin as usize].present {
            t.pins[pin as usize].in_level = level;
        }
    });
}

/// Single-line summary for `cat /dev/gpio/<N>/ctl`.
pub fn ctl_status_line(pin: u8) -> String<64> {
    let mut s: String<64> = String::new();
    let Some(st) = snapshot(pin) else {
        let _ = s.push_str("absent\n");
        return s;
    };
    let _ = s.push_str("mode=");
    let _ = s.push_str(st.mode.as_str());
    if st.mode.is_output() {
        let _ = s.push_str(" out=");
        let _ = s.push(if st.out_level { '1' } else { '0' });
    }
    let _ = s.push_str(" in=");
    let _ = s.push(if st.in_level { '1' } else { '0' });
    let _ = s.push('\n');
    s
}

/// Single-character body for `cat /dev/gpio/<N>/level`. For input modes
/// this is the cached sampled level; for outputs it's the driven level.
pub fn level_line(pin: u8) -> String<4> {
    let mut s: String<4> = String::new();
    if let Some(st) = snapshot(pin) {
        let lvl = if st.mode.is_output() {
            st.out_level
        } else {
            st.in_level
        };
        let _ = s.push(if lvl { '1' } else { '0' });
        let _ = s.push('\n');
    } else {
        let _ = s.push_str("0\n");
    }
    s
}
