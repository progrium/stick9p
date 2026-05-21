//! M5StickC Plus2 Stage 2 hardware bring-up.

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_println::println;

extern crate alloc;

use alloc::boxed::Box;

use devices::buzzer;
use devices::display;

pub fn spawn(
    spawner: &Spawner,
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO21<'static>,
    scl: esp_hal::peripherals::GPIO22<'static>,
    btn_a: esp_hal::peripherals::GPIO37<'static>,
    btn_b: esp_hal::peripherals::GPIO39<'static>,
    adc1: esp_hal::peripherals::ADC1<'static>,
    batt_adc: esp_hal::peripherals::GPIO38<'static>,
    buzzer_pin: esp_hal::peripherals::GPIO2<'static>,
    spi2: esp_hal::peripherals::SPI2<'static>,
    lcd_mosi: esp_hal::peripherals::GPIO15<'static>,
    lcd_sck: esp_hal::peripherals::GPIO13<'static>,
    lcd_dc: esp_hal::peripherals::GPIO14<'static>,
    lcd_cs: esp_hal::peripherals::GPIO5<'static>,
    lcd_rst: esp_hal::peripherals::GPIO12<'static>,
    lcd_bl: esp_hal::peripherals::GPIO27<'static>,
    ledc: esp_hal::peripherals::LEDC<'static>,
    i2s0: esp_hal::peripherals::I2S0<'static>,
    dma_i2s0: esp_hal::peripherals::DMA_I2S0<'static>,
    mic_clk: esp_hal::peripherals::GPIO0<'static>,
    mic_data: esp_hal::peripherals::GPIO34<'static>,
) {
    let fb = Box::new([0u8; display::FB_LEN]);
    let fb: &'static mut [u8; display::FB_LEN] = Box::leak(fb);
    devices::display::init(fb);

    spawner.spawn(imu_task(i2c0, sda, scl).unwrap());
    spawner.spawn(buttons_task(btn_a, btn_b).unwrap());
    spawner.spawn(power_task(adc1, batt_adc).unwrap());
    spawner.spawn(buzzer_task(buzzer_pin).unwrap());
    spawner.spawn(
        display_task(spi2, lcd_mosi, lcd_sck, lcd_dc, lcd_cs, lcd_rst, lcd_bl, ledc).unwrap(),
    );

    super::mic::spawn(
        spawner,
        i2s0,
        dma_i2s0,
        mic_clk,
        mic_data,
    );

    buzzer::request_stage2_done();
    println!("stage3: devices ready (mic PDM)");
}

#[embassy_executor::task]
async fn imu_task(
    i2c0: esp_hal::peripherals::I2C0<'static>,
    sda: esp_hal::peripherals::GPIO21<'static>,
    scl: esp_hal::peripherals::GPIO22<'static>,
) {
    let i2c = I2c::new(i2c0, I2cConfig::default())
        .unwrap()
        .with_sda(sda)
        .with_scl(scl);

    let mut imu = ImuBus { i2c };
    if imu.init().is_err() {
        println!("imu: init failed");
        loop {
            Timer::after(Duration::from_secs(5)).await;
        }
    }
    println!("imu: MPU6886 ok");

    loop {
        let hz = devices::imu::rate_hz().max(1);
        let period = Duration::from_millis(1000 / hz as u64);
        if let Ok((ax, ay, az, gx, gy, gz)) = imu.read_sample() {
            devices::imu::push_accel(ax, ay, az);
            devices::imu::push_gyro(gx, gy, gz);
        }
        Timer::after(period).await;
    }
}

struct ImuBus<I2C> {
    i2c: I2C,
}

impl<I2C> ImuBus<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    const ADDR: u8 = 0x68;

    fn init(&mut self) -> Result<(), ()> {
        self.write_reg(0x6B, &[0x00])?;
        self.write_reg(0x1C, &[0x00])?;
        self.write_reg(0x1B, &[0x00])?;
        let who = self.read_reg(0x75)?;
        if who != 0x19 {
            println!("imu: WHO_AM_I={:#x}", who);
        }
        Ok(())
    }

    fn read_sample(&mut self) -> Result<(i32, i32, i32, i32, i32, i32), ()> {
        let mut buf = [0u8; 14];
        self.read_regs(0x3B, &mut buf)?;
        let ax = i16::from_be_bytes([buf[0], buf[1]]) as i32 * 1000 / 16384;
        let ay = i16::from_be_bytes([buf[2], buf[3]]) as i32 * 1000 / 16384;
        let az = i16::from_be_bytes([buf[4], buf[5]]) as i32 * 1000 / 16384;
        let gx = i16::from_be_bytes([buf[8], buf[9]]) as i32;
        let gy = i16::from_be_bytes([buf[10], buf[11]]) as i32;
        let gz = i16::from_be_bytes([buf[12], buf[13]]) as i32;
        Ok((ax, ay, az, gx, gy, gz))
    }

    fn write_reg(&mut self, reg: u8, val: &[u8]) -> Result<(), ()> {
        let mut buf = [0u8; 8];
        buf[0] = reg;
        buf[1..1 + val.len()].copy_from_slice(val);
        self.i2c.write(Self::ADDR, &buf[..1 + val.len()]).map_err(|_| ())
    }

    fn read_reg(&mut self, reg: u8) -> Result<u8, ()> {
        let mut b = [0u8];
        self.read_regs(reg, &mut b)?;
        Ok(b[0])
    }

    fn read_regs(&mut self, reg: u8, out: &mut [u8]) -> Result<(), ()> {
        self.i2c.write_read(Self::ADDR, &[reg], out).map_err(|_| ())
    }
}

#[embassy_executor::task]
async fn buttons_task(
    btn_a: esp_hal::peripherals::GPIO37<'static>,
    btn_b: esp_hal::peripherals::GPIO39<'static>,
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

#[embassy_executor::task]
async fn power_task(
    adc1: esp_hal::peripherals::ADC1<'static>,
    adc_pin: esp_hal::peripherals::GPIO38<'static>,
) {
    use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};

    let mut cfg = AdcConfig::new();
    let mut pin = cfg.enable_pin(adc_pin, Attenuation::_11dB);
    let mut adc = Adc::new(adc1, cfg);
    loop {
        let raw = nb::block!(adc.read_oneshot(&mut pin)).unwrap_or(0);
        let mv = (raw as u32) * 3300 * 2 / 4095;
        devices::power::set_vbat_mv(mv);
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn buzzer_task(pin: esp_hal::peripherals::GPIO2<'static>) {
    let mut out = Output::new(pin, Level::Low, OutputConfig::default());

    loop {
        if buzzer::take_done_fanfare() {
            play_beep(&mut out, 880, 120).await;
            Timer::after(Duration::from_millis(60)).await;
            play_beep(&mut out, 1175, 180).await;
        }
        if let Some(req) = buzzer::take_beep() {
            play_beep(&mut out, req.freq_hz, req.ms).await;
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

async fn play_beep(out: &mut Output<'_>, freq_hz: u32, ms: u32) {
    if freq_hz == 0 || ms == 0 {
        return;
    }
    let half_us = (500_000 / freq_hz).max(1);
    let cycles = (freq_hz as u64 * ms as u64 / 1000).max(1);
    for _ in 0..cycles {
        out.set_high();
        Timer::after(Duration::from_micros(half_us as u64)).await;
        out.set_low();
        Timer::after(Duration::from_micros(half_us as u64)).await;
    }
}

struct EmbDelay;

impl embedded_hal::delay::DelayNs for EmbDelay {
    fn delay_ns(&mut self, ns: u32) {
        embassy_time::block_for(Duration::from_nanos(ns as u64));
    }
}

#[embassy_executor::task]
async fn display_task(
    spi2: esp_hal::peripherals::SPI2<'static>,
    mosi: esp_hal::peripherals::GPIO15<'static>,
    sck: esp_hal::peripherals::GPIO13<'static>,
    dc: esp_hal::peripherals::GPIO14<'static>,
    cs: esp_hal::peripherals::GPIO5<'static>,
    rst: esp_hal::peripherals::GPIO12<'static>,
    bl: esp_hal::peripherals::GPIO27<'static>,
    ledc_periph: esp_hal::peripherals::LEDC<'static>,
) {
    use display_interface_spi::SPIInterface;
    use esp_hal::gpio::DriveMode;
    use esp_hal::ledc::channel::{self, config as ch_cfg, ChannelIFace};
    use esp_hal::ledc::timer::{self, config as tmr_cfg, TimerIFace};
    use esp_hal::ledc::{Ledc, LSGlobalClkSource, LowSpeed};
    use embedded_graphics_core::pixelcolor::{Rgb565, raw::RawU16};
    use embedded_hal_bus::spi::ExclusiveDevice;
    use esp_hal::spi::master::{Config as SpiConfig, Spi};
    use esp_hal::spi::Mode;
    use esp_hal::time::Rate;
    use mipidsi::{
        Builder,
        models::ST7789,
        options::{ColorInversion, ColorOrder, Orientation, Rotation},
    };

    let cs_pin = Output::new(cs, Level::High, OutputConfig::default());
    let dc_pin = Output::new(dc, Level::Low, OutputConfig::default());
    let rst_pin = Output::new(rst, Level::High, OutputConfig::default());

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

    let ledc = Box::leak(Box::new(Ledc::new(ledc_periph)));
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    let bl_timer = Box::leak(Box::new({
        let mut t = ledc.timer::<LowSpeed>(timer::Number::Timer1);
        t.configure(tmr_cfg::Config {
            duty: tmr_cfg::Duty::Duty10Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(40),
        })
        .expect("bl timer");
        t
    }));
    let bl_pwm = Box::leak(Box::new({
        let mut ch = ledc.channel(channel::Number::Channel1, bl);
        ch.configure(ch_cfg::Config {
            timer: bl_timer,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("bl pwm");
        ch
    }));

    println!("display: ST7789 ok (CS=G5 RST=G12, BL PWM G27)");

    // Prove SPI path before 9P: solid red frame
    {
        let w = display::WIDTH as u16;
        let h = display::HEIGHT as u16;
        let red = Rgb565::new(31, 0, 0);
        let pixels = core::iter::repeat(red).take(display::FB_LEN / 2);
        match display.set_pixels(0, 0, w - 1, h - 1, pixels) {
            Ok(()) => println!("display: self-test red ok"),
            Err(e) => println!("display: self-test err {:?}", e),
        }
    }

    loop {
        apply_backlight(bl_pwm, devices::display::is_on(), devices::display::brightness());

        let on = devices::display::is_on();
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

fn apply_backlight(
    ch: &esp_hal::ledc::channel::Channel<'static, esp_hal::ledc::LowSpeed>,
    on: bool,
    level: u8,
) {
    use esp_hal::ledc::channel::ChannelIFace;
    let pct = if on && level > 0 {
        brightness_to_duty_pct(level)
    } else {
        0
    };
    let _ = ch.set_duty(pct);
}

/// Map 1..255 → LEDC duty for this panel's narrow visible range (~84–100%).
/// Below ~84% duty the backlight is effectively off; gamma/linear 0–100% only
/// moved the on/off threshold, it did not add usable steps.
fn brightness_to_duty_pct(level: u8) -> u8 {
    if level == 0 {
        return 0;
    }
    // 10-bit timer: spread 255 steps across raw duty 860..1023 (~84%..100%).
    const MIN_RAW: u32 = 860;
    const MAX_RAW: u32 = 1023;
    let raw = MIN_RAW + (level as u32 - 1) * (MAX_RAW - MIN_RAW) / 254;
    ((raw * 100 + 511) / 1024) as u8
}
