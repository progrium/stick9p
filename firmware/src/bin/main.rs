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

#[cfg(feature = "board-sticks3")]
use esp_hal::rtc_cntl::SocResetReason;
use firmware::board;
#[cfg(feature = "board-plus2")]
use firmware::led_task;
use firmware::net;
use firmware::nvs;

esp_bootloader_esp_idf::esp_app_desc!();

#[allow(clippy::large_stack_frames)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    #[cfg(feature = "board-sticks3")]
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::_160MHz);
    #[cfg(not(feature = "board-sticks3"))]
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 40 * 1024);
    // Internal-SRAM heap. esp-alloc caps the total region count at 3, so we
    // budget one slot for "reclaimed", one for plain internal SRAM, and one
    // for PSRAM (added below on the StickS3 path / via psram_allocator! on
    // Plus2). StickS3 uses a larger internal slot than Plus2 because anything
    // PSRAM can't host — structs containing `Atomic*` types (broken in PSRAM
    // on ESP32/S2/S3), DMA-capable buffers, code paths the cache hasn't
    // warmed up — has to live here.
    #[cfg(feature = "board-plus2")]
    esp_alloc::heap_allocator!(size: 24 * 1024);
    #[cfg(feature = "board-sticks3")]
    esp_alloc::heap_allocator!(size: 120 * 1024);

    // Allocate the spk PCM ring on the heap (too large for BSS without
    // running into the embassy task stack guard region on StickS3).
    devices::spk::init();

    println!();
    println!("=====================================================");
    println!(" stick9p Stage 3 — board {}", board::BOARD_NAME);
    println!(" build: {} {}", env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_NAME"));
    println!("=====================================================");

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let flash = unsafe { core::mem::transmute(peripherals.FLASH) };
    let wifi = unsafe { core::mem::transmute(peripherals.WIFI) };

    #[cfg(feature = "board-plus2")]
    {
        const MEMFS_ARENA: usize = 1024 * 1024;
        let psram = esp_hal::psram::Psram::new(peripherals.PSRAM, esp_hal::psram::PsramConfig::default());
        let (psram_ptr, psram_size) = psram.raw_parts();
        if psram_size > 0 {
            let arena_len = MEMFS_ARENA.min(psram_size);
            devices::memfs::init(psram_ptr, arena_len);
            println!("tmp: arena {} KiB", arena_len / 1024);
            if psram_size > arena_len {
                unsafe {
                    esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
                        psram_ptr.add(arena_len),
                        psram_size - arena_len,
                        esp_alloc::MemoryCapability::External.into(),
                    ));
                }
            }
            println!("psram: ready ({} KiB heap)", (psram_size - arena_len) / 1024);
        } else {
            println!("psram: init failed — /tmp unavailable");
        }
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
            peripherals.I2C1,
            peripherals.GPIO32,
            peripherals.GPIO33,
        );
    }

    #[cfg(feature = "board-sticks3")]
    {
        if let Some(reason) = esp_hal::system::reset_reason() {
            println!("boot: reset reason {:?}", reason);
            if reason == SocResetReason::ChipPowerOn {
                // ESP-IDF also uses 0x01 for brownout; treat repeat boots as suspect.
            }
        }

        let psram_periph = peripherals.PSRAM;
        firmware::dev::spawn(
            &spawner,
            peripherals.I2C0,
            peripherals.GPIO47,
            peripherals.GPIO48,
            peripherals.GPIO11,
            peripherals.GPIO12,
            peripherals.SPI2,
            peripherals.GPIO39,
            peripherals.GPIO40,
            peripherals.GPIO45,
            peripherals.GPIO41,
            peripherals.GPIO21,
            peripherals.GPIO38,
            peripherals.I2S0,
            peripherals.DMA_CH0,
            peripherals.GPIO18,
            peripherals.GPIO17,
            peripherals.GPIO15,
            peripherals.GPIO14,
            peripherals.GPIO16,
            peripherals.I2C1,
            peripherals.GPIO9,
            peripherals.GPIO10,
            peripherals.GPIO1,
            peripherals.GPIO2,
            peripherals.GPIO3,
            peripherals.GPIO4,
            peripherals.GPIO5,
            peripherals.GPIO6,
            peripherals.GPIO7,
            peripherals.GPIO8,
        );

        esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
        nvs::init(flash);

        // PMIC + IMU first; OPI PSRAM after (avoids PSRAM + 8 KB IMU + WiFi together).
        firmware::boot_gate::wait_devices_ready().await;

        const MEMFS_ARENA: usize = 2 * 1024 * 1024;
        let provision_boot = match nvs::load() {
            None => true,
            Some(c) => !c.is_valid(),
        };
        let psram = esp_hal::psram::Psram::new(psram_periph, esp_hal::psram::PsramConfig::default());
        let (psram_ptr, psram_size) = psram.raw_parts();
        if psram_size > 0 {
            if provision_boot {
                // Captive portal does not need /tmp; keep all PSRAM for heap/WiFi.
                unsafe {
                    esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
                        psram_ptr,
                        psram_size,
                        esp_alloc::MemoryCapability::External.into(),
                    ));
                }
                println!(
                    "psram: ready ({} KiB heap, /tmp deferred until STA)",
                    psram_size / 1024
                );
            } else {
                let arena_len = MEMFS_ARENA.min(psram_size);
                devices::memfs::init(psram_ptr, arena_len);
                println!("tmp: arena {} KiB", arena_len / 1024);
                if psram_size > arena_len {
                    unsafe {
                        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
                            psram_ptr.add(arena_len),
                            psram_size - arena_len,
                            esp_alloc::MemoryCapability::External.into(),
                        ));
                    }
                }
                println!(
                    "psram: ready ({} KiB heap)",
                    psram_size.saturating_sub(arena_len) / 1024
                );
            }
        } else {
            println!("psram: init failed — running on internal SRAM only");
        }
        embassy_time::Timer::after(embassy_time::Duration::from_millis(200)).await;
    }

    #[cfg(not(feature = "board-sticks3"))]
    {
        esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
        nvs::init(flash);
    }

    #[cfg(not(feature = "board-sticks3"))]
    firmware::boot_gate::wait_devices_ready().await;

    if let Some(cfg) = nvs::load() {
        if cfg.is_valid() {
            println!("nvs: using stored ssid {}", cfg.ssid.as_str());
            net::sta::run(&spawner, wifi, cfg).await;
        }
    }

    println!("nvs: no WiFi config — starting provisioning AP");
    net::provision::run(&spawner, wifi).await;
}
