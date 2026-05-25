//! User-claimable GPIO pin glue (`/dev/gpio/<N>`).
//!
//! Each pin claimed by the board lives in an `esp_hal::gpio::Flex` so
//! mode changes (`in` ↔ `out`) don't require deconstructing the driver
//! — we just toggle the input/output enable bits and re-apply the
//! input or output config. That side-steps the lifetime gymnastics of
//! pulling an `AnyPin` back out of an `Input`/`Output` (which esp-hal
//! 1.1's stable API doesn't support).

use core::cell::RefCell;
use critical_section::Mutex;
use esp_hal::gpio::{DriveMode, Flex, InputConfig, Level, OutputConfig, Pull};

use devices::gpio::Mode;

const NUM_SLOTS: usize = 11; // pins 0..=10, indexed by pin number

struct Slot {
    flex: Option<Flex<'static>>,
}

const SLOT_INIT: Slot = Slot { flex: None };

static SLOTS: Mutex<RefCell<[Slot; NUM_SLOTS]>> = Mutex::new(RefCell::new([SLOT_INIT; NUM_SLOTS]));

/// Hand a raw `Flex`-wrapped pin to the GPIO subsystem. Boards build the
/// `Flex` once (from the corresponding `peripherals::GPIO<N>`) and pass
/// it here together with the schematic pin number; `register` then makes
/// the pin visible to the 9P tree in floating-input mode.
pub fn install_pin(pin: u8, flex: Flex<'static>) {
    if (pin as usize) >= NUM_SLOTS {
        return;
    }
    critical_section::with(|cs| {
        let mut slots = SLOTS.borrow(cs).borrow_mut();
        slots[pin as usize].flex = Some(flex);
    });
    apply_mode(pin, Mode::InFloating);
    devices::gpio::register(pin);
}

/// `/dev/gpio/<N>/ctl` writer.
pub fn on_ctl(pin: u8, line: &str) -> Result<(), &'static str> {
    let mode = devices::gpio::parse_mode(line)?;
    if devices::gpio::snapshot(pin).is_none() {
        return Err("pin not present on this board");
    }
    apply_mode(pin, mode);
    devices::gpio::set_mode(pin, mode);
    Ok(())
}

/// `/dev/gpio/<N>/level` writer.
pub fn on_level(pin: u8, line: &str) -> Result<(), &'static str> {
    let snap = devices::gpio::snapshot(pin).ok_or("pin not present on this board")?;
    if !snap.mode.is_output() {
        return Err("pin is not an output (write to ctl first)");
    }
    let level = match line.trim() {
        "0" | "low" | "off" => false,
        "1" | "high" | "on" => true,
        _ => return Err("level: 0 | 1"),
    };
    apply_output_level(pin, level)?;
    devices::gpio::set_out_level(pin, level);
    Ok(())
}

/// Sample the hardware level so the cached `in_level` the 9P layer reads
/// reflects what's actually on the pin. Outputs sample the readback of
/// their own drive; inputs sample the external state.
pub fn refresh_level(pin: u8) {
    if (pin as usize) >= NUM_SLOTS {
        return;
    }
    let level = critical_section::with(|cs| {
        let slots = SLOTS.borrow(cs).borrow();
        slots[pin as usize].flex.as_ref().map(|f| f.is_high())
    });
    if let Some(high) = level {
        devices::gpio::set_in_level(pin, high);
    }
}

fn apply_mode(pin: u8, mode: Mode) {
    let idx = pin as usize;
    if idx >= NUM_SLOTS {
        return;
    }
    critical_section::with(|cs| {
        let mut slots = SLOTS.borrow(cs).borrow_mut();
        let Some(flex) = slots[idx].flex.as_mut() else { return };
        match mode {
            Mode::InFloating => {
                flex.set_output_enable(false);
                flex.apply_input_config(&InputConfig::default().with_pull(Pull::None));
                flex.set_input_enable(true);
            }
            Mode::InPullup => {
                flex.set_output_enable(false);
                flex.apply_input_config(&InputConfig::default().with_pull(Pull::Up));
                flex.set_input_enable(true);
            }
            Mode::InPulldown => {
                flex.set_output_enable(false);
                flex.apply_input_config(&InputConfig::default().with_pull(Pull::Down));
                flex.set_input_enable(true);
            }
            Mode::PushPull => {
                flex.apply_output_config(
                    &OutputConfig::default().with_drive_mode(DriveMode::PushPull),
                );
                flex.set_level(Level::Low);
                flex.set_output_enable(true);
            }
            Mode::OpenDrain => {
                flex.apply_output_config(
                    &OutputConfig::default().with_drive_mode(DriveMode::OpenDrain),
                );
                // Open-drain idles high (released = Hi-Z).
                flex.set_level(Level::High);
                flex.set_output_enable(true);
                // Leave the input buffer on so the user can read back the
                // line state — handy when sharing the pin with another
                // open-drain talker.
                flex.set_input_enable(true);
            }
        }
    });
}

fn apply_output_level(pin: u8, level: bool) -> Result<(), &'static str> {
    let idx = pin as usize;
    if idx >= NUM_SLOTS {
        return Err("pin out of range");
    }
    critical_section::with(|cs| {
        let mut slots = SLOTS.borrow(cs).borrow_mut();
        let Some(flex) = slots[idx].flex.as_mut() else {
            return Err("pin not present on this board");
        };
        flex.set_level(if level { Level::High } else { Level::Low });
        Ok(())
    })
}
