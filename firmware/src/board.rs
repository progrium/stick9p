//! Board pin definitions.

#[cfg(feature = "board-plus2")]
pub mod pins {
    use esp_hal::gpio::{Level, Output, OutputConfig};

    pub fn init_hold(pin: esp_hal::peripherals::GPIO4<'_>) -> Output<'_> {
        let mut hold = Output::new(pin, Level::Low, OutputConfig::default());
        hold.set_high();
        hold
    }

    pub fn init_led(pin: esp_hal::peripherals::GPIO19<'_>) -> Output<'_> {
        Output::new(pin, Level::Low, OutputConfig::default())
    }
}

pub const BOARD_NAME: &str = {
    #[cfg(feature = "board-plus2")]
    {
        "plus2"
    }
    #[cfg(feature = "board-sticks3")]
    {
        "sticks3"
    }
};

pub const FW_VERSION: &str = "stick9p-0.5.2-stage3-mic";

/// SoC model string. Chosen by board feature so we don't need to read it from
/// efuse (the chip variant is fixed per board: Plus2 = ESP32, StickS3 = ESP32-S3).
pub const CHIP_MODEL: &str = {
    #[cfg(feature = "board-plus2")]
    {
        "esp32"
    }
    #[cfg(feature = "board-sticks3")]
    {
        "esp32s3"
    }
};

/// Number of CPU cores on the SoC.
pub const CHIP_CORES: u8 = {
    #[cfg(feature = "board-plus2")]
    {
        2
    }
    #[cfg(feature = "board-sticks3")]
    {
        2
    }
};

/// Producers for `/sys/mac`, `/sys/chip`, `/sys/heap`, `/sys/tmpfs`. Each
/// returns newline-terminated lines sized to fit in a single 9P Tread.
pub mod sys_info {
    use core::fmt::Write;
    use esp_hal::efuse;
    use heapless::String;

    pub fn mac_line() -> String<24> {
        let mut s = String::new();
        let mac = efuse::base_mac_address();
        let _ = write!(&mut s, "{}\n", mac);
        s
    }

    pub fn chip_line() -> String<96> {
        let mut s = String::new();
        let rev = efuse::chip_revision();
        let cpu_mhz = esp_hal::clock::cpu_clock().as_hz() / 1_000_000;
        let _ = write!(
            &mut s,
            "model={} rev={}.{} cores={} cpu_mhz={}\n",
            super::CHIP_MODEL,
            rev.major,
            rev.minor,
            super::CHIP_CORES,
            cpu_mhz,
        );
        s
    }

    pub fn heap_line() -> String<160> {
        // Aggregate every internal region into "sram" and every external region
        // into "psram". `esp_alloc::HEAP` may hold up to 3 regions per pool, so
        // summing them keeps the file scriptable (one line per memory kind)
        // regardless of how `main.rs` decided to carve internal RAM.
        use esp_alloc::MemoryCapability;
        let stats = esp_alloc::HEAP.stats();
        let mut sram = (0usize, 0usize, 0usize); // free, used, total
        let mut psram = (0usize, 0usize, 0usize);
        for region in stats.region_stats.iter().flatten() {
            let tgt = if region.capabilities.contains(MemoryCapability::External) {
                &mut psram
            } else {
                &mut sram
            };
            tgt.0 += region.free;
            tgt.1 += region.used;
            tgt.2 += region.size;
        }

        let mut s = String::new();
        if sram.2 > 0 {
            let _ = write!(
                &mut s,
                "sram free={} used={} total={}\n",
                sram.0, sram.1, sram.2,
            );
        }
        if psram.2 > 0 {
            let _ = write!(
                &mut s,
                "psram free={} used={} total={}\n",
                psram.0, psram.1, psram.2,
            );
        }
        s
    }

    /// `/sys/tmpfs` — arena + inode usage for the `/tmp` ramfs (not `esp_alloc`).
    pub fn tmpfs_line() -> String<96> {
        let mut s = String::new();
        if let Some((free, used, total)) = devices::memfs::arena_stats() {
            let _ = write!(
                &mut s,
                "arena free={} used={} total={}\n",
                free, used, total
            );
        } else {
            let _ = write!(&mut s, "arena unavailable\n");
        }
        let (ino_used, ino_total) = devices::memfs::inode_stats();
        let ino_free = ino_total.saturating_sub(ino_used);
        let _ = write!(
            &mut s,
            "inodes free={} used={} total={}\n",
            ino_free, ino_used, ino_total
        );
        s
    }
}
