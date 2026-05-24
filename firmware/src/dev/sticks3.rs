//! M5Stack StickS3 (board-sticks3) device bring-up.
//!
//! Stage 1: M5PM1 PMIC init on I²C0.
//! Stage 2: ST7789P3 135×240 LCD on SPI2 with the framebuffer + ctl surface
//!          shared with Plus2 (`/dev/display/{ctl,fb,brightness,info}`).
//!
//! The single status LED is **autonomously driven by the M5PM1 firmware** for
//! power/charge indication and is not user-controllable from our side; see
//! `ISSUES.md` § `/dev/led` for the full investigation.

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_println::println;

extern crate alloc;
use alloc::boxed::Box;

use devices::display;

/// Signalled by `m5pm1_task` after the L3B rail (LCD backlight, mic, spk) is
/// enabled. The display task waits on this before driving SPI so that the
/// panel isn't talked to while its power rail is still floating.
static L3B_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub fn spawn(
    spawner: &Spawner,
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO47<'static>,
    scl: esp_hal::peripherals::GPIO48<'static>,
    spi2: esp_hal::peripherals::SPI2<'static>,
    lcd_mosi: esp_hal::peripherals::GPIO39<'static>,
    lcd_sck: esp_hal::peripherals::GPIO40<'static>,
    lcd_dc: esp_hal::peripherals::GPIO45<'static>,
    lcd_cs: esp_hal::peripherals::GPIO41<'static>,
    lcd_rst: esp_hal::peripherals::GPIO21<'static>,
    lcd_bl: esp_hal::peripherals::GPIO38<'static>,
) {
    let fb = Box::new([0u8; display::FB_LEN]);
    let fb: &'static mut [u8; display::FB_LEN] = Box::leak(fb);
    display::init(fb);
    display::splash_booting(crate::board::BOARD_NAME);

    spawner.spawn(m5pm1_task(i2c0, sda, scl).unwrap());
    spawner.spawn(
        display_task(spi2, lcd_mosi, lcd_sck, lcd_dc, lcd_cs, lcd_rst, lcd_bl).unwrap(),
    );
}

// ---------------------------------------------------------------------------
// M5PM1 (I²C 0x6E) — minimal init so the chip is in a known state for later
// stages (battery readouts, IMU INT, watchdog). The LED is owned by M5PM1
// firmware; see ISSUES.md.
// ---------------------------------------------------------------------------

const M5PM1_ADDR: u8 = 0x6E;

#[allow(dead_code)]
mod reg {
    pub const DEVICE_ID: u8 = 0x00;
    pub const HW_REV: u8 = 0x02;
    pub const SW_REV: u8 = 0x03;
    pub const PWR_CFG: u8 = 0x06;
    pub const I2C_CFG: u8 = 0x09;
    pub const GPIO_MODE: u8 = 0x10;
    pub const GPIO_OUT: u8 = 0x11;
    pub const GPIO_DRV: u8 = 0x13;
    pub const GPIO_FUNC0: u8 = 0x16; // covers GPIO0+GPIO1 (2 bits each)
    pub const GPIO_FUNC1: u8 = 0x17; // covers GPIO2+GPIO3 (2 bits each)
    pub const VBAT_L: u8 = 0x22;
    pub const VBAT_H: u8 = 0x23;
}

#[embassy_executor::task]
async fn m5pm1_task(
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO47<'static>,
    scl: esp_hal::peripherals::GPIO48<'static>,
) {
    let i2c = I2c::new(i2c0, I2cConfig::default())
        .unwrap()
        .with_sda(sda)
        .with_scl(scl);
    let mut pm = M5Pm1 { i2c };

    if pm.init().is_err() {
        println!("m5pm1: init failed");
        loop {
            Timer::after(Duration::from_secs(5)).await;
        }
    }

    if pm.enable_l3b().is_err() {
        println!("m5pm1: L3B enable failed (LCD/mic/spk may stay off)");
    } else {
        println!("m5pm1: L3B rail enabled (LCD BL / mic / spk)");
    }
    L3B_READY.signal(());

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

struct M5Pm1<I2C> {
    i2c: I2C,
}

impl<I2C> M5Pm1<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    fn init(&mut self) -> Result<(), ()> {
        let id = self.read_reg(reg::DEVICE_ID)?;
        let hw = self.read_reg(reg::HW_REV).unwrap_or(0xff);
        let sw = self.read_reg(reg::SW_REV).unwrap_or(0xff);
        println!(
            "m5pm1: DEVICE_ID={:#04x} HW={:#04x} SW={:#04x} (LED is PMIC-internal)",
            id, hw, sw
        );

        // Disable I²C auto-sleep so the chip never NACKs writes between
        // transactions. Bits [3:0] = 0 (disabled); bit 4 = 0 (100 kHz host).
        self.write_reg(reg::I2C_CFG, 0x00)?;
        Ok(())
    }

    /// Enable the L3B rail (LCD backlight + mic + speaker amp).
    ///
    /// On StickS3 the L3B regulator's EN line is wired to M5PM1 `PYG2`.
    /// The Arduino reference library configures PYG2 in **OTHER** function
    /// (FUNC1 bits [1:0] = 0b11) so the chip routes its internal L3B_EN
    /// signal to the pin. We also force the underlying GPIO direction to
    /// output and the output latch high so the rail comes up even when the
    /// pin is read back as plain GPIO (which is what most current firmware
    /// rev. 0x4f appears to expose).
    fn enable_l3b(&mut self) -> Result<(), ()> {
        let mut func1 = self.read_reg(reg::GPIO_FUNC1).unwrap_or(0);
        func1 &= !0b11;
        func1 |= 0b11; // OTHER / L3B_EN
        self.write_reg(reg::GPIO_FUNC1, func1)?;

        let mode = self.read_reg(reg::GPIO_MODE)? | (1 << 2);
        self.write_reg(reg::GPIO_MODE, mode)?;

        let out = self.read_reg(reg::GPIO_OUT)? | (1 << 2);
        self.write_reg(reg::GPIO_OUT, out)?;

        // Push-pull drive so the EN line actually pulls high (open-drain
        // alone would only sink).
        let drv = self.read_reg(reg::GPIO_DRV)? & !(1 << 2);
        self.write_reg(reg::GPIO_DRV, drv)?;

        let m = self.read_reg(reg::GPIO_MODE).unwrap_or(0xff);
        let o = self.read_reg(reg::GPIO_OUT).unwrap_or(0xff);
        let f = self.read_reg(reg::GPIO_FUNC1).unwrap_or(0xff);
        println!(
            "m5pm1: L3B regs MODE={:#04x} OUT={:#04x} FUNC1={:#04x}",
            m, o, f
        );
        Ok(())
    }

    fn write_reg(&mut self, reg: u8, val: u8) -> Result<(), ()> {
        self.i2c.write(M5PM1_ADDR, &[reg, val]).map_err(|_| ())
    }

    fn read_reg(&mut self, reg: u8) -> Result<u8, ()> {
        let mut b = [0u8];
        self.i2c
            .write_read(M5PM1_ADDR, &[reg], &mut b)
            .map_err(|_| ())?;
        Ok(b[0])
    }
}

// ---------------------------------------------------------------------------
// ST7789P3 135×240 LCD over SPI2.
// Pins (M5Stick S3 schematic v0.6, 2025-11-11):
//   MOSI=G39, SCK=G40, RS/DC=G45, CS=G41, RST=G21, BL=G38
// The 135×240 IPS panel sits on the ST7789 controller's 240×320 frame with
// the same 52,40 offset as Plus2's ST7789V2.
// ---------------------------------------------------------------------------

#[embassy_executor::task]
async fn display_task(
    spi2: esp_hal::peripherals::SPI2<'static>,
    mosi: esp_hal::peripherals::GPIO39<'static>,
    sck: esp_hal::peripherals::GPIO40<'static>,
    dc: esp_hal::peripherals::GPIO45<'static>,
    cs: esp_hal::peripherals::GPIO41<'static>,
    rst: esp_hal::peripherals::GPIO21<'static>,
    bl: esp_hal::peripherals::GPIO38<'static>,
) {
    use display_interface_spi::SPIInterface;
    use embedded_graphics_core::pixelcolor::{raw::RawU16, Rgb565};
    use embedded_hal_bus::spi::ExclusiveDevice;
    use esp_hal::spi::master::{Config as SpiConfig, Spi};
    use esp_hal::spi::Mode;
    use esp_hal::time::Rate;
    use mipidsi::{
        models::ST7789,
        options::{ColorInversion, ColorOrder, Orientation, Rotation},
        Builder,
    };

    let cs_pin = Output::new(cs, Level::High, OutputConfig::default());
    let dc_pin = Output::new(dc, Level::Low, OutputConfig::default());
    let rst_pin = Output::new(rst, Level::High, OutputConfig::default());
    // Start BL low; we'll bring it up after the L3B rail is enabled.
    let mut bl_pin = Output::new(bl, Level::Low, OutputConfig::default());

    // Block here until M5PM1 has switched on the L3B rail (LCD/MIC/SPK).
    // Without this the SPI talks to a panel with no analog supply, the
    // backlight is off, and the user sees a black screen even though the
    // controller is happily ACKing commands.
    L3B_READY.wait().await;
    Timer::after(Duration::from_millis(50)).await;
    bl_pin.set_high();
    println!("display: L3B up, BL high");

    let spi_cfg = SpiConfig::default()
        .with_frequency(Rate::from_mhz(40))
        .with_mode(Mode::_0);
    let spi = Spi::new(spi2, spi_cfg)
        .expect("spi")
        .with_sck(sck)
        .with_mosi(mosi);

    let spi_dev = ExclusiveDevice::new_no_delay(spi, cs_pin).expect("spi dev");
    let di = SPIInterface::new(spi_dev, dc_pin);

    let mut delay = EmbDelay;
    let mut display = Builder::new(ST7789, di)
        .reset_pin(rst_pin)
        .color_order(ColorOrder::Rgb)
        .invert_colors(ColorInversion::Inverted)
        .orientation(Orientation::new().rotate(Rotation::Deg0))
        .display_size(display::WIDTH as u16, display::HEIGHT as u16)
        .display_offset(52, 40)
        .init(&mut delay)
        .expect("lcd init");

    println!("display: ST7789P3 ok (CS=G41 RST=G21 BL=G38)");

    // Prove SPI path before 9P: solid green frame for 250 ms then black.
    {
        let w = display::WIDTH as u16;
        let h = display::HEIGHT as u16;
        let green = Rgb565::new(0, 63, 0);
        let pixels = core::iter::repeat(green).take(display::FB_LEN / 2);
        match display.set_pixels(0, 0, w - 1, h - 1, pixels) {
            Ok(()) => println!("display: self-test green ok"),
            Err(e) => println!("display: self-test err {:?}", e),
        }
    }
    Timer::after(Duration::from_millis(250)).await;

    loop {
        let on = devices::display::is_on();
        if !on {
            bl_pin.set_low();
        } else {
            bl_pin.set_high();
        }

        if devices::display::take_dirty() && on {
            devices::display::with_fb(|fb| {
                let w = display::WIDTH as u16;
                let h = display::HEIGHT as u16;
                let pixels = fb.chunks_exact(2).map(|chunk| {
                    let raw = u16::from_le_bytes([chunk[0], chunk[1]]);
                    Rgb565::from(RawU16::new(raw))
                });
                if display.set_pixels(0, 0, w - 1, h - 1, pixels).is_err() {
                    println!("display: flush err");
                }
            });
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

struct EmbDelay;

impl embedded_hal::delay::DelayNs for EmbDelay {
    fn delay_ns(&mut self, ns: u32) {
        embassy_time::block_for(Duration::from_nanos(ns as u64));
    }
}
