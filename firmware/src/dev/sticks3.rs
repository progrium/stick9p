//! M5Stack StickS3 (board-sticks3) device bring-up.
//!
//! Stage 1: M5PM1 PMIC init on I²C0.
//! Stage 2: ST7789P3 135×240 LCD on SPI2 with the shared framebuffer + ctl
//!          surface (`/dev/display/{ctl,fb,brightness,info}`),
//!          BMI270 IMU on the same I²C0 bus (`/dev/imu/{accel,gyro,ctl}`),
//!          GPIO11/GPIO12 user buttons (`/dev/buttons/{a,b,event,ctl}`), and
//!          VBAT polled from M5PM1 (`/dev/power/{battery,vbat_mv}`).
//!
//! The single status LED is **autonomously driven by the M5PM1 firmware** for
//! power/charge indication and is not user-controllable from our side; see
//! `ISSUES.md` § `/dev/led` for the full investigation.

use core::cell::RefCell;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_println::println;

extern crate alloc;
use alloc::boxed::Box;

use devices::{buzzer, display, spk};

/// Signalled by the I²C task after L3B rail (LCD/MIC/SPK) is up so the display
/// task only drives SPI once the panel's analog supply is live.
static L3B_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signalled by the I²C task after ES8311 has been programmed and the AW8737
/// amp has been enabled, so the audio task only starts I²S DMA once the codec
/// is ready to receive samples.
static AUDIO_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub fn spawn(
    spawner: &Spawner,
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO47<'static>,
    scl: esp_hal::peripherals::GPIO48<'static>,
    btn_a: esp_hal::peripherals::GPIO11<'static>, // KEY1 BtnA (schematic v0.6)
    btn_b: esp_hal::peripherals::GPIO12<'static>, // KEY2 BtnB
    spi2: esp_hal::peripherals::SPI2<'static>,
    lcd_mosi: esp_hal::peripherals::GPIO39<'static>,
    lcd_sck: esp_hal::peripherals::GPIO40<'static>,
    lcd_dc: esp_hal::peripherals::GPIO45<'static>,
    lcd_cs: esp_hal::peripherals::GPIO41<'static>,
    lcd_rst: esp_hal::peripherals::GPIO21<'static>,
    lcd_bl: esp_hal::peripherals::GPIO38<'static>,
    i2s0: esp_hal::peripherals::I2S0<'static>,
    dma_i2s0: esp_hal::peripherals::DMA_CH0<'static>,
    i2s_mclk: esp_hal::peripherals::GPIO18<'static>,
    i2s_bclk: esp_hal::peripherals::GPIO17<'static>,
    i2s_lrck: esp_hal::peripherals::GPIO15<'static>,
    i2s_dout: esp_hal::peripherals::GPIO14<'static>,
    i2c1: esp_hal::peripherals::I2C1<'static>,
    i2c1_sda: esp_hal::peripherals::GPIO9<'static>,
    i2c1_scl: esp_hal::peripherals::GPIO10<'static>,
    gpio1: esp_hal::peripherals::GPIO1<'static>,
    gpio2: esp_hal::peripherals::GPIO2<'static>,
    gpio3: esp_hal::peripherals::GPIO3<'static>,
    gpio4: esp_hal::peripherals::GPIO4<'static>,
    gpio5: esp_hal::peripherals::GPIO5<'static>,
    gpio6: esp_hal::peripherals::GPIO6<'static>,
    gpio7: esp_hal::peripherals::GPIO7<'static>,
    gpio8: esp_hal::peripherals::GPIO8<'static>,
) {
    let fb = Box::new([0u8; display::FB_LEN]);
    let fb: &'static mut [u8; display::FB_LEN] = Box::leak(fb);
    display::init(fb);
    display::splash_booting(crate::board::BOARD_NAME);

    spawner.spawn(i2c_task(i2c0, sda, scl).unwrap());
    spawner.spawn(buttons_task(btn_a, btn_b).unwrap());
    spawner.spawn(
        display_task(spi2, lcd_mosi, lcd_sck, lcd_dc, lcd_cs, lcd_rst, lcd_bl).unwrap(),
    );
    spawner.spawn(
        audio_task(i2s0, dma_i2s0, i2s_mclk, i2s_bclk, i2s_lrck, i2s_dout).unwrap(),
    );

    // External I²C bus 1 (Grove HY2.0 PORT.A: SDA=G9, SCL=G10). The bus is
    // built once, handed to the shared `dev::i2c1` glue, and from there is
    // owned by the `/dev/i2c/1` 9P handlers (no driver task — transactions
    // happen synchronously in the 9P session).
    let bus1 = I2c::new(i2c1, I2cConfig::default())
        .unwrap()
        .with_sda(i2c1_sda)
        .with_scl(i2c1_scl);
    crate::dev::i2c1::install(bus1);

    // Claimable Hat2-Bus GPIOs G1..G8 — wrapped in `Flex` so the user
    // can switch each pin between input and output at runtime via
    // `/dev/gpio/<N>/ctl`. We *don't* expose G9/G10 (Grove I²C) or
    // G43/G44 (legacy UART pins reused for boot diagnostics).
    use esp_hal::gpio::Flex;
    crate::dev::gpio::install_pin(1, Flex::new(gpio1));
    crate::dev::gpio::install_pin(2, Flex::new(gpio2));
    crate::dev::gpio::install_pin(3, Flex::new(gpio3));
    crate::dev::gpio::install_pin(4, Flex::new(gpio4));
    crate::dev::gpio::install_pin(5, Flex::new(gpio5));
    crate::dev::gpio::install_pin(6, Flex::new(gpio6));
    crate::dev::gpio::install_pin(7, Flex::new(gpio7));
    crate::dev::gpio::install_pin(8, Flex::new(gpio8));

    // Once the audio path is up we want the boot fanfare to fire (same two-beep
    // pattern as Plus2 — sequenced through `buzzer::take_done_fanfare` so the
    // 9P /dev/buzzer/ctl surface keeps working uniformly across boards).
    buzzer::request_stage2_done();
}

// ---------------------------------------------------------------------------
// I²C0 shared bus: M5PM1 (0x6E) + BMI270 (0x68) [+ ES8311 (0x18) once audio
// lands]. All access funnels through this one task so we don't need a mutex
// on the bus; embedded-hal-bus's `RefCellDevice` gives each driver a
// non-overlapping `embedded_hal::i2c::I2c` view of the underlying peripheral.
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
    pub const GPIO_FUNC0: u8 = 0x16;
    pub const GPIO_FUNC1: u8 = 0x17;
    pub const VBAT_L: u8 = 0x22;
    pub const VBAT_H: u8 = 0x23;
}

#[embassy_executor::task]
async fn i2c_task(
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO47<'static>,
    scl: esp_hal::peripherals::GPIO48<'static>,
) {
    use bmi2::{
        config::BMI270_CONFIG_FILE,
        types::{
            AccBwp, AccConf, AccRange, Burst, GyrBwp, GyrConf, GyrRange, GyrRangeVal, Odr,
            OisRange, PerfMode, PwrCtrl,
        },
        Bmi2, I2cAddr,
    };
    use embedded_hal_bus::i2c::RefCellDevice;

    let i2c_raw = I2c::new(i2c0, I2cConfig::default())
        .unwrap()
        .with_sda(sda)
        .with_scl(scl);
    let i2c_cell = RefCell::new(i2c_raw);

    // --- M5PM1: identify + enable L3B rail (gates LCD BL, mic, spk) -------
    let mut pm = M5Pm1 {
        i2c: RefCellDevice::new(&i2c_cell),
    };
    if pm.init().is_err() {
        println!("m5pm1: init failed");
    }
    if pm.enable_l3b().is_err() {
        println!("m5pm1: L3B enable failed (LCD/mic/spk may stay off)");
    } else {
        println!("m5pm1: L3B rail enabled (LCD BL / mic / spk)");
    }
    L3B_READY.signal(());

    // --- BMI270: chip-id + 8 KB config blob upload + ACC/GYR enable ------
    let mut bmi = Bmi2::<_, _, 512>::new_i2c(
        RefCellDevice::new(&i2c_cell),
        EmbDelay,
        I2cAddr::Default,
        Burst::new(255),
    );
    // --- ES8311 (audio codec on the same I²C bus, 0x18) -------------------
    let mut es = Es8311 {
        i2c: RefCellDevice::new(&i2c_cell),
    };
    let audio_ok = match es.init_16k_mono_from_mclk() {
        Ok(()) => {
            // Plain GPIO high on M5PM1 PYG3 enables the AW8737 amp rail.
            if pm.enable_spk_amp().is_err() {
                println!("m5pm1: SPK amp enable failed");
            } else {
                println!("m5pm1: AW8737 amp enabled (PYG3 high)");
            }
            AUDIO_READY.signal(());
            true
        }
        Err(_) => {
            println!("es8311: init failed");
            false
        }
    };
    let _ = audio_ok;

    // --- BMI270 ------------------------------------------------------------
    let imu_ok = match bmi.get_chip_id() {
        Ok(id) => {
            println!("bmi270: CHIP_ID={:#04x} (expect 0x24)", id);
            match bmi.init(&BMI270_CONFIG_FILE) {
                Ok(_) => {
                    if let Err(_) = bmi.set_pwr_ctrl(PwrCtrl {
                        aux_en: false,
                        gyr_en: true,
                        acc_en: true,
                        temp_en: false,
                    }) {
                        println!("bmi270: pwr_ctrl err");
                    }
                    let _ = bmi.set_acc_conf(AccConf {
                        odr: Odr::Odr100,
                        bwp: AccBwp::Osr2Avg2,
                        filter_perf: PerfMode::Perf,
                    });
                    let _ = bmi.set_acc_range(AccRange::Range4g);
                    let _ = bmi.set_gyr_conf(GyrConf {
                        odr: Odr::Odr100,
                        bwp: GyrBwp::Osr2,
                        filter_perf: PerfMode::Perf,
                        noise_perf: PerfMode::Perf,
                    });
                    let _ = bmi.set_gyr_range(GyrRange {
                        range: GyrRangeVal::Range1000,
                        ois_range: OisRange::Range250,
                    });
                    println!("bmi270: init ok (acc=±4g, gyr=±1000dps, ODR=100Hz)");
                    true
                }
                Err(_) => {
                    println!("bmi270: init blob upload failed");
                    false
                }
            }
        }
        Err(_) => {
            println!("bmi270: chip-id read failed (NACK?)");
            false
        }
    };

    // --- Steady-state poll: IMU at requested rate, VBAT once per second ---
    // Use Option<Instant> so the first iteration always reads VBAT (rather
    // than relying on Instant::now() − some duration, which underflows at
    // boot before 5 s have actually elapsed).
    let mut last_vbat: Option<Instant> = None;
    loop {
        let hz = devices::imu::rate_hz().max(1) as u64;
        let period_ms = (1000 / hz).max(2);

        if imu_ok {
            if let Ok(data) = bmi.get_data() {
                // Counts at ±4 g: 16384 LSB ≈ 4 g → 1 mg ≈ 16384/4000 ≈ 4.096 LSB
                // So mg = raw * 4000 / 16384.
                devices::imu::push_accel(
                    data.acc.x as i32 * 4000 / 16384,
                    data.acc.y as i32 * 4000 / 16384,
                    data.acc.z as i32 * 4000 / 16384,
                );
                // Gyro raw is signed 16-bit at ±1000 dps full-scale.
                // millidegrees-per-second = raw * 1000_000 / 32768 ≈ raw * 30.5.
                devices::imu::push_gyro(
                    data.gyr.x as i32 * 1000000 / 32768,
                    data.gyr.y as i32 * 1000000 / 32768,
                    data.gyr.z as i32 * 1000000 / 32768,
                );
            }
        }

        let due = match last_vbat {
            None => true,
            Some(t) => t.elapsed() >= Duration::from_secs(1),
        };
        if due {
            if let Ok(mv) = pm.read_vbat_mv() {
                devices::power::set_vbat_mv(mv);
            }
            last_vbat = Some(Instant::now());
        }

        Timer::after(Duration::from_millis(period_ms)).await;
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
        self.write_reg(reg::I2C_CFG, 0x00)?;
        Ok(())
    }

    /// Enable the L3B rail (LCD backlight + mic + speaker amp). See DESIGN
    /// "Stage 2 — sensors & display" for the ordering quirk.
    fn enable_l3b(&mut self) -> Result<(), ()> {
        let mut func1 = self.read_reg(reg::GPIO_FUNC1).unwrap_or(0);
        func1 &= !0b11;
        func1 |= 0b11;
        self.write_reg(reg::GPIO_FUNC1, func1)?;

        let mode = self.read_reg(reg::GPIO_MODE)? | (1 << 2);
        self.write_reg(reg::GPIO_MODE, mode)?;

        let out = self.read_reg(reg::GPIO_OUT)? | (1 << 2);
        self.write_reg(reg::GPIO_OUT, out)?;

        let drv = self.read_reg(reg::GPIO_DRV)? & !(1 << 2);
        self.write_reg(reg::GPIO_DRV, drv)?;
        Ok(())
    }

    /// Enable the AW8737 speaker amplifier by driving M5PM1 PYG3 high.
    /// Per the M5PM1 docs the SPK amp is gated by a plain GPIO output on PYG3
    /// (not the `OTHER`/`PYG3_SPK_Pulse` function — that name is just the
    /// schematic label).
    fn enable_spk_amp(&mut self) -> Result<(), ()> {
        // FUNC1 bits[3:2] = GPIO3 function: 0b00 = GPIO, 0b11 = OTHER.
        let mut func1 = self.read_reg(reg::GPIO_FUNC1).unwrap_or(0);
        func1 &= !(0b11 << 2);
        self.write_reg(reg::GPIO_FUNC1, func1)?;

        let mode = self.read_reg(reg::GPIO_MODE)? | (1 << 3);
        self.write_reg(reg::GPIO_MODE, mode)?;

        let drv = self.read_reg(reg::GPIO_DRV)? & !(1 << 3); // push-pull
        self.write_reg(reg::GPIO_DRV, drv)?;

        let out = self.read_reg(reg::GPIO_OUT)? | (1 << 3);
        self.write_reg(reg::GPIO_OUT, out)?;
        Ok(())
    }

    /// Read the battery voltage in mV from M5PM1 regs 0x22/0x23 (little-endian).
    fn read_vbat_mv(&mut self) -> Result<u32, ()> {
        let lo = self.read_reg(reg::VBAT_L)? as u32;
        let hi = self.read_reg(reg::VBAT_H)? as u32;
        Ok((hi << 8) | lo)
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
// Buttons — KEY1 = G11 (BtnA), KEY2 = G12 (BtnB). Active-low, internal pull-up.
// The side power/reset button is wired through M5PM1 PWR_BTN and is *not*
// reachable from an ESP32 GPIO, so we only expose two buttons here.
// ---------------------------------------------------------------------------

#[embassy_executor::task]
async fn buttons_task(
    btn_a: esp_hal::peripherals::GPIO11<'static>,
    btn_b: esp_hal::peripherals::GPIO12<'static>,
) {
    let a = Input::new(btn_a, InputConfig::default().with_pull(Pull::Up));
    let b = Input::new(btn_b, InputConfig::default().with_pull(Pull::Up));
    let mut prev_a = a.level() == Level::Low;
    let mut prev_b = b.level() == Level::Low;
    devices::buttons::set_a(prev_a);
    devices::buttons::set_b(prev_b);

    loop {
        let a_down = a.level() == Level::Low;
        let b_down = b.level() == Level::Low;
        devices::buttons::set_a(a_down);
        devices::buttons::set_b(b_down);
        if a_down != prev_a {
            devices::buttons::push_event('a', a_down);
            prev_a = a_down;
        }
        if b_down != prev_b {
            devices::buttons::push_event('b', b_down);
            prev_b = b_down;
        }
        Timer::after(Duration::from_millis(20)).await;
    }
}

// ---------------------------------------------------------------------------
// ST7789P3 135×240 LCD over SPI2.
// Pins (StickS3 schematic v0.6, 2025-11-11):
//   MOSI=G39, SCK=G40, RS/DC=G45, CS=G41, RST=G21, BL=G38
// 135×240 IPS panel sits on the ST7789 controller's 240×320 frame with the
// same 52/40 offset as Plus2's ST7789V2.
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
    let mut bl_pin = Output::new(bl, Level::Low, OutputConfig::default());

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

// ---------------------------------------------------------------------------
// ES8311 codec (I²C 0x18). Minimal init for 16 kHz mono playback with MCLK
// supplied by the ESP at 256 × Fs = 4.096 MHz. Register values match the
// `coeff_div` row for {mclk=4096000, rate=16000} in Espressif's reference
// driver (`audio_hal/driver/es8311/es8311.c`), inlined here so the boot path
// stays in one file and we don't pull in the whole codec_dev C component.
// ---------------------------------------------------------------------------

const ES8311_ADDR: u8 = 0x18;

#[allow(dead_code)]
mod es8311_reg {
    pub const RESET: u8 = 0x00;
    pub const CLK_MGR1: u8 = 0x01;
    pub const CLK_MGR2: u8 = 0x02;
    pub const CLK_MGR3: u8 = 0x03;
    pub const CLK_MGR4: u8 = 0x04;
    pub const CLK_MGR5: u8 = 0x05;
    pub const CLK_MGR6: u8 = 0x06;
    pub const CLK_MGR7: u8 = 0x07;
    pub const CLK_MGR8: u8 = 0x08;
    pub const SDPIN: u8 = 0x09;
    pub const SDPOUT: u8 = 0x0A;
    pub const SYSTEM_0B: u8 = 0x0B;
    pub const SYSTEM_0C: u8 = 0x0C;
    pub const SYSTEM_0D: u8 = 0x0D;
    pub const SYSTEM_0E: u8 = 0x0E;
    pub const SYSTEM_10: u8 = 0x10;
    pub const SYSTEM_11: u8 = 0x11;
    pub const SYSTEM_12: u8 = 0x12;
    pub const SYSTEM_13: u8 = 0x13;
    pub const SYSTEM_14: u8 = 0x14;
    pub const ADC_15: u8 = 0x15;
    pub const ADC_16: u8 = 0x16;
    pub const ADC_17: u8 = 0x17;
    pub const ADC_1B: u8 = 0x1B;
    pub const ADC_1C: u8 = 0x1C;
    pub const DAC_31: u8 = 0x31;
    pub const DAC_32: u8 = 0x32;
    pub const DAC_37: u8 = 0x37;
    pub const GPIO_44: u8 = 0x44;
    pub const GP_45: u8 = 0x45;
}

struct Es8311<I2C> {
    i2c: I2C,
}

impl<I2C> Es8311<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    fn write(&mut self, reg: u8, val: u8) -> Result<(), ()> {
        self.i2c.write(ES8311_ADDR, &[reg, val]).map_err(|_| ())
    }

    fn read(&mut self, reg: u8) -> Result<u8, ()> {
        let mut b = [0u8];
        self.i2c
            .write_read(ES8311_ADDR, &[reg], &mut b)
            .map_err(|_| ())?;
        Ok(b[0])
    }

    /// Bring up the codec for 16 kHz / 16-bit / mono playback with MCLK from
    /// the I²S master at 256 × Fs (= 4.096 MHz). After this returns, feeding
    /// audio over I²S TX will reach the AW8737 amp.
    fn init_16k_mono_from_mclk(&mut self) -> Result<(), ()> {
        use es8311_reg as r;

        // Common opening dance straight from the Espressif reference driver:
        // enable clock manager + sysctl rails, then take the chip out of reset.
        self.write(r::CLK_MGR1, 0x30)?;
        self.write(r::CLK_MGR2, 0x00)?;
        self.write(r::CLK_MGR3, 0x10)?;
        self.write(r::ADC_16, 0x24)?;
        self.write(r::CLK_MGR4, 0x10)?;
        self.write(r::CLK_MGR5, 0x00)?;
        self.write(r::SYSTEM_0B, 0x00)?;
        self.write(r::SYSTEM_0C, 0x00)?;
        self.write(r::SYSTEM_10, 0x1F)?;
        self.write(r::SYSTEM_11, 0x7F)?;
        self.write(r::RESET, 0x80)?;

        // Slave mode (ES8311 is the I²S slave; ESP is master).
        let r0 = self.read(r::RESET).unwrap_or(0);
        self.write(r::RESET, r0 & 0xBF)?;
        self.write(r::CLK_MGR1, 0x3F)?;

        // MCLK comes from MCLK pin (not BCLK).
        let r1 = self.read(r::CLK_MGR1).unwrap_or(0);
        self.write(r::CLK_MGR1, r1 & 0x7F)?;

        // Clock dividers for {MCLK = 4.096 MHz, Fs = 16 kHz}:
        //   pre_div=1, mult=1, adc_div=1, dac_div=1, fs_mode=0,
        //   lrck_h=0x00, lrck_l=0xFF, bclk_div=4, adc_osr=0x10, dac_osr=0x10
        let r2 = self.read(r::CLK_MGR2).unwrap_or(0) & 0x07;
        self.write(r::CLK_MGR2, r2)?; // (1-1)<<5 | 0<<3
        let r5 = self.read(r::CLK_MGR5).unwrap_or(0) & 0x00;
        self.write(r::CLK_MGR5, r5)?; // adc_div-1=0, dac_div-1=0
        let r3 = self.read(r::CLK_MGR3).unwrap_or(0) & 0x80;
        self.write(r::CLK_MGR3, r3 | 0x10)?; // adc_osr=0x10
        let r4 = self.read(r::CLK_MGR4).unwrap_or(0) & 0x80;
        self.write(r::CLK_MGR4, r4 | 0x10)?; // dac_osr=0x10
        let r7 = self.read(r::CLK_MGR7).unwrap_or(0) & 0xC0;
        self.write(r::CLK_MGR7, r7)?; // lrck_h=0
        self.write(r::CLK_MGR8, 0xFF)?; // lrck_l=0xFF
        let r6 = self.read(r::CLK_MGR6).unwrap_or(0) & 0xE0;
        self.write(r::CLK_MGR6, r6 | ((4 - 1) & 0x1F))?; // bclk_div=4

        // I²S format: Philips 16-bit / 16-bit (DAC SDPIN word len = 16).
        let sdpin = self.read(r::SDPIN).unwrap_or(0);
        self.write(r::SDPIN, sdpin & 0xE3)?; // bits[4:2]=000 → 16-bit
        let sdpout = self.read(r::SDPOUT).unwrap_or(0);
        self.write(r::SDPOUT, sdpout & 0xE3)?; // mic word len 16-bit

        // Common DAC bring-up from the reference es8311_start():
        //   power up DAC + speaker driver, unmute, ramp to default gain.
        self.write(r::SYSTEM_13, 0x10)?;
        self.write(r::ADC_1B, 0x0A)?;
        self.write(r::ADC_1C, 0x6A)?;
        self.write(r::GPIO_44, 0x08)?;

        self.write(r::DAC_32, 0xBF)?; // DAC volume max
        self.write(r::SYSTEM_0E, 0x02)?;
        self.write(r::SYSTEM_12, 0x00)?;
        self.write(r::SYSTEM_14, 0x1A)?;
        self.write(r::SYSTEM_0D, 0x01)?;
        self.write(r::ADC_15, 0x40)?;
        self.write(r::DAC_37, 0x48)?;
        self.write(r::GP_45, 0x00)?;

        // Default DAC volume — 0xBF is full-scale. We sit close to it (0xB8 ≈
        // −2 dB) so a software gain of 256 (unity) is already audibly loud
        // out of the AW8737. Clients can drop the software gain via
        // `/dev/spk/ctl gain <q8>` if needed.
        self.write(r::DAC_32, 0xB8)?;
        println!("es8311: init ok (16 kHz mono, MCLK=4.096 MHz, slave)");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// I²S TX audio path: drives the ES8311 DAC at 16 kHz / 16-bit stereo
// (mono content duplicated to both slots).
//
// MCLK=G18, BCLK=G17, LRCK=G15, DOUT=G14; DIN=G16 left unused for now (mic).
//
// Uses **circular DMA** via `dma_circular_buffers!` so BCLK never stops —
// avoids the inter-chunk clicks/tremolo of one-shot transfers. The DMA
// keeps reading from a 4 KiB stereo ring at 16 kHz/256 = 16 KiB/s per
// channel = 64 KiB/s total, and `push_with` refills it from either the
// boot fanfare mono buffer or `/dev/spk/pcm` mono samples. Mono → stereo
// expansion + software gain happens inside the closure so the DMA buffer
// is never silent unless intentional.
//
// Buffer alignment matters: `dma_circular_buffers!` lays out a static
// `[u32; N/4]` so the DMA controller gets word-aligned reads. An earlier
// `Box::leak` approach with a `Vec<u8>` wedged the chip because heap
// allocations are byte-aligned and circular reads at high rate require
// word alignment on ESP32-S3.
// ---------------------------------------------------------------------------

const SAMPLE_RATE_HZ: u32 = 16000;

/// Length of the precomputed fanfare buffer: two beeps (880 Hz 120 ms and
/// 1175 Hz 180 ms) separated by a 60 ms gap, **mono** s16le (expanded to
/// stereo at push time so we share the same path as `/dev/spk/pcm`).
const FANFARE_TONE_A_MS: u32 = 120;
const FANFARE_GAP_MS: u32 = 60;
const FANFARE_TONE_B_MS: u32 = 180;
const FANFARE_TOTAL_MS: u32 = FANFARE_TONE_A_MS + FANFARE_GAP_MS + FANFARE_TONE_B_MS;
const FANFARE_FRAMES: usize = (SAMPLE_RATE_HZ as usize) * (FANFARE_TOTAL_MS as usize) / 1000;
const FANFARE_MONO_BYTES: usize = FANFARE_FRAMES * 2;

/// Circular DMA ring size in bytes (stereo s16le).
///
/// **Must be a multiple of 12** so that esp-hal's circular split (3
/// descriptors of `len/3 + len%3` bytes when `len ≤ chunk_size*2`) lands on
/// stereo-frame (4-byte) boundaries. 6144 ≈ 96 ms @ 16 kHz with three
/// 2048-byte descriptors — anything off this boundary makes `push_with`
/// hand back slices that begin mid-frame, which scrambles L/R into a loud
/// metallic high-frequency aliasing tone.
const DMA_RING_BYTES: usize = 6144;
/// Scratch for one mono drain from the spk ring. 512 bytes = 256 samples =
/// 16 ms — plenty smaller than the DMA ring so push_with isn't starved.
const MONO_SCRATCH_BYTES: usize = 512;

/// Audio task state captured outside the closure so push_with can mutate
/// fanfare position across iterations without borrowing self twice.
struct AudioState {
    fanfare_mono: &'static [u8],
    fanfare_pos: Option<usize>, // Some(n) while playing fanfare from offset n
    /// Last sample written to the DMA buffer (post-gain). On underrun we
    /// hold this value rather than writing zeros — going from +30k to 0
    /// in one sample period is an audible click. Holding the DAC at the
    /// last voltage produces no transient. Decays toward 0 once we're
    /// idle so the speaker eventually goes truly silent.
    last_sample: i16,
    /// Set whenever the previous fill ran out of data and we padded with
    /// the held sample. The next fill that produces fresh samples
    /// cross-fades the first FADE_FRAMES from `last_sample` to the new
    /// data so the held-to-fresh transition doesn't sound like a click.
    held_pending: bool,
}

/// Number of stereo frames to cross-fade when transitioning from a held
/// sample (after an underrun) to fresh audio. 64 frames ≈ 4 ms @ 16 kHz —
/// long enough to mask the step, short enough that brief underruns don't
/// audibly attenuate the wavefront.
const FADE_FRAMES: usize = 64;

#[embassy_executor::task]
async fn audio_task(
    i2s0: esp_hal::peripherals::I2S0<'static>,
    dma: esp_hal::peripherals::DMA_CH0<'static>,
    mclk: esp_hal::peripherals::GPIO18<'static>,
    bclk: esp_hal::peripherals::GPIO17<'static>,
    lrck: esp_hal::peripherals::GPIO15<'static>,
    dout: esp_hal::peripherals::GPIO14<'static>,
) {
    use esp_hal::i2s::master::{Config as I2sConfig, DataFormat, I2s};
    use esp_hal::time::Rate;

    AUDIO_READY.wait().await;

    let cfg = I2sConfig::new_tdm_philips()
        .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
        .with_data_format(DataFormat::Data16Channel16);

    let i2s = match I2s::new(i2s0, dma, cfg) {
        Ok(i) => i.into_async().with_mclk(mclk),
        Err(e) => {
            println!("i2s: config err {:?}", e);
            return;
        }
    };

    // Properly-aligned static buffer + circular descriptor chain.
    let (_rx_buf, _rx_desc, tx_buffer, tx_descriptors) =
        esp_hal::dma_circular_buffers!(0, DMA_RING_BYTES);

    let tx = i2s
        .i2s_tx
        .with_bclk(bclk)
        .with_ws(lrck)
        .with_dout(dout)
        .build(tx_descriptors);

    let mut circ = match tx.write_dma_circular_async(tx_buffer) {
        Ok(c) => c,
        Err(e) => {
            println!("i2s: circular dma err {:?}", e);
            return;
        }
    };
    println!(
        "audio: I²S circular DMA running ({} kHz, ring {} B)",
        SAMPLE_RATE_HZ / 1000,
        DMA_RING_BYTES,
    );

    let fanfare_mono: &'static [u8] = Box::leak(build_fanfare_mono_buffer());
    let mut state = AudioState {
        fanfare_mono,
        fanfare_pos: None,
        last_sample: 0,
        held_pending: false,
    };

    loop {
        let res = circ.push_with(|dma_slice| state.fill_chunk(dma_slice)).await;
        if let Err(e) = res {
            println!("audio: push err {:?}", e);
            return;
        }
    }
}

impl AudioState {
    /// Fill the next `dma_slice` from the circular DMA buffer. Returns the
    /// number of bytes written (always the full slice so BCLK keeps
    /// running).
    ///
    /// Three click-prevention measures combine:
    ///   1. Underrun pads hold `last_sample` (no peak-to-zero step).
    ///   2. When fresh audio arrives after a held pad, the first
    ///      `FADE_FRAMES` are cross-faded from `last_sample` to the new
    ///      samples (no held-to-fresh step).
    ///   3. When nothing is producing, the held value decays toward 0
    ///      exponentially (silent finish without a click at `stop`).
    fn fill_chunk(&mut self, dma_slice: &mut [u8]) -> usize {
        let chunk_len = dma_slice.len();
        let stereo_frames = chunk_len / 4;
        let valid_end = stereo_frames * 4;
        for b in &mut dma_slice[valid_end..] {
            *b = 0;
        }

        if self.fanfare_pos.is_none() && buzzer::take_done_fanfare() {
            self.fanfare_pos = Some(0);
        }

        let gain = spk::gain_q8();
        let mut frames_done = 0;

        if let Some(start) = self.fanfare_pos {
            let remaining_mono = self.fanfare_mono.len() - start;
            let mono_take = remaining_mono.min(stereo_frames * 2) & !1;
            let frames = mono_take / 2;
            // Fanfare lives in a separate Box::leak'd buffer so the borrow
            // is independent of `self`.
            let src = &self.fanfare_mono[start..start + mono_take];
            expand_with_fade(
                src,
                &mut dma_slice[..frames * 4],
                256,
                &mut self.last_sample,
                &mut self.held_pending,
            );
            frames_done += frames;
            let new_pos = start + mono_take;
            self.fanfare_pos = if new_pos >= self.fanfare_mono.len() {
                None
            } else {
                Some(new_pos)
            };
        }

        // Stack-local scratch so we don't have a self-borrow conflict
        // with `&mut self.last_sample` / `held_pending` below. 512 bytes
        // is well within an embassy task's stack budget.
        let mut mono_scratch = [0u8; MONO_SCRATCH_BYTES];
        while frames_done < stereo_frames && spk::is_running() {
            let frames_left = stereo_frames - frames_done;
            let want_mono = (frames_left * 2).min(MONO_SCRATCH_BYTES);
            let n = spk::try_drain(&mut mono_scratch[..want_mono]);
            if n < 2 {
                break;
            }
            let n_even = n & !1;
            let frames = n_even / 2;
            let off = frames_done * 4;
            expand_with_fade(
                &mono_scratch[..n_even],
                &mut dma_slice[off..off + frames * 4],
                gain,
                &mut self.last_sample,
                &mut self.held_pending,
            );
            frames_done += frames;
        }

        if frames_done < stereo_frames {
            if spk::is_running() && self.fanfare_pos.is_none() {
                spk::note_underrun();
            }
            let producing = spk::is_running() || self.fanfare_pos.is_some();
            if !producing {
                // ~50 ms exponential decay per chunk so the speaker reaches
                // true silence within a few chunks after `stop`.
                self.last_sample = (self.last_sample as i32 * 220 / 256) as i16;
            }
            self.held_pending = true;
            let hold = self.last_sample;
            let b = hold.to_le_bytes();
            let start = frames_done * 4;
            let mut p = start;
            while p + 4 <= valid_end {
                dma_slice[p] = b[0];
                dma_slice[p + 1] = b[1];
                dma_slice[p + 2] = b[0];
                dma_slice[p + 3] = b[1];
                p += 4;
            }
        }

        chunk_len
    }
}

/// Write `mono` s16le samples (mono → stereo with `gain_q8`) into
/// `stereo`, cross-fading the first `FADE_FRAMES` from `*last_sample` to
/// the new samples when `*held_pending`. Updates both refs on exit.
fn expand_with_fade(
    mono: &[u8],
    stereo: &mut [u8],
    gain_q8: u16,
    last_sample: &mut i16,
    held_pending: &mut bool,
) {
    let n_samples = mono.len() / 2;
    if n_samples == 0 {
        return;
    }
    let held = *last_sample;
    let fade_n = if *held_pending { FADE_FRAMES.min(n_samples) } else { 0 };
    let mut last = held;
    for i in 0..n_samples {
        let s = i16::from_le_bytes([mono[i * 2], mono[i * 2 + 1]]);
        let scaled = ((s as i32 * gain_q8 as i32) >> 8)
            .clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let out = if i < fade_n {
            // Linear cross-fade. k=0 → fully held, k=fade_n → fully new.
            let k = (i + 1) as i32;
            let m = fade_n as i32;
            let v = held as i32 * (m - k) + scaled as i32 * k;
            (v / m) as i16
        } else {
            scaled
        };
        let b = out.to_le_bytes();
        let off = i * 4;
        stereo[off] = b[0];
        stereo[off + 1] = b[1];
        stereo[off + 2] = b[0];
        stereo[off + 3] = b[1];
        last = out;
    }
    *last_sample = last;
    *held_pending = false;
}

/// Build a mono s16le PCM buffer with the two-beep boot fanfare.
fn build_fanfare_mono_buffer() -> Box<[u8]> {
    let mut buf = alloc::vec![0u8; FANFARE_MONO_BYTES].into_boxed_slice();
    let mut frame = 0usize;

    fn fill_tone(buf: &mut [u8], frame_start: usize, n_frames: usize, freq_hz: u32, amp_q15: i32) {
        let period = (SAMPLE_RATE_HZ + freq_hz / 2) / freq_hz;
        for i in 0..n_frames {
            let phase = (i as u32) % period;
            let half = period / 2;
            let t = phase as i32 - half as i32;
            let tabs = t.unsigned_abs() as i32;
            let para = 4 * t * (half as i32 - tabs) / (half as i32 * half as i32 / 32);
            let sample = ((para as i64 * amp_q15 as i64) >> 7) as i32;
            let s = sample.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let off = (frame_start + i) * 2;
            let bytes = s.to_le_bytes();
            buf[off] = bytes[0];
            buf[off + 1] = bytes[1];
        }
    }

    let a_frames = (SAMPLE_RATE_HZ * FANFARE_TONE_A_MS / 1000) as usize;
    let g_frames = (SAMPLE_RATE_HZ * FANFARE_GAP_MS / 1000) as usize;
    let b_frames = (SAMPLE_RATE_HZ * FANFARE_TONE_B_MS / 1000) as usize;

    fill_tone(&mut buf, frame, a_frames, 880, 12000);
    frame += a_frames;
    frame += g_frames; // gap stays zero
    fill_tone(&mut buf, frame, b_frames, 1175, 12000);

    buf
}
