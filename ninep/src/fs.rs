//! Static 9P filesystem (Stage 1 + Stage 2).

use crate::vfs::{Qid, QT_DIR, QT_FILE};
use crate::wire::QidWire;
use heapless::String;

/// Regular file, owner read+write (Linux v9fs expects S_IFREG).
const MODE_FILE_WR: u32 = 0o100_664;
const MODE_FILE_RD: u32 = 0o100_444;

pub const PATH_ROOT: u64 = 1;
pub const PATH_README: u64 = 2;
pub const PATH_CTL: u64 = 3;
pub const PATH_SYS: u64 = 4;
pub const PATH_SYS_BOARD: u64 = 5;
pub const PATH_SYS_VERSION: u64 = 6;
pub const PATH_SYS_UPTIME: u64 = 7;
pub const PATH_SYS_REBOOT: u64 = 8;
pub const PATH_DEV: u64 = 9;
pub const PATH_DEV_LED: u64 = 10;
pub const PATH_DEV_LED_CTL: u64 = 11;
pub const PATH_DEV_LED_STATE: u64 = 12;
pub const PATH_DEV_DISPLAY: u64 = 13;
pub const PATH_DEV_DISPLAY_CTL: u64 = 14;
pub const PATH_DEV_DISPLAY_BRIGHTNESS: u64 = 15;
pub const PATH_DEV_DISPLAY_FB: u64 = 16;
pub const PATH_DEV_DISPLAY_INFO: u64 = 17;
pub const PATH_DEV_IMU: u64 = 18;
pub const PATH_DEV_IMU_CTL: u64 = 19;
pub const PATH_DEV_IMU_ACCEL: u64 = 20;
pub const PATH_DEV_IMU_GYRO: u64 = 21;
pub const PATH_DEV_BUTTONS: u64 = 22;
pub const PATH_DEV_BTN_A: u64 = 23;
pub const PATH_DEV_BTN_B: u64 = 24;
pub const PATH_DEV_BTN_EVENT: u64 = 25;
pub const PATH_DEV_BUTTONS_CTL: u64 = 26;
pub const PATH_DEV_POWER: u64 = 27;
pub const PATH_DEV_POWER_CTL: u64 = 28;
pub const PATH_DEV_POWER_BATTERY: u64 = 29;
pub const PATH_DEV_POWER_VBAT: u64 = 30;
pub const PATH_DEV_BUZZER: u64 = 31;
pub const PATH_DEV_BUZZER_CTL: u64 = 32;
pub const PATH_DEV_MIC: u64 = 33;
pub const PATH_DEV_MIC_CTL: u64 = 34;
pub const PATH_DEV_MIC_PCM: u64 = 35;
pub const PATH_DEV_SPK: u64 = 36;
pub const PATH_DEV_SPK_CTL: u64 = 37;
pub const PATH_DEV_SPK_PCM: u64 = 38;
pub const PATH_DEV_SPK_INFO: u64 = 39;
pub const PATH_DEV_I2C: u64 = 40;
pub const PATH_DEV_I2C_1: u64 = 41;
pub const PATH_DEV_I2C_1_CTL: u64 = 42;
pub const PATH_DEV_I2C_1_SCAN: u64 = 43;
pub const PATH_DEV_I2C_1_DATA: u64 = 44;
pub const PATH_DEV_GPIO: u64 = 50;
/// Base path for `/dev/gpio/<N>` directories. The pin number (1..=8) is
/// added to derive the actual path id.
pub const PATH_DEV_GPIO_PIN_BASE: u64 = 60;
pub const PATH_DEV_GPIO_PIN_CTL_BASE: u64 = 70;
pub const PATH_DEV_GPIO_PIN_LEVEL_BASE: u64 = 80;

/// Static names for `/dev/gpio/<N>` directory entries. Indexed by pin
/// number (1..=8 from the StickS3 Hat2 header), with `?` for the
/// always-invalid 0 slot.
const GPIO_PIN_NAMES: &[&str] = &["?", "1", "2", "3", "4", "5", "6", "7", "8"];

fn gpio_pin_name(n: u8) -> &'static str {
    GPIO_PIN_NAMES
        .get(n as usize)
        .copied()
        .unwrap_or("?")
}

const GPIO_PIN1_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(1), Node::DevGpioPinLevel(1)];
const GPIO_PIN2_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(2), Node::DevGpioPinLevel(2)];
const GPIO_PIN3_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(3), Node::DevGpioPinLevel(3)];
const GPIO_PIN4_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(4), Node::DevGpioPinLevel(4)];
const GPIO_PIN5_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(5), Node::DevGpioPinLevel(5)];
const GPIO_PIN6_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(6), Node::DevGpioPinLevel(6)];
const GPIO_PIN7_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(7), Node::DevGpioPinLevel(7)];
const GPIO_PIN8_CHILDREN: &[Node] = &[Node::DevGpioPinCtl(8), Node::DevGpioPinLevel(8)];

/// All pin dirs under `/dev/gpio` — one entry per `devices::gpio::CLAIMABLE_PINS`.
/// Pin presence is then per-board, decided at runtime: reads of an
/// unregistered pin return `absent\n` without crashing the listing.
const GPIO_DIR_CHILDREN: &[Node] = &[
    Node::DevGpioPin(1),
    Node::DevGpioPin(2),
    Node::DevGpioPin(3),
    Node::DevGpioPin(4),
    Node::DevGpioPin(5),
    Node::DevGpioPin(6),
    Node::DevGpioPin(7),
    Node::DevGpioPin(8),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node {
    Root,
    Readme,
    Ctl,
    Sys,
    SysBoard,
    SysVersion,
    SysUptime,
    SysReboot,
    Dev,
    DevLed,
    DevLedCtl,
    DevLedState,
    DevDisplay,
    DevDisplayCtl,
    DevDisplayBrightness,
    DevDisplayFb,
    DevDisplayInfo,
    DevImu,
    DevImuCtl,
    DevImuAccel,
    DevImuGyro,
    DevButtons,
    DevButtonsCtl,
    DevBtnA,
    DevBtnB,
    DevBtnEvent,
    DevPower,
    DevPowerCtl,
    DevPowerBattery,
    DevPowerVbat,
    DevBuzzer,
    DevBuzzerCtl,
    DevMic,
    DevMicCtl,
    DevMicPcm,
    DevSpk,
    DevSpkCtl,
    DevSpkPcm,
    DevSpkInfo,
    DevI2c,
    DevI2c1,
    DevI2c1Ctl,
    DevI2c1Scan,
    DevI2c1Data,
    DevGpio,
    /// `/dev/gpio/<N>` directory. Inner u8 is the GPIO pin number.
    DevGpioPin(u8),
    DevGpioPinCtl(u8),
    DevGpioPinLevel(u8),
}

impl Node {
    pub fn from_path(path: u64) -> Option<Self> {
        Some(match path {
            PATH_ROOT => Node::Root,
            PATH_README => Node::Readme,
            PATH_CTL => Node::Ctl,
            PATH_SYS => Node::Sys,
            PATH_SYS_BOARD => Node::SysBoard,
            PATH_SYS_VERSION => Node::SysVersion,
            PATH_SYS_UPTIME => Node::SysUptime,
            PATH_SYS_REBOOT => Node::SysReboot,
            PATH_DEV => Node::Dev,
            PATH_DEV_LED => Node::DevLed,
            PATH_DEV_LED_CTL => Node::DevLedCtl,
            PATH_DEV_LED_STATE => Node::DevLedState,
            PATH_DEV_DISPLAY => Node::DevDisplay,
            PATH_DEV_DISPLAY_CTL => Node::DevDisplayCtl,
            PATH_DEV_DISPLAY_BRIGHTNESS => Node::DevDisplayBrightness,
            PATH_DEV_DISPLAY_FB => Node::DevDisplayFb,
            PATH_DEV_DISPLAY_INFO => Node::DevDisplayInfo,
            PATH_DEV_IMU => Node::DevImu,
            PATH_DEV_IMU_CTL => Node::DevImuCtl,
            PATH_DEV_IMU_ACCEL => Node::DevImuAccel,
            PATH_DEV_IMU_GYRO => Node::DevImuGyro,
            PATH_DEV_BUTTONS => Node::DevButtons,
            PATH_DEV_BUTTONS_CTL => Node::DevButtonsCtl,
            PATH_DEV_BTN_A => Node::DevBtnA,
            PATH_DEV_BTN_B => Node::DevBtnB,
            PATH_DEV_BTN_EVENT => Node::DevBtnEvent,
            PATH_DEV_POWER => Node::DevPower,
            PATH_DEV_POWER_CTL => Node::DevPowerCtl,
            PATH_DEV_POWER_BATTERY => Node::DevPowerBattery,
            PATH_DEV_POWER_VBAT => Node::DevPowerVbat,
            PATH_DEV_BUZZER => Node::DevBuzzer,
            PATH_DEV_BUZZER_CTL => Node::DevBuzzerCtl,
            PATH_DEV_MIC => Node::DevMic,
            PATH_DEV_MIC_CTL => Node::DevMicCtl,
            PATH_DEV_MIC_PCM => Node::DevMicPcm,
            PATH_DEV_SPK => Node::DevSpk,
            PATH_DEV_SPK_CTL => Node::DevSpkCtl,
            PATH_DEV_SPK_PCM => Node::DevSpkPcm,
            PATH_DEV_SPK_INFO => Node::DevSpkInfo,
            PATH_DEV_I2C => Node::DevI2c,
            PATH_DEV_I2C_1 => Node::DevI2c1,
            PATH_DEV_I2C_1_CTL => Node::DevI2c1Ctl,
            PATH_DEV_I2C_1_SCAN => Node::DevI2c1Scan,
            PATH_DEV_I2C_1_DATA => Node::DevI2c1Data,
            PATH_DEV_GPIO => Node::DevGpio,
            p if (PATH_DEV_GPIO_PIN_BASE..PATH_DEV_GPIO_PIN_CTL_BASE).contains(&p) => {
                Node::DevGpioPin((p - PATH_DEV_GPIO_PIN_BASE) as u8 + 1)
            }
            p if (PATH_DEV_GPIO_PIN_CTL_BASE..PATH_DEV_GPIO_PIN_LEVEL_BASE).contains(&p) => {
                Node::DevGpioPinCtl((p - PATH_DEV_GPIO_PIN_CTL_BASE) as u8 + 1)
            }
            p if (PATH_DEV_GPIO_PIN_LEVEL_BASE..PATH_DEV_GPIO_PIN_LEVEL_BASE + 16).contains(&p) => {
                Node::DevGpioPinLevel((p - PATH_DEV_GPIO_PIN_LEVEL_BASE) as u8 + 1)
            }
            _ => return None,
        })
    }

    pub fn qid(self) -> Qid {
        let (typ, path) = match self {
            Node::Root
            | Node::Sys
            | Node::Dev
            | Node::DevLed
            | Node::DevDisplay
            | Node::DevImu
            | Node::DevButtons
            | Node::DevPower
            | Node::DevBuzzer
            | Node::DevMic
            | Node::DevSpk
            | Node::DevI2c
            | Node::DevI2c1
            | Node::DevGpio => (QT_DIR, self.path()),
            Node::DevGpioPin(_) => (QT_DIR, self.path()),
            _ => (QT_FILE, self.path()),
        };
        Qid {
            typ,
            vers: 0,
            path,
        }
    }

    pub fn path(self) -> u64 {
        match self {
            Node::Root => PATH_ROOT,
            Node::Readme => PATH_README,
            Node::Ctl => PATH_CTL,
            Node::Sys => PATH_SYS,
            Node::SysBoard => PATH_SYS_BOARD,
            Node::SysVersion => PATH_SYS_VERSION,
            Node::SysUptime => PATH_SYS_UPTIME,
            Node::SysReboot => PATH_SYS_REBOOT,
            Node::Dev => PATH_DEV,
            Node::DevLed => PATH_DEV_LED,
            Node::DevLedCtl => PATH_DEV_LED_CTL,
            Node::DevLedState => PATH_DEV_LED_STATE,
            Node::DevDisplay => PATH_DEV_DISPLAY,
            Node::DevDisplayCtl => PATH_DEV_DISPLAY_CTL,
            Node::DevDisplayBrightness => PATH_DEV_DISPLAY_BRIGHTNESS,
            Node::DevDisplayFb => PATH_DEV_DISPLAY_FB,
            Node::DevDisplayInfo => PATH_DEV_DISPLAY_INFO,
            Node::DevImu => PATH_DEV_IMU,
            Node::DevImuCtl => PATH_DEV_IMU_CTL,
            Node::DevImuAccel => PATH_DEV_IMU_ACCEL,
            Node::DevImuGyro => PATH_DEV_IMU_GYRO,
            Node::DevButtons => PATH_DEV_BUTTONS,
            Node::DevButtonsCtl => PATH_DEV_BUTTONS_CTL,
            Node::DevBtnA => PATH_DEV_BTN_A,
            Node::DevBtnB => PATH_DEV_BTN_B,
            Node::DevBtnEvent => PATH_DEV_BTN_EVENT,
            Node::DevPower => PATH_DEV_POWER,
            Node::DevPowerCtl => PATH_DEV_POWER_CTL,
            Node::DevPowerBattery => PATH_DEV_POWER_BATTERY,
            Node::DevPowerVbat => PATH_DEV_POWER_VBAT,
            Node::DevBuzzer => PATH_DEV_BUZZER,
            Node::DevBuzzerCtl => PATH_DEV_BUZZER_CTL,
            Node::DevMic => PATH_DEV_MIC,
            Node::DevMicCtl => PATH_DEV_MIC_CTL,
            Node::DevMicPcm => PATH_DEV_MIC_PCM,
            Node::DevSpk => PATH_DEV_SPK,
            Node::DevSpkCtl => PATH_DEV_SPK_CTL,
            Node::DevSpkPcm => PATH_DEV_SPK_PCM,
            Node::DevSpkInfo => PATH_DEV_SPK_INFO,
            Node::DevI2c => PATH_DEV_I2C,
            Node::DevI2c1 => PATH_DEV_I2C_1,
            Node::DevI2c1Ctl => PATH_DEV_I2C_1_CTL,
            Node::DevI2c1Scan => PATH_DEV_I2C_1_SCAN,
            Node::DevI2c1Data => PATH_DEV_I2C_1_DATA,
            Node::DevGpio => PATH_DEV_GPIO,
            Node::DevGpioPin(n) => PATH_DEV_GPIO_PIN_BASE + (n.saturating_sub(1)) as u64,
            Node::DevGpioPinCtl(n) => PATH_DEV_GPIO_PIN_CTL_BASE + (n.saturating_sub(1)) as u64,
            Node::DevGpioPinLevel(n) => PATH_DEV_GPIO_PIN_LEVEL_BASE + (n.saturating_sub(1)) as u64,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Node::Root => "/",
            Node::Readme => "README",
            Node::Ctl => "ctl",
            Node::Sys => "sys",
            Node::SysBoard => "board",
            Node::SysVersion => "version",
            Node::SysUptime => "uptime",
            Node::SysReboot => "reboot",
            Node::Dev => "dev",
            Node::DevLed => "led",
            Node::DevLedCtl => "ctl",
            Node::DevLedState => "state",
            Node::DevDisplay => "display",
            Node::DevDisplayCtl => "ctl",
            Node::DevDisplayBrightness => "brightness",
            Node::DevDisplayFb => "fb",
            Node::DevDisplayInfo => "info",
            Node::DevImu => "imu",
            Node::DevImuCtl => "ctl",
            Node::DevImuAccel => "accel",
            Node::DevImuGyro => "gyro",
            Node::DevButtons => "buttons",
            Node::DevButtonsCtl => "ctl",
            Node::DevBtnA => "a",
            Node::DevBtnB => "b",
            Node::DevBtnEvent => "event",
            Node::DevPower => "power",
            Node::DevPowerCtl => "ctl",
            Node::DevPowerBattery => "battery",
            Node::DevPowerVbat => "vbat_mv",
            Node::DevBuzzer => "buzzer",
            Node::DevBuzzerCtl => "ctl",
            Node::DevMic => "mic",
            Node::DevMicCtl => "ctl",
            Node::DevMicPcm => "pcm",
            Node::DevSpk => "spk",
            Node::DevSpkCtl => "ctl",
            Node::DevSpkPcm => "pcm",
            Node::DevSpkInfo => "info",
            Node::DevI2c => "i2c",
            Node::DevI2c1 => "1",
            Node::DevI2c1Ctl => "ctl",
            Node::DevI2c1Scan => "scan",
            Node::DevI2c1Data => "data",
            Node::DevGpio => "gpio",
            Node::DevGpioPin(n) => gpio_pin_name(n),
            Node::DevGpioPinCtl(_) => "ctl",
            Node::DevGpioPinLevel(_) => "level",
        }
    }

    pub fn mode(self) -> u32 {
        match self {
            Node::Root | Node::Sys | Node::Dev | Node::DevLed | Node::DevDisplay
            | Node::DevImu
            | Node::DevButtons
            | Node::DevPower
            | Node::DevBuzzer
            | Node::DevMic
            | Node::DevSpk
            | Node::DevI2c
            | Node::DevI2c1
            | Node::DevGpio => {
                0x80000000 | 0o555
            }
            Node::DevGpioPin(_) => 0x80000000 | 0o555,
            Node::DevLedCtl
            | Node::Ctl
            | Node::SysReboot
            | Node::DevDisplayCtl
            | Node::DevDisplayBrightness
            | Node::DevDisplayFb
            | Node::DevImuCtl
            | Node::DevButtonsCtl
            | Node::DevBtnEvent
            | Node::DevPowerCtl
            | Node::DevBuzzerCtl
            | Node::DevMicCtl
            | Node::DevSpkCtl
            | Node::DevSpkPcm
            | Node::DevI2c1Ctl
            | Node::DevI2c1Data => MODE_FILE_WR,
            Node::DevGpioPinCtl(_) | Node::DevGpioPinLevel(_) => MODE_FILE_WR,
            _ => MODE_FILE_RD,
        }
    }

    pub fn length(self) -> u64 {
        match self {
            Node::Readme => crate::readme::TEXT.len() as u64,
            Node::DevDisplayFb => devices::display::FB_LEN as u64,
            // Streaming files: large fake length prevents v9fs page cache from
            // returning EOF based on i_size=0 before data is available.
            Node::DevBtnEvent | Node::DevMicPcm | Node::DevSpkPcm => u32::MAX as u64,
            _ => 0,
        }
    }

    pub fn walk(self, name: &str) -> Option<Node> {
        if name.is_empty() || name == "." || name == "/" {
            return Some(self);
        }
        if name == ".." {
            return Some(Node::Root);
        }
        match (self, name) {
            (Node::Root, "README") => Some(Node::Readme),
            (Node::Root, "ctl") => Some(Node::Ctl),
            (Node::Root, "sys") => Some(Node::Sys),
            (Node::Root, "dev") => Some(Node::Dev),
            (Node::Sys, "board") => Some(Node::SysBoard),
            (Node::Sys, "version") => Some(Node::SysVersion),
            (Node::Sys, "uptime") => Some(Node::SysUptime),
            (Node::Sys, "reboot") => Some(Node::SysReboot),
            (Node::Dev, "led") => Some(Node::DevLed),
            (Node::Dev, "display") => Some(Node::DevDisplay),
            (Node::Dev, "imu") => Some(Node::DevImu),
            (Node::Dev, "buttons") => Some(Node::DevButtons),
            (Node::Dev, "power") => Some(Node::DevPower),
            (Node::Dev, "buzzer") => Some(Node::DevBuzzer),
            (Node::Dev, "mic") => Some(Node::DevMic),
            (Node::Dev, "spk") => Some(Node::DevSpk),
            (Node::Dev, "i2c") => Some(Node::DevI2c),
            (Node::DevI2c, "1") => Some(Node::DevI2c1),
            (Node::DevI2c1, "ctl") => Some(Node::DevI2c1Ctl),
            (Node::DevI2c1, "scan") => Some(Node::DevI2c1Scan),
            (Node::DevI2c1, "data") => Some(Node::DevI2c1Data),
            (Node::Dev, "gpio") => Some(Node::DevGpio),
            (Node::DevGpio, name) => {
                let n: u8 = name.parse().ok()?;
                if devices::gpio::CLAIMABLE_PINS.contains(&n) {
                    Some(Node::DevGpioPin(n))
                } else {
                    None
                }
            }
            (Node::DevGpioPin(n), "ctl") => Some(Node::DevGpioPinCtl(n)),
            (Node::DevGpioPin(n), "level") => Some(Node::DevGpioPinLevel(n)),
            (Node::DevLed, "ctl") => Some(Node::DevLedCtl),
            (Node::DevLed, "state") => Some(Node::DevLedState),
            (Node::DevDisplay, "ctl") => Some(Node::DevDisplayCtl),
            (Node::DevDisplay, "brightness") => Some(Node::DevDisplayBrightness),
            (Node::DevDisplay, "fb") => Some(Node::DevDisplayFb),
            (Node::DevDisplay, "info") => Some(Node::DevDisplayInfo),
            (Node::DevImu, "ctl") => Some(Node::DevImuCtl),
            (Node::DevImu, "accel") => Some(Node::DevImuAccel),
            (Node::DevImu, "gyro") => Some(Node::DevImuGyro),
            (Node::DevButtons, "ctl") => Some(Node::DevButtonsCtl),
            (Node::DevButtons, "a") => Some(Node::DevBtnA),
            (Node::DevButtons, "b") => Some(Node::DevBtnB),
            (Node::DevButtons, "event") => Some(Node::DevBtnEvent),
            (Node::DevPower, "ctl") => Some(Node::DevPowerCtl),
            (Node::DevPower, "battery") => Some(Node::DevPowerBattery),
            (Node::DevPower, "vbat_mv") => Some(Node::DevPowerVbat),
            (Node::DevBuzzer, "ctl") => Some(Node::DevBuzzerCtl),
            (Node::DevMic, "ctl") => Some(Node::DevMicCtl),
            (Node::DevMic, "pcm") => Some(Node::DevMicPcm),
            (Node::DevSpk, "ctl") => Some(Node::DevSpkCtl),
            (Node::DevSpk, "pcm") => Some(Node::DevSpkPcm),
            (Node::DevSpk, "info") => Some(Node::DevSpkInfo),
            _ => None,
        }
    }

    pub fn children(self) -> &'static [Node] {
        match self {
            Node::Root => &[Node::Readme, Node::Ctl, Node::Sys, Node::Dev],
            Node::Sys => &[
                Node::SysBoard,
                Node::SysVersion,
                Node::SysUptime,
                Node::SysReboot,
            ],
            Node::Dev => &[
                Node::DevLed,
                Node::DevDisplay,
                Node::DevImu,
                Node::DevButtons,
                Node::DevPower,
                Node::DevBuzzer,
                Node::DevMic,
                Node::DevSpk,
                Node::DevI2c,
                Node::DevGpio,
            ],
            Node::DevLed => &[Node::DevLedCtl, Node::DevLedState],
            Node::DevDisplay => &[
                Node::DevDisplayCtl,
                Node::DevDisplayBrightness,
                Node::DevDisplayFb,
                Node::DevDisplayInfo,
            ],
            Node::DevImu => &[Node::DevImuCtl, Node::DevImuAccel, Node::DevImuGyro],
            Node::DevButtons => &[
                Node::DevButtonsCtl,
                Node::DevBtnA,
                Node::DevBtnB,
                Node::DevBtnEvent,
            ],
            Node::DevPower => &[
                Node::DevPowerCtl,
                Node::DevPowerBattery,
                Node::DevPowerVbat,
            ],
            Node::DevBuzzer => &[Node::DevBuzzerCtl],
            Node::DevMic => &[Node::DevMicCtl, Node::DevMicPcm],
            Node::DevSpk => &[Node::DevSpkCtl, Node::DevSpkPcm, Node::DevSpkInfo],
            Node::DevI2c => &[Node::DevI2c1],
            Node::DevI2c1 => &[Node::DevI2c1Ctl, Node::DevI2c1Scan, Node::DevI2c1Data],
            Node::DevGpio => GPIO_DIR_CHILDREN,
            Node::DevGpioPin(1) => GPIO_PIN1_CHILDREN,
            Node::DevGpioPin(2) => GPIO_PIN2_CHILDREN,
            Node::DevGpioPin(3) => GPIO_PIN3_CHILDREN,
            Node::DevGpioPin(4) => GPIO_PIN4_CHILDREN,
            Node::DevGpioPin(5) => GPIO_PIN5_CHILDREN,
            Node::DevGpioPin(6) => GPIO_PIN6_CHILDREN,
            Node::DevGpioPin(7) => GPIO_PIN7_CHILDREN,
            Node::DevGpioPin(8) => GPIO_PIN8_CHILDREN,
            _ => &[],
        }
    }
}

pub struct FsContext<'a> {
    pub board_name: &'a str,
    pub version: &'a str,
    pub uptime_ms: fn() -> u64,
    pub led_state_line: fn() -> heapless::String<32>,
    pub on_led_ctl: fn(&str) -> Result<(), &'static str>,
    pub request_reboot: fn(),
    pub read_display_fb: fn(u64, &mut [u8]) -> usize,
    pub write_display_fb: fn(u64, &[u8]) -> usize,
    pub on_display_ctl: fn(&str) -> Result<(), &'static str>,
    pub on_display_brightness: fn(&str) -> Result<(), &'static str>,
    pub read_imu_accel: fn(u64, &mut [u8]) -> usize,
    pub read_imu_gyro: fn(u64, &mut [u8]) -> usize,
    pub on_imu_ctl: fn(&str) -> Result<(), &'static str>,
    pub read_btn_a: fn(u64, &mut [u8]) -> usize,
    pub read_btn_b: fn(u64, &mut [u8]) -> usize,
    pub read_btn_event: fn(u64, &mut [u8]) -> usize,
    pub on_buttons_ctl: fn(&str) -> Result<(), &'static str>,
    pub read_power_battery: fn(u64, &mut [u8]) -> usize,
    pub read_power_vbat: fn(u64, &mut [u8]) -> usize,
    pub on_power_ctl: fn(&str) -> Result<(), &'static str>,
    pub on_buzzer_ctl: fn(&str) -> Result<(), &'static str>,
    pub read_mic_pcm: fn(u64, &mut [u8]) -> usize,
    pub on_mic_ctl: fn(&str) -> Result<(), &'static str>,
    pub write_spk_pcm: fn(u64, &[u8]) -> usize,
    pub on_spk_ctl: fn(&str) -> Result<(), &'static str>,
    /// `/dev/i2c/1/ctl` writes: `freq HZ` plus the transaction subset
    /// (`r ADDR COUNT`, `w ADDR B...`, `rw ADDR W... COUNT`).
    pub on_i2c1_ctl: fn(&str) -> Result<(), &'static str>,
    /// `/dev/i2c/1/data` writes: parse + execute a transaction line
    /// (same subset as above, minus `freq`).
    pub on_i2c1_data: fn(&str) -> Result<(), &'static str>,
    /// `/dev/i2c/1/scan` reads: triggers a fresh scan and returns the
    /// detected addresses (one hex per line).
    pub read_i2c1_scan: fn(u64, &mut [u8]) -> usize,
    /// `/dev/gpio/<N>/ctl` writes — first arg is the pin number, second
    /// is the mode line (`in`/`in-pup`/`in-pdn`/`out`/`out-od`).
    pub on_gpio_ctl: fn(u8, &str) -> Result<(), &'static str>,
    /// `/dev/gpio/<N>/level` writes — `0` or `1` (only valid on outputs).
    pub on_gpio_level: fn(u8, &str) -> Result<(), &'static str>,
    /// `/dev/gpio/<N>/level` reads — refresh the cached input level (if
    /// the pin is configured as input) and return it.
    pub refresh_gpio_level: fn(u8),
}

impl<'a> Default for FsContext<'a> {
    fn default() -> Self {
        Self {
            board_name: "unknown",
            version: "unknown",
            uptime_ms: || 0,
            led_state_line: || heapless::String::new(),
            on_led_ctl: |_| Err("no led"),
            request_reboot: || {},
            read_display_fb: |_, _| 0,
            write_display_fb: |_, _| 0,
            on_display_ctl: |_| Err("no display"),
            on_display_brightness: |_| Err("no display"),
            read_imu_accel: |_, _| 0,
            read_imu_gyro: |_, _| 0,
            on_imu_ctl: |_| Err("no imu"),
            read_btn_a: |_, _| 0,
            read_btn_b: |_, _| 0,
            read_btn_event: |_, _| 0,
            on_buttons_ctl: |_| Err("no buttons"),
            read_power_battery: |_, _| 0,
            read_power_vbat: |_, _| 0,
            on_power_ctl: |_| Err("no power"),
            on_buzzer_ctl: |_| Err("no buzzer"),
            read_mic_pcm: |_, _| 0,
            on_mic_ctl: |_| Err("no mic"),
            write_spk_pcm: |_, _| 0,
            on_spk_ctl: |_| Err("no speaker"),
            on_i2c1_ctl: |_| Err("no i2c bus"),
            on_i2c1_data: |_| Err("no i2c bus"),
            read_i2c1_scan: |_, _| 0,
            on_gpio_ctl: |_, _| Err("no gpio on this board"),
            on_gpio_level: |_, _| Err("no gpio on this board"),
            refresh_gpio_level: |_| {},
        }
    }
}

pub fn is_writable(node: Node) -> bool {
    matches!(
        node,
        Node::DevLedCtl
            | Node::SysReboot
            | Node::Ctl
            | Node::DevDisplayCtl
            | Node::DevDisplayBrightness
            | Node::DevDisplayFb
            | Node::DevImuCtl
            | Node::DevButtonsCtl
            | Node::DevBtnEvent
            | Node::DevPowerCtl
            | Node::DevBuzzerCtl
            | Node::DevMicCtl
            | Node::DevSpkCtl
            | Node::DevSpkPcm
            | Node::DevI2c1Ctl
            | Node::DevI2c1Data
            | Node::DevGpioPinCtl(_)
            | Node::DevGpioPinLevel(_)
    )
}

pub fn read_file(node: Node, ctx: &FsContext<'_>, off: u64, buf: &mut [u8]) -> usize {
    if node == Node::DevDisplayFb {
        return (ctx.read_display_fb)(off, buf);
    }
    if node == Node::DevImuAccel {
        return (ctx.read_imu_accel)(off, buf);
    }
    if node == Node::DevImuGyro {
        return (ctx.read_imu_gyro)(off, buf);
    }
    if node == Node::DevBtnA {
        return (ctx.read_btn_a)(off, buf);
    }
    if node == Node::DevBtnB {
        return (ctx.read_btn_b)(off, buf);
    }
    if node == Node::DevBtnEvent {
        return (ctx.read_btn_event)(off, buf);
    }
    if node == Node::DevMicPcm {
        return (ctx.read_mic_pcm)(off, buf);
    }
    if node == Node::DevPowerBattery {
        return (ctx.read_power_battery)(off, buf);
    }
    if node == Node::DevPowerVbat {
        return (ctx.read_power_vbat)(off, buf);
    }
    if node == Node::Readme {
        return copy_string(crate::readme::TEXT, off, buf);
    }
    if node == Node::DevI2c1Scan {
        return (ctx.read_i2c1_scan)(off, buf);
    }
    if node == Node::DevI2c1Data {
        return devices::i2c1::read_data(off, buf);
    }
    if let Node::DevGpioPinLevel(pin) = node {
        // Refresh the cached input level by sampling the hardware once.
        (ctx.refresh_gpio_level)(pin);
        let line = devices::gpio::level_line(pin);
        return copy_string(line.as_str(), off, buf);
    }
    if let Node::DevGpioPinCtl(pin) = node {
        let line = devices::gpio::ctl_status_line(pin);
        return copy_string(line.as_str(), off, buf);
    }

    let mut s: String<128> = String::new();
    match node {
        Node::SysBoard => {
            let _ = s.push_str(ctx.board_name);
            let _ = s.push('\n');
        }
        Node::SysVersion => {
            let _ = s.push_str(ctx.version);
            let _ = s.push('\n');
        }
        Node::SysUptime => {
            let ms = (ctx.uptime_ms)();
            let _ = s.push_str(&heapless::String::<32>::from(u64_to_str(ms)));
            let _ = s.push('\n');
        }
        Node::DevLedState => {
            let line = (ctx.led_state_line)();
            let _ = s.push_str(line.as_str());
        }
        Node::Ctl => {
            let _ = s.push_str("msize=4096\n");
        }
        Node::DevDisplayInfo => {
            let _ = s.push_str("st7789v2 135x240 rgb565 le\n");
        }
        Node::DevDisplayCtl => {
            let line = devices::display::ctl_status_line();
            let _ = s.push_str(line.as_str());
        }
        Node::DevDisplayBrightness => {
            let line = devices::display::brightness_line();
            let _ = s.push_str(line.as_str());
            let _ = s.push('\n');
        }
        Node::DevMicCtl => {
            let line = devices::mic::status_line();
            let _ = s.push_str(line.as_str());
        }
        Node::DevSpkCtl => {
            let line = devices::spk::status_line();
            let _ = s.push_str(line.as_str());
        }
        Node::DevSpkInfo => {
            let _ = s.push_str(devices::spk::INFO_TEXT);
        }
        Node::DevI2c1Ctl => {
            let line = devices::i2c1::status_line();
            let _ = s.push_str(line.as_str());
        }
        _ => return 0,
    }
    copy_string(&s, off, buf)
}

pub fn write_file(node: Node, ctx: &FsContext<'_>, off: u64, data: &[u8]) -> Result<usize, &'static str> {
    if node == Node::DevDisplayFb {
        let n = (ctx.write_display_fb)(off, data);
        return Ok(n);
    }
    if node == Node::DevSpkPcm {
        // Binary PCM stream — never parse as UTF-8.
        let n = (ctx.write_spk_pcm)(off, data);
        return Ok(n);
    }

    let s = core::str::from_utf8(data).map_err(|_| "bad utf8")?.trim();
    match node {
        Node::DevLedCtl => (ctx.on_led_ctl)(s).map(|_| data.len()),
        Node::SysReboot => {
            (ctx.request_reboot)();
            Ok(data.len())
        }
        Node::Ctl if s.starts_with("msize ") => Ok(data.len()),
        Node::DevDisplayCtl => (ctx.on_display_ctl)(s).map(|_| data.len()),
        Node::DevDisplayBrightness => (ctx.on_display_brightness)(s).map(|_| data.len()),
        Node::DevImuCtl => (ctx.on_imu_ctl)(s).map(|_| data.len()),
        Node::DevButtonsCtl | Node::DevBtnEvent => (ctx.on_buttons_ctl)(s).map(|_| data.len()),
        Node::DevPowerCtl => (ctx.on_power_ctl)(s).map(|_| data.len()),
        Node::DevBuzzerCtl => (ctx.on_buzzer_ctl)(s).map(|_| data.len()),
        Node::DevMicCtl => (ctx.on_mic_ctl)(s).map(|_| data.len()),
        Node::DevSpkCtl => (ctx.on_spk_ctl)(s).map(|_| data.len()),
        Node::DevI2c1Ctl => (ctx.on_i2c1_ctl)(s).map(|_| data.len()),
        Node::DevI2c1Data => (ctx.on_i2c1_data)(s).map(|_| data.len()),
        Node::DevGpioPinCtl(pin) => (ctx.on_gpio_ctl)(pin, s).map(|_| data.len()),
        Node::DevGpioPinLevel(pin) => (ctx.on_gpio_level)(pin, s).map(|_| data.len()),
        _ => Err("permission denied"),
    }
}

pub fn pack_dir_list(node: Node, off: u64, buf: &mut [u8]) -> usize {
    let children = node.children();
    let mut written = 0usize;
    let mut skip = off as usize;
    for child in children {
        let q = QidWire {
            typ: child.qid().typ,
            vers: 0,
            path: child.path(),
        };
        let mut tmp = [0u8; 128];
        let mut o = 0usize;
        crate::wire::encode_stat(&mut tmp, &mut o, q, child.mode(), child.length(), child.name());
        if skip >= o {
            skip -= o;
            continue;
        }
        let start = skip;
        skip = 0;
        let slice = &tmp[start..o];
        if written + slice.len() > buf.len() {
            break;
        }
        buf[written..written + slice.len()].copy_from_slice(slice);
        written += slice.len();
    }
    written
}

fn copy_string(s: &str, off: u64, buf: &mut [u8]) -> usize {
    let bytes = s.as_bytes();
    if off >= bytes.len() as u64 {
        return 0;
    }
    let start = off as usize;
    let n = (bytes.len() - start).min(buf.len());
    buf[..n].copy_from_slice(&bytes[start..start + n]);
    n
}

fn u64_to_str(mut n: u64) -> heapless::String<32> {
    let mut s = heapless::String::<32>::new();
    if n == 0 {
        let _ = s.push('0');
        return s;
    }
    let mut digits = heapless::Vec::<u8, 20>::new();
    while n > 0 {
        let _ = digits.push((n % 10) as u8 + b'0');
        n /= 10;
    }
    while let Some(d) = digits.pop() {
        let _ = s.push(d as char);
    }
    s
}
