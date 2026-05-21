//! WiFi credentials in flash (simple blob).

use embedded_storage::ReadStorage;
use embedded_storage::Storage;
use esp_storage::FlashStorage;
use static_cell::StaticCell;

const MAGIC: u32 = 0x5739_4631; // "W9F1"
const NVS_OFFSET: u32 = 0x3d_0000;

static FLASH: StaticCell<FlashStorage<'static>> = StaticCell::new();
static mut FLASH_PTR: *mut FlashStorage<'static> = core::ptr::null_mut();

#[derive(Clone)]
pub struct WifiConfig {
    pub ssid: heapless::String<32>,
    pub password: heapless::String<64>,
}

impl WifiConfig {
    pub fn is_valid(&self) -> bool {
        !self.ssid.is_empty()
    }
}

pub fn init(flash: esp_hal::peripherals::FLASH<'static>) {
    let f = FLASH.init(FlashStorage::new(flash));
    unsafe { FLASH_PTR = f };
}

fn flash() -> &'static mut FlashStorage<'static> {
    unsafe {
        if FLASH_PTR.is_null() {
            panic!("nvs flash not initialized");
        }
        &mut *FLASH_PTR
    }
}

pub fn load() -> Option<WifiConfig> {
    let flash = flash();
    let mut buf = [0u8; 128];
    flash.read(NVS_OFFSET, &mut buf).ok()?;
    let magic = u32::from_le_bytes(buf[0..4].try_into().ok()?);
    if magic != MAGIC {
        return None;
    }
    let ssid_len = buf[4] as usize;
    let pass_len = buf[36] as usize;
    if ssid_len > 32 || pass_len > 64 {
        return None;
    }
    let ssid = core::str::from_utf8(&buf[8..8 + ssid_len]).ok()?;
    let pass = core::str::from_utf8(&buf[40..40 + pass_len]).ok()?;
    let mut cfg = WifiConfig {
        ssid: heapless::String::new(),
        password: heapless::String::new(),
    };
    cfg.ssid.push_str(ssid).ok()?;
    cfg.password.push_str(pass).ok()?;
    Some(cfg)
}

pub fn erase() -> Result<(), ()> {
    let flash = flash();
    let buf = [0u8; 128];
    flash.write(NVS_OFFSET, &buf).map_err(|_| ())
}

pub fn save(cfg: &WifiConfig) -> Result<(), ()> {
    let flash = flash();
    let mut buf = [0u8; 128];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4] = cfg.ssid.len() as u8;
    buf[8..8 + cfg.ssid.len()].copy_from_slice(cfg.ssid.as_bytes());
    buf[36] = cfg.password.len() as u8;
    buf[40..40 + cfg.password.len()].copy_from_slice(cfg.password.as_bytes());
    flash.write(NVS_OFFSET, &buf).map_err(|_| ())
}
