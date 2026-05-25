//! External I²C bus 1 — `/dev/i2c/1`.
//!
//! Shared between StickS3 (Grove G9/G10) and Plus2 (Grove G32/G33). The
//! 9P handler invokes the blocking-mode I²C peripheral synchronously from
//! within the `Session::run` async task; we guard the peripheral with a
//! `critical_section::Mutex<RefCell<…>>` so concurrent sessions can't
//! interleave transactions on the bus.
//!
//! ESP-HAL's blocking I²C polls the peripheral's status registers rather
//! than relying on interrupts, so the brief IRQ-disable window from the
//! critical section is harmless to WiFi / I²S DMA / display SPI.

use core::cell::RefCell;
use critical_section::Mutex;
use esp_hal::Blocking;
use esp_hal::i2c::master::{Config as I2cConfig, Error as I2cError, I2c};
use esp_hal::time::Rate;

use devices::i2c1 as state;

static BUS: Mutex<RefCell<Option<I2c<'static, Blocking>>>> = Mutex::new(RefCell::new(None));

/// Hand the bus over to the static slot the 9P handlers borrow from. Called
/// from board-specific `spawn` once after the peripheral is built.
pub fn install(bus: I2c<'static, Blocking>) {
    critical_section::with(|cs| *BUS.borrow(cs).borrow_mut() = Some(bus));
}

/// Apply the currently configured frequency to the bus. Called lazily on
/// the first transaction so `freq N` writes don't take effect until then;
/// also re-applied each transaction so it tracks `devices::i2c1::freq_hz()`
/// without us tracking dirtiness ourselves. The cost is one register
/// write per transaction, dwarfed by the bus-level latency.
fn apply_freq(bus: &mut I2c<'static, Blocking>) -> Result<(), &'static str> {
    let cfg = I2cConfig::default().with_frequency(Rate::from_hz(state::freq_hz()));
    bus.apply_config(&cfg).map_err(|_| "bad freq")
}

fn execute(xfer: &state::Xfer) -> Result<(), &'static str> {
    critical_section::with(|cs| {
        let mut guard = BUS.borrow(cs).borrow_mut();
        let bus = guard.as_mut().ok_or("i2c1 not initialized")?;
        apply_freq(bus)?;
        match xfer {
            state::Xfer::Read { addr, count } => {
                let mut buf = [0u8; state::MAX_XFER];
                let buf = &mut buf[..*count];
                match bus.read(*addr, buf) {
                    Ok(()) => {
                        state::record_ok(buf);
                        Ok(())
                    }
                    Err(e) => {
                        let msg = err_str(e);
                        state::record_err(msg);
                        Err(msg)
                    }
                }
            }
            state::Xfer::Write { addr, data } => match bus.write(*addr, data) {
                Ok(()) => {
                    state::record_ok(&[]);
                    Ok(())
                }
                Err(e) => {
                    let msg = err_str(e);
                    state::record_err(msg);
                    Err(msg)
                }
            },
            state::Xfer::WriteRead { addr, write, read } => {
                let mut buf = [0u8; state::MAX_XFER];
                let rbuf = &mut buf[..*read];
                match bus.write_read(*addr, write, rbuf) {
                    Ok(()) => {
                        state::record_ok(rbuf);
                        Ok(())
                    }
                    Err(e) => {
                        let msg = err_str(e);
                        state::record_err(msg);
                        Err(msg)
                    }
                }
            }
        }
    })
}

/// Probe all 7-bit addresses except the reserved low (`0x00..=0x07`) and
/// high (`0x78..=0x7f`) ranges. A zero-byte write that ACKs counts as
/// presence; if it NACKs we fall back to a 1-byte read since some devices
/// (notably the FT260, MCP4725 in single-byte mode) only ACK on read.
///
/// Each address is probed inside its **own** critical section so the
/// per-NACK wait (~90 µs at 100 kHz) doesn't compound into a multi-ms
/// IRQ stall — WiFi RX, audio DMA top-ups, and other 9P traffic stay
/// responsive across the ~10 ms total scan.
pub fn scan_and_cache() {
    let mut found = [0u8; 120];
    let mut n = 0usize;
    for addr in 0x08u8..=0x77u8 {
        let acked = critical_section::with(|cs| {
            let mut guard = BUS.borrow(cs).borrow_mut();
            let Some(bus) = guard.as_mut() else { return false };
            let _ = apply_freq(bus);
            if bus.write(addr, &[]).is_ok() {
                return true;
            }
            let mut probe = [0u8; 1];
            bus.read(addr, &mut probe).is_ok()
        });
        if acked && n < found.len() {
            found[n] = addr;
            n += 1;
        }
    }
    state::record_scan(&found[..n]);
}

fn err_str(e: I2cError) -> &'static str {
    use I2cError::*;
    // Best-effort labelling; full debug printing isn't worth the format
    // weight here and most clients only care about ok/err anyway.
    match e {
        AcknowledgeCheckFailed(_) => "nack",
        ArbitrationLost => "arbitration lost",
        FifoExceeded => "fifo overflow",
        Timeout => "timeout",
        _ => "i2c error",
    }
}

// ---------------------------------------------------------------------------
// 9P FsContext glue. These functions match the `fn(...)` signatures expected
// by `ninep::fs::FsContext`, so they can be installed directly without a
// closure-allocation.

pub fn on_ctl(line: &str) -> Result<(), &'static str> {
    match state::parse_ctl(line)? {
        None => Ok(()),
        Some(xfer) => execute(&xfer),
    }
}

pub fn on_data(line: &str) -> Result<(), &'static str> {
    match state::parse_xfer(line)? {
        None => Ok(()),
        Some(xfer) => execute(&xfer),
    }
}

pub fn read_scan(off: u64, buf: &mut [u8]) -> usize {
    // A fresh probe is run only on offset 0; subsequent reads (continuation
    // of a long line list) serve from the cache. This keeps `cat` cheap
    // when the listing exceeds a single 9P Tread payload.
    if off == 0 {
        scan_and_cache();
    }
    state::read_scan(off, buf)
}
