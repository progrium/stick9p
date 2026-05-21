#![no_std]
#![no_main]
#![deny(clippy::mem_forget)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use firmware::board;
use firmware::led_task;
use firmware::net;
use firmware::nvs;

esp_bootloader_esp_idf::esp_app_desc!();

#[allow(clippy::large_stack_frames)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 40 * 1024);
    esp_alloc::heap_allocator!(size: 24 * 1024);

    println!("stick9p Stage 3 — board {}", board::BOARD_NAME);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let flash = unsafe { core::mem::transmute(peripherals.FLASH) };
    let wifi = unsafe { core::mem::transmute(peripherals.WIFI) };

    #[cfg(feature = "board-plus2")]
    {
        esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);
        let _hold = board::pins::init_hold(peripherals.GPIO4);
        let led: esp_hal::gpio::Output<'static> =
            unsafe { core::mem::transmute(board::pins::init_led(peripherals.GPIO19)) };
        spawner.spawn(led_task::run(led).unwrap());
        firmware::dev::spawn(
            &spawner,
            peripherals.I2C0,
            peripherals.GPIO21,
            peripherals.GPIO22,
            peripherals.GPIO37,
            peripherals.GPIO39,
            peripherals.ADC1,
            peripherals.GPIO38,
            peripherals.GPIO2,
            peripherals.SPI2,
            peripherals.GPIO15,
            peripherals.GPIO13,
            peripherals.GPIO14,
            peripherals.GPIO5,
            peripherals.GPIO12,
            peripherals.GPIO27,
            peripherals.LEDC,
            peripherals.I2S0,
            peripherals.DMA_I2S0,
            peripherals.GPIO0,
            peripherals.GPIO34,
        );
    }

    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    nvs::init(flash);

    if let Some(cfg) = nvs::load() {
        if cfg.is_valid() {
            println!("nvs: using stored ssid {}", cfg.ssid.as_str());
            net::sta::run(&spawner, wifi, cfg).await;
        }
    }

    println!("nvs: no WiFi config — starting provisioning AP");
    net::provision::run(&spawner, wifi).await;
}
