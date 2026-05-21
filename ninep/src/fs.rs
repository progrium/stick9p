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
            | Node::DevMic => (QT_DIR, self.path()),
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
        }
    }

    pub fn mode(self) -> u32 {
        match self {
            Node::Root | Node::Sys | Node::Dev | Node::DevLed | Node::DevDisplay
            | Node::DevImu
            | Node::DevButtons
            | Node::DevPower
            | Node::DevBuzzer
            | Node::DevMic => {
                0x80000000 | 0o555
            }
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
            | Node::DevMicCtl => MODE_FILE_WR,
            _ => MODE_FILE_RD,
        }
    }

    pub fn length(self) -> u64 {
        match self {
            Node::Readme => crate::readme::TEXT.len() as u64,
            Node::DevDisplayFb => devices::display::FB_LEN as u64,
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
        _ => return 0,
    }
    copy_string(&s, off, buf)
}

pub fn write_file(node: Node, ctx: &FsContext<'_>, off: u64, data: &[u8]) -> Result<usize, &'static str> {
    if node == Node::DevDisplayFb {
        let n = (ctx.write_display_fb)(off, data);
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
