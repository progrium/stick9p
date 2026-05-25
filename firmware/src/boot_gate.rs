//! StickS3 boot sequencing: stagger high-current bring-up and one fanfare.

#[cfg(feature = "board-sticks3")]
mod imp {
    use core::sync::atomic::{AtomicU8, Ordering};

    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embassy_sync::signal::Signal;
    use esp_println::println;

    /// PMIC identified + BMI270 programmed (no L3B / codec / amp yet).
    static DEVICES_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
    /// DHCP up — safe to enable L3B, display, and codec (still no amp / I²S).
    static NETWORK_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
    /// AW8737 amp enabled — audio_task may start I²S TX for fanfare.
    static AMP_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();
    /// All subsystems up — fanfare is the only intentional sound.
    static BOOT_COMPLETE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

    const BIT_CODEC: u8 = 1;
    const BIT_DISPLAY: u8 = 2;
    const BIT_NET9P: u8 = 4;
    const ALL_BITS: u8 = BIT_CODEC | BIT_DISPLAY | BIT_NET9P;

    static SUBSYSTEMS: AtomicU8 = AtomicU8::new(0);
    /// Latched when codec+display+9p are ready. Not consumed by `wait_boot_complete`.
    static BOOT_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

    pub fn signal_devices_ready() {
        DEVICES_READY.signal(());
    }

    pub async fn wait_devices_ready() {
        DEVICES_READY.wait().await;
    }

    pub fn signal_network_ready() {
        NETWORK_READY.signal(());
    }

    pub async fn wait_network_ready() {
        NETWORK_READY.wait().await;
    }

    pub fn signal_amp_ready() {
        AMP_READY.signal(());
    }

    pub async fn wait_amp_ready() {
        AMP_READY.wait().await;
    }

    pub async fn wait_boot_complete() {
        BOOT_COMPLETE.wait().await;
    }

    pub fn boot_is_done() -> bool {
        BOOT_DONE.load(Ordering::Acquire)
    }

    /// Call when codec, display, or 9P listener reaches its ready point.
    pub fn mark_subsystem_ready(bit: u8) {
        let prev = SUBSYSTEMS.fetch_or(bit, Ordering::SeqCst);
        let now = prev | bit;
        if now == ALL_BITS && prev != ALL_BITS {
            BOOT_DONE.store(true, Ordering::Release);
            println!("boot: ready (codec+display+9p)");
            BOOT_COMPLETE.signal(());
        }
    }

    pub const SUBSYS_CODEC: u8 = BIT_CODEC;
    pub const SUBSYS_DISPLAY: u8 = BIT_DISPLAY;
    pub const SUBSYS_NET9P: u8 = BIT_NET9P;
}

#[cfg(not(feature = "board-sticks3"))]
mod imp {
    pub fn signal_devices_ready() {}
    pub async fn wait_devices_ready() {}
    pub fn signal_network_ready() {}
    pub async fn wait_network_ready() {}
    pub fn signal_amp_ready() {}
    pub async fn wait_amp_ready() {}
    pub async fn wait_boot_complete() {}
    pub fn boot_is_done() -> bool {
        true
    }
    pub fn mark_subsystem_ready(_bit: u8) {}
    pub const SUBSYS_CODEC: u8 = 0;
    pub const SUBSYS_DISPLAY: u8 = 0;
    pub const SUBSYS_NET9P: u8 = 0;
}

pub use imp::*;
