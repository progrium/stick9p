# A 9P/Wanix-Style Filesystem for M5Stack Stick Devices — Complete Design

## TL;DR

- **Build it on `esp-hal` + Embassy in pure `no_std` Rust**, with a custom hand-rolled 9P2000 server (port 564) over `embassy-net` TCP as the primary transport, **9P over WebSocket** (optional bearer auth) for browsers and Wanix, an `embassy-usb` CDC-ACM endpoint on StickS3 (UART-bridge fallback on Plus2), and a `trouble-host` BLE GATT byte-pipe where the radio exists. The `rs9p` crate (github.com/rs9p/rs9p) is Tokio-only and implements 9P2000.L exclusively (its `main()` is `#[tokio::main]` and `docs.rs/rs9p` describes it as "a tokio-based async implementation of the 9P2000.L protocol"); it is therefore unusable on ESP targets and you must write a small `no_std` 9P codec on top of `embedded-io-async`.
- **Primary target: M5Stack StickS3 (SKU K150, ESP32-S3).** Secondary target: **M5StickC Plus2 (SKU K016-P2, ESP32)** via compile-time board profiles — same 9P tree shape, but nodes are omitted or swapped where hardware differs (see §6).
- **Map every present peripheral to a Plan-9-style synthetic tree** (`/dev/display`, `/dev/imu`, `/dev/buttons`, `/dev/power`, `/dev/mic`, `/dev/spk` or `/dev/buzzer`, `/dev/ir/{tx,rx}`, `/dev/led`, `/dev/gpio`, `/dev/i2c`, `/dev/adc`, `/net`, `/sys`) using textual `ctl` files for control and binary `data` files for bulk I/O. StickS3 has **no BM8563 RTC and no piezo buzzer**; Plus2 has the opposite profile (hardware RTC, buzzer, no I²S speaker, IR TX only).
- **Zero-config WiFi after flash:** first boot (or factory reset) starts a setup **soft-AP + captive portal**; the LCD shows SSID/password and `http://192.168.4.1/`; saving credentials reboots into STA mode with TCP/564 and WebSocket live. No serial commands required for typical users.
- **Mount from anywhere**: `mount -t 9p -o trans=tcp,port=564,version=9p2000 <ip> /mnt/stick` on Linux, `9pfuse` on macOS, `wss://<ip>:8080/9p` in a browser (with optional `?token=`), or Wanix over WebSocket/WebSerial. Example: `echo 'rate 100' > /mnt/stick/dev/imu/ctl; cat /mnt/stick/dev/imu/accel`.

---

## Key Findings

### What this device actually is

The **M5Stack StickS3 (SKU K150)**, announced 23 January 2026, is the ESP32-S3 successor to the M5StickC Plus2. It is built around the **ESP32-S3-PICO-1-N8R8** SiP (dual-core Xtensa LX7 @ 240 MHz, 8 MB flash, 8 MB Octal PSRAM), a 1.14" 135 × 240 ST7789P3 LCD, a Bosch **BMI270** 6-axis IMU, a custom **M5PM1** power-management IC (I²C 0x6E), an **ES8311** I²S audio codec + **AW8737** 1 W amplifier + MEMS mic (65 dB SNR), IR TX (GPIO46) and IR RX (GPIO42), two user buttons (GPIO11, GPIO12), a side power/reset button wired to the M5PM1's `PWR_BTN`, a single green status LED gated by the M5PM1's `LED_EN` pin, USB-C OTG full-speed, 2.4 GHz WiFi + BLE 5, and HY2.0-4P Grove + Hat2-Bus 2.54-16P expansion. There is **no BM8563 RTC**, **no separate AXP2101/AXP192**, **no piezo buzzer**, **no magnetometer, and no Hall sensor**. The M5Stack `product_i2c_addr` index lists exactly three devices on the internal bus: BMI270 (0x68), M5PM1 (0x6e), ES8311 (0x18).

### Why Plan 9 fits

The "everything is a filesystem" model collapses every peripheral into a single, language-agnostic, network-mountable API. 9P is small enough to implement in a few hundred lines of `no_std` Rust, and the Linux kernel already speaks it (`mount -t 9p`). Wanix v0.3-preview (tractordev/wanix, September 2025) recently re-validated this idea by stating verbatim that "we've ended up with a radically simple architecture around per-process namespaces composed of file service capabilities using similar design patterns to those found in Plan 9", and that "the Wanix microkernel is now simply a VFS module with several built-in file services exposed via a standard filesystem API. This ends up making the module itself a file service." That is exactly the abstraction we want for a single ESP32-S3 exposing many capabilities to many possible clients.

### Rust toolchain choice

`esp-hal 1.0` (released 30 October 2025; per Scott Mabin's announcement: "Today, the Rust team at Espressif is excited to announce the official 1.0.0 release for esp-hal, the first vendor-backed Rust SDK!") + `esp-radio` (the renamed `esp-wifi`) + `esp-hal-embassy` is the right foundation. `embassy-net 0.7` provides async TCP, `embassy-usb` provides CDC-ACM, and `trouble-host` provides BLE GATT — all three plug into the same Embassy executor. `esp-idf-hal`/`std` is the wrong choice here because we want deterministic memory behaviour and an event-driven server, not FreeRTOS threads.

---

## Details

### 1. Hardware inventory (cited)

| Subsystem | Part / spec | Pins / address | Source |
|---|---|---|---|
| **SoC** | ESP32-S3-PICO-1-N8R8, dual-core Xtensa LX7 @ 240 MHz, 8 MB flash, 8 MB Octal PSRAM, WiFi 2.4 GHz, BLE 5, native USB-OTG | — | M5Stack StickS3 docs, Specifications table |
| **Display** | ST7789P3, 135×240, RGB565 | MOSI=G39, SCK=G40, RS/DC=G45, CS=G41, RST=G21, BL=G38 | StickS3 PinMap → LCD |
| **IMU** | Bosch BMI270 6-axis (accel + gyro, no magnetometer) | I²C 0x68 on SCL=G48 / SDA=G47; INT routed via M5PM1 G4 (`PYG4_IMU_INT`) | StickS3 PinMap; M5PM1 datasheet |
| **PMIC** | **M5PM1** (M5Stack proprietary). Provides charge control, multiple rails, wake timer, watchdog, NeoPixel driver (unused on StickS3), 5 GPIOs. Datasheet v1.9 (HW:5/SW:S, 22 Jan 2026). | I²C 0x6E. PYG0=`PYG0_CHG_STAT` charger STAT in, PYG1=`PYG1_IRQ`→ESP G13, PYG2=`PYG2_L3B_EN` rail enable (LCD BL / mic / spk), PYG3=`PYG3_SPK_Pulse` (AW8737 gain-set pulse), PYG4=`PYG4_IMU_INT` | M5PM1 Datasheet EN v1.9 PDF; StickS3 PinMap |
| **Battery sense** | Read over I²C from M5PM1 reg `VBAT_L/H` 0x22/0x23 (mV); `VIN_L/H` 0x24/25; `5VOUT_L/H` 0x26/27. No fuel-gauge / SoC%. | I²C only — no ESP32 ADC pin involved | M5PM1 datasheet §IV.3 |
| **"RTC"** | **No wall-clock RTC chip.** M5PM1 has a 32-bit seconds wake-up timer (regs 0x38–0x3B, action via `TIM_CFG` 0x3C: power-on/power-off/restart) and 32 bytes scratch RAM (0xA0–0xBF). Wall clock must come from SNTP over WiFi. | I²C 0x6E | M5PM1 datasheet §IV |
| **Microphone** | MEMS mic, 65 dB SNR, captured via ES8311 ADC over I²S | DIN to ES8311 (codec on G18 MCLK / G14 DOUT / G17 BCLK / G15 LRCK / G16 DIN, plus I²C 0x18) | StickS3 Specifications |
| **Speaker** | AW8737 power amp + 8 Ω 1 W cavity speaker driven by ES8311 DAC; amplifier enable/gain pulsed by M5PM1 PYG3 (`AW8737A_PULSE` register 0x53) | I²S as above | StickS3 Specifications |
| **Buttons** | KEY1=GPIO11, KEY2=GPIO12, side power/reset on M5PM1 `PWR_BTN` (single click=power-on, double click=power-off, long press=download mode) | — | StickS3 PinMap & "Button Operation Instructions" |
| **IR** | TX=GPIO46, RX=GPIO42; RX must use RMT peripheral; speaker amp must be off during IR RX | — | StickS3 docs "Infrared Reception Notes" |
| **LED** | Single discrete green LED gated by M5PM1 `LED_EN` pin; firmware-controlled flash patterns (500 ms in download mode, etc.) — driven by `PWR_CFG` bit 4 | M5PM1 only | M5PM1 datasheet §V.4 |
| **GPIO / Grove** | HY2.0-4P PORT.A: GND, 5V, G9, G10 (default I²C/UART/IO) | — | StickS3 PinMap → HY2.0-4P |
| **Hat2-Bus** | 16-pin: GND, EXT_5V, BOOT, G1, G8, BAT, 3V3_L2, 5V_IN, G5, G4, G6, G7, G43, G44, G2, G3 | — | StickS3 PinMap → Hat2-Bus |
| **ADC** | ESP32-S3 SAR ADC1/ADC2 available on Hat2-Bus + Grove GPIOs (G1, G2, G3, G4, G5, G6, G7, G8, G9, G10, G43, G44) | — | ESP32-S3 TRM via M5Stack |
| **RMT** | ESP32-S3 RMT used for IR TX/RX (and could drive WS2812 if added externally) | — | StickS3 "must use ESP32 RMT peripheral" |
| **Radios** | WiFi 2.4 GHz b/g/n, BLE 5.0 (long-range) | shared 2.4 GHz front-end | StickS3 Specifications |
| **USB** | Native USB-OTG full-speed via ESP32-S3 internal PHY (USB-C connector); ROM also exposes USB-Serial-JTAG | — | ESP-IDF programming guide / esp-hal `otg_fs` |
| **Internal die temp** | ESP32-S3 on-die temperature sensor available via esp-hal `tsens` | — | esp-hal docs |
| **Not present** | No magnetometer, no Hall sensor, no piezo buzzer, no on-board WS2812 RGB LED, no microSD slot | — | per inventory |

### 2. Plan 9 / Wanix design philosophy used here

**The 9P2000 protocol** consists of paired T/R messages: `Tversion/Rversion` (negotiates protocol + `msize`), `Tauth/Rauth` (optional), `Tattach/Rattach` (gets a root fid/qid), `Twalk/Rwalk` (descends paths and clones a fid), `Topen/Ropen` (with `iounit`), `Tcreate/Rcreate`, `Tread/Rread` (offset+count), `Twrite/Rwrite`, `Tclunk/Rclunk` (release fid), `Tremove`, `Tstat/Rstat`, `Twstat`, plus `Tflush` and `Rerror`. Each message carries a 2-byte tag that lets the client pipeline T-messages and correlate replies. Files are identified at runtime by a **fid** (a 32-bit client-chosen handle, analogous to a Unix fd) and at the server by a **qid** (13 bytes: 1-byte type, 4-byte version, 8-byte path — analogous to an inode with versioning that survives delete/recreate).

**Plan 9 idioms** we adopt:
- "ctl + data" pattern: every interesting subsystem exposes a `ctl` file accepting human-readable commands (`echo 'rate 100' > /dev/imu/ctl`) and a `data` file (or several named data files) carrying the actual byte stream.
- One device per directory. A read of the directory yields a stat-stream of entries (the kernel client turns this into `ls`).
- Streaming reads: reading `data` on a sensor never returns "EOF"; it blocks until the next sample. This matches 9P's `Tread` semantics (the server can withhold an `Rread` arbitrarily, or return `count=0` to signal end-of-stream).
- Synthetic file servers compose: the StickS3 server is itself just another 9P file service that can be `bind`-mounted into a larger Wanix namespace.

**Wanix-specific design choices** we mirror: Wanix treats file services as **capabilities**. Transports carry implicit capability (WiFi route, USB cable, BLE pairing). **WebSocket adds optional explicit auth** (see §4.5) for internet-facing deployments; USB and captive-portal setup remain unauthenticated by default. A Wanix client can mount via **WebSocket** (preferred in-browser), WebSerial/WebUSB, or a host `stick9p-bridge` relay.

### 3. Synthetic filesystem tree (the actual design)

```
/                              (root, qid type=QTDIR)
├── README                     (read-only, board summary & build info)
├── ctl                        (server-wide ctl: 'reboot', 'shutdown', 'msize N')
├── dev/
│   ├── display/
│   │   ├── ctl                # 'on'|'off'|'invert on|off'|'rotation 0..3'|'fill RRGGBB'|'flush'|'region x y w h'|'text x y RRGGBB …'|'font builtin'|'scale 1|2'
│   │   ├── brightness         # text "0".."255"; write to set, read returns current
│   │   ├── backlight          # 'on'|'off' (drives M5PM1 PYG2_L3B_EN)
│   │   ├── info               # 'st7789p3 135x240 rgb565 le' (read-only)
│   │   └── fb                 # 64,800-byte framebuffer (135*240*2), RGB565 LE, seekable
│   ├── imu/                   # BMI270
│   │   ├── ctl                # 'rate 25|50|100|200|400|800', 'range 2|4|8|16', 'gyro_range 125|250|500|1000|2000', 'fifo on|off', 'tap on|off', 'wakeup on|off'
│   │   ├── accel              # streaming text "ax ay az\n" int16 milli-g per line OR binary 6 bytes/sample if opened OEXCL
│   │   ├── gyro               # streaming text "gx gy gz\n" milli-dps per line
│   │   ├── temp               # one line: "T=29.4\n" (BMI270 has die-temp)
│   │   ├── fifo               # raw BMI270 FIFO bytes when fifo on
│   │   └── event              # tap / wake / step events as one-line text records
│   ├── buttons/
│   │   ├── a                  # read returns "0\n" or "1\n" current level
│   │   ├── b                  # same
│   │   ├── pwr                # virtual: M5PM1-side button (single/double/long events)
│   │   └── event              # newline-delimited "a down 12345ms\n" style stream
│   ├── led/
│   │   ├── ctl                # 'on'|'off'|'blink ms_on ms_off'|'pattern dl|idle|run'
│   │   └── state              # current "on"|"off"|"blink 500 500"
│   ├── power/                 # M5PM1
│   │   ├── ctl                # 'charge on|off'|'ldo on|off'|'dcdc3v3 on|off'|'ext5v on|off'|'l3b on|off'|'shutdown'|'reset'|'download'|'cutoff_mv 2500'
│   │   ├── battery            # text snapshot: "vbat_mv=4087 charging=1 source=BAT lvp_mv=2500\n"
│   │   ├── vbat_mv            # plain integer mV (M5PM1 reg 0x22/23)
│   │   ├── vin_mv             # M5PM1 reg 0x24/25
│   │   ├── v5out_mv           # M5PM1 reg 0x26/27
│   │   ├── source             # "BAT"|"VIN"|"VINOUT"
│   │   ├── chg_stat           # "0"/"1" from PYG0_CHG_STAT
│   │   ├── rail/
│   │   │   ├── dcdc5v         # 'on'/'off'
│   │   │   ├── ldo3v3         # 'on'/'off' (powers IMU)
│   │   │   ├── dcdc3v3        # 'on'/'off' (powers ESP itself; writing 'off' will brown out the device unless on USB)
│   │   │   ├── ext5v          # 'on'/'off' (Grove/Hat EXT_5V, also gates IR TX/RX)
│   │   │   └── l3b            # 'on'/'off' (LCD BL, MIC, SPK)
│   │   ├── wdt                # write seconds to arm WDT_CNT (reg 0x0A); write 0 to disable; read remaining; clear with 0xA5 to WDT_KEY (0x0B)
│   │   ├── irq_status         # read-and-clear of M5PM1 IRQ flags
│   │   └── rtcmem             # 32 bytes of M5PM1 scratch RAM (regs 0xA0–0xBF), seekable
│   ├── rtc/                   # StickS3: M5PM1 wake timer + SNTP soft clock; Plus2: BM8563 hardware clock
│   │   ├── ctl                # S3: 'wake_after N', 'action poweron|poweroff|restart'; P2: 'alarm 2026-05-20 12:00'
│   │   ├── timer              # S3: M5PM1 countdown (read-only); P2: BM8563 alarm countdown
│   │   └── time               # "1716230400.123\n" — SNTP on S3, BM8563 on Plus2
│   ├── mic/                   # ES8311 ADC path
│   │   ├── ctl                # 'rate 8000|16000|32000|48000', 'bits 16|24', 'gain 0..7', 'start'/'stop'
│   │   └── pcm                # binary little-endian PCM stream, blocks until samples
│   ├── spk/                   # ES8311 DAC + AW8737 (StickS3 only; absent on Plus2)
│   │   ├── ctl                # 'rate 8000..48000', 'bits 16|24', 'volume 0..100', 'mute on|off', 'amp on|off'
│   │   └── pcm                # write PCM bytes, server I²S DMAs them out
│   ├── buzzer/                # Plus2 only (passive buzzer G2); absent on StickS3
│   │   └── ctl                # 'beep <freq_hz> <ms>'|'stop'
│   ├── ir/
│   │   ├── ctl                # 'carrier 38000', 'duty 33'
│   │   ├── tx                 # write raw RMT symbol pairs (binary u32 little-endian: hi_ticks<<16|lo_ticks); or text NEC: 'nec 0x20DF10EF'
│   │   └── rx                 # read decoded events: 'nec 0x20DF10EF\n' or 'raw <hex pairs>\n' (auto-mutes amp while open)
│   ├── gpio/
│   │   ├── ctl                # 'claim 7'  marks G7 as exclusive
│   │   └── N/                 # one dir per claimable GPIO (1,2,3,4,5,6,7,8,9,10,43,44 — Hat2/Grove pins)
│   │       ├── ctl            # 'mode in|out|in_pullup|in_pulldown|analog'
│   │       ├── value          # "0"/"1" — read current; write to drive (if mode=out)
│   │       └── edge           # read blocks for next rising/falling edge: 'rise 1234ms\n'
│   ├── i2c/
│   │   ├── ctl                # 'bus 0 sda 47 scl 48 hz 400000'  (re-init internal bus)
│   │   └── 0/                 # internal bus (BMI270, M5PM1, ES8311 live here — pre-claimed)
│   │       ├── ctl            # 'scan' triggers fresh scan, leaves results in scan
│   │       ├── scan           # text: "0x18\n0x68\n0x6e\n"
│   │       └── HH/            # one dir per claimed external address
│   │           ├── ctl        # 'reg 0x10' set current register pointer
│   │           ├── data       # read/write transfers (start, addr, reg if set, bytes)
│   │           └── raw        # raw transfer interface: write to send, read to recv N bytes
│   ├── spi/0/                 # SPI bus reuse of LCD bus when 'release lcd' is given via /dev/display/ctl
│   │   ├── ctl, data
│   ├── uart/                  # ESP32-S3 has 3 UARTs; expose UART1 on Grove G9/G10 when remuxed
│   │   ├── ctl, data
│   ├── adc/
│   │   ├── ctl                # 'attenuation 0|2.5|6|11 dB', 'width 9..13 bit'
│   │   └── N                  # one read-only file per claimable analog-capable pin, returns "mv=812\n"
│   └── usb/
│       ├── ctl                # 'role device|host', 'reset'
│       └── state              # 'configured', 'detached', or 'host: vid:pid'
├── net/                       # WiFi + BLE control surface
│   ├── ctl                    # 'wifi sta ssid PASS', 'wifi ap NAME PASS', 'wifi off', 'provision', 'factory_reset', 'ble adv on|off', 'mdns stick9p'
│   ├── wifi/
│   │   ├── ctl                # same commands as /net/ctl scoped to wifi
│   │   ├── scan               # read returns latest scan; write 'go' triggers fresh scan
│   │   ├── status             # "sta connected ip=192.168.1.42 rssi=-58\n" or "ap provisioning\n"
│   │   ├── ip                 # current IPv4
│   │   └── provision          # read: "mode=ap|sta|idle\nssid=...\nip=...\n"; write 'start' forces setup AP
│   ├── ble/
│   │   ├── ctl                # 'adv on', 'name M5STICKS3', 'pipe open' (open 9P-over-GATT)
│   │   ├── peers              # list of connected centrals
│   │   └── rssi               # last RSSI
│   ├── tcp/clone              # open returns new fid pointing at a tcp dir (Plan 9-style)
│   └── udp/clone
├── tmp/                       # PSRAM-backed ramfs — dynamic files/dirs (see §5.1)
│   └── …                      # user-created via Tcreate; not listed statically
└── sys/
    ├── hostname               # rw text (planned)
    ├── reboot                 # write any byte → reboot
    ├── uptime                 # seconds since boot
    ├── heap                   # esp_alloc: `sram free=… used=… total=…` (+ `psram …` when present)
    ├── tmpfs                  # `/tmp` ramfs: `arena …` + `inodes …` (not esp_alloc — §5.1)
    ├── cpu                    # "cores=2 mhz=240 temp=46.2\n" (uses ESP32-S3 internal TSENS; planned)
    ├── log                    # streaming defmt/log output (read-only)
    ├── version                # firmware version, git sha, board=sticks3|plus2
    ├── board                  # read-only: "sticks3"|"plus2" (compile-time or efuse-detected)
    └── mount/9p               # client-mounted 9P services pass-through (allows union mounts at boot)
```

#### Read/write semantics summary

- All `ctl` files: write-only behaviour-wise; reading returns current settings as `key=value` lines. Writes parse one space-separated command per write. Multiple commands per write are separated by `\n`. Plan 9 convention.
- All sensor "data" files (`accel`, `gyro`, `temp`, `vbat_mv`, etc.): each `Tread` returns either the next sample (sensor blocks until new sample arrives) or, if opened with `OREAD|OTRUNC`, returns an instantaneous snapshot string and EOFs.
- `fb` (framebuffer): seekable, 64,800 bytes, RGB565 LE. Writes are buffered until `flush` is written to `display/ctl`, OR a write that crosses the end of the buffer auto-flushes.
- **Display text** (`display/ctl`): firmware draws into the same RGB565 `fb` using a single embedded bitmap font (see below). Each `text` command auto-flushes to the panel; mixed `fb` + `text` clients should end with `flush` if ordering matters.
- `pcm` (mic, speaker): streaming binary; blocks. Backpressure is implemented by deferring `Rread`/`Rwrite` replies — 9P's natural mechanism.
- Buttons/IR rx/edge files: each `Tread` returns exactly one event line. Multiple readers each see their own event stream via per-fid queues.
- **`/tmp/*` (ramfs):** seekable regular files and directories backed by a reserved PSRAM arena (see §5.1). Reads past EOF return 0 bytes (not streaming). Binary-safe — no UTF-8 parsing on file payloads.
- `iounit` (returned in `Ropen`) is sized to fit one logical record (e.g. 6 bytes for one accel sample, 2,048 for `pcm` matching one DMA half-buffer).

#### Display text (`/dev/display/ctl`)

Clients can render strings without pushing a full framebuffer from the host. Most `ctl` verbs are one line each (`\n` separates multiple commands in one write). The **`text` verb is special**: everything after `text <x> <y> <RRGGBB> ` is the string, and **`\n` inside that payload wraps to the next display line** (X unchanged). A newline only starts a new ctl command if the following line begins with another verb (`fill`, `text`, `flush`, `scale`, …).

| Command | Meaning |
|---------|---------|
| `text <x> <y> <RRGGBB> <string…>` | Draw at pixel `(x,y)`. **Everything after the third argument** is the string (spaces allowed, no quoting). `\n` in the string starts a new line (X unchanged, Y += `8 × scale`). |
| `font builtin` | Only font in v1 (default if never set). |
| `scale 1\|2` | 1× or 2× glyph size (provisioning splash uses `scale 2`). |

**Semantics:**

- **Charset:** ASCII printable (`0x20`–`0x7E`) only; other code points → `BadCtl` (or skip — pick one at implement time and document in `/README`).
- **Font:** one embedded 8×8 bitmap in flash (~1 KB); no TTF, no UTF-8 in v1.
- **Color:** same 6-digit `RRGGBB` hex as `fill`; converted to RGB565 when blitting.
- **Clipping:** pixels outside 135×240 are dropped; no word wrap.
- **Flush:** each `text` auto-flushes after drawing (unlike raw `fb` writes).
- **Read `ctl`:** includes `font=builtin` and `scale=1` (or current values).

**Non-goals (v1):** alignment, inverse/video, word wrap, separate `/dev/display/text` stream file (revisit if long binary-safe payloads are needed).

#### Concrete usage examples

```sh
# Mount on Linux (must be root or have CAP_SYS_ADMIN; v9fs is in mainline)
mkdir -p /mnt/stick
mount -t 9p -o trans=tcp,port=564,version=9p2000,uname=$USER,msize=8192 \
      192.168.1.42 /mnt/stick

# Read battery
cat /mnt/stick/dev/power/battery        # vbat_mv=4087 charging=1 source=BAT lvp_mv=2500

# Stream IMU at 100 Hz
echo 'rate 100' > /mnt/stick/dev/imu/ctl
head -n 50 /mnt/stick/dev/imu/accel     # 50 samples then close

# Blink the LED
echo 'blink 500 500' > /mnt/stick/dev/led/ctl

# Send an IR NEC code (Sony "power")
echo 'nec 0xA90' > /mnt/stick/dev/ir/tx

# Schedule a wake in 10 minutes and power off
echo 'action poweron'      > /mnt/stick/dev/rtc/ctl
echo 'wake_after 600'      > /mnt/stick/dev/rtc/ctl
echo 'shutdown'            > /mnt/stick/dev/power/ctl

# Record 5 seconds of mic at 16 kHz
echo 'rate 16000' > /mnt/stick/dev/mic/ctl
echo 'start'      > /mnt/stick/dev/mic/ctl
dd if=/mnt/stick/dev/mic/pcm of=clip.s16 bs=32000 count=5

# Push a 135x240 PNG (after sw conversion) to the screen
convert image.png -resize 135x240 rgb565:- > /mnt/stick/dev/display/fb
echo flush > /mnt/stick/dev/display/ctl

# Draw text (firmware font; auto-flushes)
echo 'fill 000000' > /mnt/stick/dev/display/ctl
printf 'text 8 8 ffffff WiFi Setup\nNetwork: Stick9p-A1B2\nPassword: 8f3k2m9x\n' \
  > /mnt/stick/dev/display/ctl
printf 'scale 2\ntext 4 120 00ff00 Ready\nscale 1\n' > /mnt/stick/dev/display/ctl

# Drive a Grove I²C device at address 0x40 on Grove (G9=SDA, G10=SCL)
echo 'bus 1 sda 9 scl 10 hz 100000' > /mnt/stick/dev/i2c/ctl
mkdir /mnt/stick/dev/i2c/1/0x40       # implicit claim via mkdir
echo 'reg 0x06' > /mnt/stick/dev/i2c/1/0x40/ctl
printf '\xff' > /mnt/stick/dev/i2c/1/0x40/data
```

### 4. Transports

All transports share one `ninep::server::Session` implementation: a byte-oriented `embedded-io-async` stream with the standard 9P `size[4]` length prefix. Transports differ only in how bytes enter/leave the chip.

| Transport | StickS3 | Plus2 | Typical client |
|---|---|---|---|
| TCP/564 (WiFi STA) | ✓ primary | ✓ primary | `mount -t 9p`, `9pfuse` |
| WebSocket `/9p` | ✓ builtin | ✓ builtin | Browser, Wanix, `websocat` |
| USB CDC-ACM | ✓ native OTG | — (UART bridge only) | `trans=fd`, WebSerial |
| UART serial | ✓ via JTAG | ✓ CH9102 `/dev/ttyUSB*` | `stick9p-bridge`, dev |
| BLE GATT pipe | ✓ BLE 5 | ○ classic BLE (optional) | mobile, headless |
| Captive HTTP | ✓ setup only | ✓ setup only | phone browser |

#### 4.1 TCP over WiFi (primary, post-provisioning)

- Listen on **TCP/564** (registered 9P port) on the STA interface after provisioning. One `embassy-net` `TcpSocket` accept loop; one Embassy task per active 9P session. `msize` up to ~8 KiB (4 KiB default, safe on ESP RAM).
- Reachable from `mount -t 9p -o trans=tcp,port=564,version=9p2000,uname=$USER <ip> /mnt/stick` on Linux; `9pfuse 'tcp!IP!564' /mnt` on macOS.
- Latency ~3–10 ms RTT on 2.4 GHz; throughput ~1–3 MB/s — enough for ~15–40 fps full framebuffer pushes on StickS3.
- **Not available during provisioning AP mode** except on a debug port (optional `TCP/564` on AP for developers who already know the PSK).

#### 4.2 WebSocket (builtin; optional auth)

Browsers cannot open raw TCP/564. **WebSocket is a first-class transport**, not only a host-side relay.

**Endpoint (STA mode, default):**
- `ws://<hostname-or-ip>:8080/9p` — cleartext on LAN (mDNS `stick9p.local`)
- `wss://<hostname>:443/9p` — optional TLS termination on a **host reverse proxy** in production; on-device TLS is out of scope for v1 (RAM + cert rotation)

**Framing:**
- **Binary frames only** after RFC 6455 handshake.
- Payload is a **raw 9P byte stream** (same as TCP): concatenated messages each prefixed with `size[4]`. A single WS frame may carry one or more complete 9P messages; the session parser buffers until `size` bytes are available (identical to TCP codec).
- **Text frames are rejected** (close connection with code 1003) except during an optional pre-auth phase (below).

**Handshake (minimal `no_std` server on port 8080):**
1. `GET /9p HTTP/1.1` with `Upgrade: websocket`, `Connection: Upgrade`, valid `Sec-WebSocket-Key`.
2. Optional subprotocol negotiation (see auth).
3. Respond `101 Switching Protocols` + `Sec-WebSocket-Accept`; then delegate socket to the same `ninep::server` loop as TCP.

**Implementation notes:** a ~200-line WS handshake + frame encoder/decoder (mask bit on client→server only) avoids pulling in full `tungstenite` on ESP. Share the HTTP listener with the captive portal on port 80 during provisioning (different paths: `/` → setup HTML, `/9p` → WS upgrade disabled until STA mode).

**Clients:**
- Wanix / custom JS: `new WebSocket("ws://192.168.1.42:8080/9p")` + binaryType `"arraybuffer"`.
- Host relay still useful: `tools/stick9p-bridge --listen :8080 --upstream tcp://127.0.0.1:564` for dev without reflashing.
- Plan 9 heritage: Plan 9's `websocket` helper (see [p9f.org magic man websocket](https://p9f.org/magic/man2html/8/websocket)) tunnels 9P over WS — same layering we use.

#### 4.3 USB CDC-ACM (StickS3 fallback)

- **StickS3:** `esp-hal::otg_fs` + `embassy-usb` CDC-ACM. 9P runs on the byte stream; Linux `trans=fd` or **WebSerial** in Chrome.
- Linux ≥ 6.12: `trans=usbg` 9P USB-gadget transport ([KernelNewbies Linux 6.12](https://kernelnewbies.org/Linux_6.12), Pengutronix series) where applicable.
- Throughput ~1 MB/s (USB FS). Primary transport during factory bring-up before WiFi is provisioned.

#### 4.4 UART serial (Plus2 primary; StickS3 debug)

- **Plus2:** USB is a **CH9102 UART bridge** (not USB-OTG). Expose 9P on the existing serial link at 115200 (or 921600 for throughput). Users mount via `stick9p-bridge` → TCP/WS on the host. Same path as early TinyGo experiments in `.local/exp1/`.
- **StickS3:** ROM **USB-Serial-JTAG** can mirror 9P for development when WiFi is not configured.

#### 4.5 BLE GATT byte-pipe (low-power; StickS3-first)

- `trouble-host` + `esp-radio` BLE. One custom 128-bit service, two characteristics: `…0001` write-without-response (host→device), `…0002` notify (device→host).
- Negotiate `msize=240` (fits ATT MTU 247 on BLE 5). Throughput ~30–150 KiB/s practical.
- **Plus2:** ESP32 BLE exists but is not the focus; treat as optional compile feature (`board-plus2` may disable BLE to save IRAM).

#### 4.6 WebSerial / WebUSB (browser, cable-attached)

- Chrome/Edge **WebSerial** on CDC (S3) or UART (Plus2): no IP needed. Complements WS for Wanix when the device is plugged in.

#### 4.7 Auth model (transport-layer; WS optional)

We do **not** implement full Plan 9 factotum/`Tauth` crypto on-device in v1. Policy by transport:

| Transport | Default auth | Optional hardening |
|---|---|---|
| TCP/564 | None on LAN | Bind to `192.168.x.x` only; firewall; SSH tunnel |
| WebSocket | **Optional bearer token** | See below |
| USB / serial | Physical possession | — |
| BLE | Pairing + bond | — |
| Captive portal HTTP | AP PSK (printed on screen) | Short-lived open AP acceptable |

**WebSocket optional auth (recommended for WAN exposure):**

1. **Provisioning:** captive portal form includes optional field "9P access token" (or auto-generate 16-byte hex shown on display). Stored in NVS as `ninep_token`.
2. **Handshake:** client sends `Sec-WebSocket-Protocol: 9p2000, bearer-<token>` (subprotocol list per RFC 6455). Server accepts only if token matches NVS, or if NVS token is empty (auth disabled).
3. **Alternative:** query string `ws://ip:8080/9p?token=<hex>` for clients that cannot set subprotocols (slightly leakier — logs on proxies).
4. **Pre-attach gate:** if token valid but you want per-mount identity, require `Tattach` `aname` to equal `token:<hex>` — redundant with (2) but helps multi-tenant relays.
5. **`Tauth`:** always stub `Rerror "no auth"` unless we later add a Noise/file-based challenge under `/auth/`.

**TCP auth:** mirror the same token by rejecting non-loopback `Tattach` unless `aname` matches (optional feature flag `AUTH_TOKEN`). Default off on TCP for compatibility with stock `mount -t 9p`.

**Caveat:** optional auth is **not** a substitute for TLS on the public internet — use `wss://` behind Caddy/nginx with the token as a second factor.

#### 4.8 WiFi first-boot provisioning (captive portal — simplest UX)

**Goal:** after flashing firmware, the user never types WiFi credentials over serial. Flow matches consumer IoT (ESP-Touch / SmartConfig alternatives rejected as opaque; captive portal is the most reliable cross-phone UX).

**Trigger provisioning mode when:**
- NVS key `wifi/ssid` is missing or empty, **or**
- user writes `provision` to `/net/ctl` or `/net/wifi/provision`, **or**
- user holds Button A 5 s at boot (board-specific pin), **or**
- `factory_reset` clears NVS and reboots into provision.

**Provisioning sequence:**

```
[Boot] → NVS has SSID? ─no→ PROVISIONING MODE
                              │
                              ├─ Display: SSID "Stick9p-A1B2", PSK "xxxx", URL
                              ├─ esp-radio: soft-AP 192.168.4.1/24, DHCP server
                              ├─ DNS on :53: ALL A queries → 192.168.4.1 (captive)
                              ├─ HTTP :80  GET / → setup HTML (scan + password + token)
                              │            POST /save → validate → NVS → 200 "OK rebooting"
                              └─ Reboot → STA connect → mDNS stick9p.local → TCP/564 + WS/8080
```

**Display content (StickS3 / Plus2, 135×240):**

```
  WiFi Setup
  Network: Stick9p-A1B2
  Password: 8f3k2m9x
  Open: http://192.168.4.1/
  (or wait… captive opens)
```

Firmware renders this via `display/ctl` (`fill` background, `scale 2` + `text` for the title, `scale 1` for body lines). QR code optional later. Show AP password prominently — phone must join this network first.

**Captive portal behaviour:**
- Phone joins `Stick9p-XXXX` (WPA2-PSK from display, **not** open — avoids drive-by hijacks on public AP).
- Any HTTP request (`http://neverssl.com`, `http://captive.apple.com`, etc.) hits our :80 server via DNS hijack → redirect `302` to `/`.
- **Scan before AP:** run `wifi scan` in STA-capable firmware **before** starting AP (ESP32 cannot scan while AP is up without channel switching tricks). Cache SSID list in RAM for the HTML `<select>`.
- **Setup page:** minimal embedded HTML (stored as `include_str!` or compressed in flash): SSID dropdown + manual SSID field, WPA2 password, optional 9P token, `[Save]`.
- **POST /save** body: `ssid=...&pass=...&token=...` → write NVS → `esp_restart()`.

**After reboot (STA mode):**
- Connect with stored credentials (retry backoff; display "Connecting…" / "Failed: wrong password" with button to re-enter provision).
- DHCP; announce **`stick9p.local`** via mDNS (`mdns` feature in `embassy-net` or lightweight responder).
- Start **TCP :564**, **WebSocket :8080/9p**, display STA IP + "Ready" for 10 s.

**Implementation stack (simplest path for `no_std` + Embassy):**
- Pattern matches **[esp-wifi-caddy](https://crates.io/crates/esp-wifi-caddy)** (Embassy + esp-radio, captive DNS, HTTP config) — evaluate for direct use vs porting the DNS/HTTP pieces to avoid `std`.
- DNS: single-handler "return AP IP for all names" on UDP/53.
- HTTP: one `embassy-net` TCP listener :80, ~4 KiB request buffer, no TLS, no WebSockets on :80 during provision (keep WS on 8080 STA-only).
- **Do not** run 9P on the provisioning AP by default (reduces attack surface); developers can enable `PROVISION_9P=1` compile flag.

**`/net/wifi/provision` file:** read returns `mode=ap ssid=Stick9p-A1B2 ip=192.168.4.1`; write `start` re-enters provisioning (STA will drop).

**Fallbacks if captive portal fails:**
- USB serial: `echo 'wifi sta MyNet pass' > /dev/...` via bridge (document in README).
- `/net/wifi/ctl` remains the power-user API from the design tree.

### 5. Implementation architecture in Rust

#### Crate workspace

```
stick9p/
├── Cargo.toml                 # workspace
├── firmware/                  # bin crate, no_std
│   ├── src/main.rs
│   └── src/transport/{tcp.rs, ws.rs, usb.rs, uart.rs, ble.rs, provision.rs}
├── ninep/                     # no_std 9P2000 codec + dispatcher
│   ├── src/wire.rs            # encode/decode T/R messages
│   ├── src/fs.rs              # static path table (`Node` enum), walk, read/write dispatch
│   ├── src/server.rs          # fid table, Session loop, blocking stream reads
│   └── src/vfs.rs             # `Qid` + QT_* constants only (wire types)
├── devices/                   # peripheral state; ctl/read fns — wired via `FsContext`, not 9P traits
│   ├── src/display.rs
│   ├── src/imu.rs
│   ├── src/buttons.rs
│   ├── src/power.rs           # M5PM1
│   ├── src/audio.rs           # mic + spk via ES8311
│   ├── src/ir.rs              # RMT
│   ├── src/led.rs
│   ├── src/gpio.rs
│   ├── src/i2c.rs
│   └── src/net.rs             # /net/*
└── tools/
    └── stick9p-bridge/        # std crate that re-exports CDC/BLE as TCP for `mount -t 9p`
```

#### Key dependencies (versions current as of May 2026)

- `esp-hal = "1.0"` with `esp32s3`, `esp-hal-embassy`, `esp-rtos` feature
- `esp-radio` (the new name for `esp-wifi`)
- `embassy-executor`, `embassy-time`, `embassy-sync`, `embassy-net = "0.7"` with `tcp,udp,dns,dhcpv4`
- `embassy-usb` + `embassy-usb-driver` (esp-hal `otg_fs` integration)
- `trouble-host` (BLE) + `bt-hci`
- `mipidsi` (display driver; the maintained successor to the `st7789` crate — the upstream `st7789` README explicitly notes: "v0.7 of this crate is the last release. mipidsi is a new generic driver that contains ST7789 support and should serve as a drop in replacement for this driver.")
- `embedded-graphics` (optional; can be skipped — clients push raw RGB565)
- `bmi2 = "0.1"` for the BMI270 (qrasmont/bmi2)
- A locally-written driver for `M5PM1` (no crate exists yet — write a thin I²C wrapper over the 1.9 register map)
- A locally-written driver for `ES8311` (no upstream Rust crate — port the small Espressif C reference)
- `heapless = "0.8"` for `Vec<_, N>` / `String<N>`
- **No `rs9p`**: it depends on `tokio` and is `.L`-only. We write our own ~600-LOC codec.

#### Task layout

```rust
// Pseudocode for main.rs
#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    let p = esp_hal::init(Config::default().with_cpu_clock(CpuClock::max()));
    // Per-peripheral channels for streaming sensors
    static IMU_CH: Channel<NoopRawMutex, ImuSample, 32> = Channel::new();
    static BTN_CH: Channel<NoopRawMutex, BtnEvent, 16> = Channel::new();
    static IR_RX_CH: Channel<NoopRawMutex, IrFrame, 8> = Channel::new();

    // I²C bus 0 (internal): BMI270 + M5PM1 + ES8311
    let i2c0 = I2c::new(p.I2C0, Config::default().with_frequency(400.kHz()))
        .with_sda(p.GPIO47).with_scl(p.GPIO48).into_async();
    let i2c0 = mk_static!(I2cBus, Mutex::new(i2c0));

    spawner.spawn(devices::power::task(i2c0)).unwrap();   // M5PM1 polling + IRQ
    spawner.spawn(devices::imu::task(i2c0, &IMU_CH)).unwrap();
    spawner.spawn(devices::audio::task(p.I2S0, /* ES8311 */ i2c0)).unwrap();
    spawner.spawn(devices::ir::task(p.RMT, p.GPIO46, p.GPIO42, &IR_RX_CH)).unwrap();
    spawner.spawn(devices::buttons::task(p.GPIO11, p.GPIO12, &BTN_CH)).unwrap();
    spawner.spawn(devices::display::task(/* SPI2 + GPIO39/40/45/41/21/38 */)).unwrap();
    spawner.spawn(devices::led::task(i2c0)).unwrap();     // toggles M5PM1 LED_EN bit

    // 9P tree: static `ninep::fs::Node` + `FsContext` callbacks (see §5 — not dyn Node traits)
    spawner.spawn(net::services::ninep_tcp_server(stack)).unwrap();

    // Transports — each spawns one task per active session
    let radio = esp_radio::init().unwrap();
    spawner.spawn(net::provision::task(&radio, &display)).unwrap(); // AP + captive portal if NVS empty
    spawner.spawn(transport::tcp::serve(&radio, root)).unwrap();
    spawner.spawn(transport::ws::serve(&radio, root)).unwrap();    // :8080/9p, STA only
    spawner.spawn(transport::usb::serve(p.USB0, root)).unwrap();   // sticks3
    spawner.spawn(transport::uart::serve(p.UART0, root)).unwrap(); // plus2 + debug
    spawner.spawn(transport::ble::serve(&radio, p.BT, root)).unwrap();
}
```

#### VFS: original plan vs what we built

**Original plan (not implemented):** a small object-safe async VFS — `Node` and `Handle` traits with `dyn` dispatch. Each path would be a `&'static dyn Node`; `open` would return `Box<dyn Handle>`. Peripherals would implement those traits inside `devices/`, and `ninep` would only speak trait methods. `Stat`, `VfsError`, and full walk/list/open would live in `ninep/src/vfs.rs`.

**What we built instead:** a **static path table** and **function-pointer context**, which matches the shipped Plus2 firmware and keeps stacks predictable on ESP32.

| Piece | Role |
|-------|------|
| `ninep/src/fs.rs` | `Node` enum (every path + qid), `resolve_path` / `walk`, `read_file` / `write_file`, `pack_dir_list` |
| `ninep/src/fs.rs` | `FsContext` — board name, version, and `fn` pointers for each device read/ctl hook |
| `ninep/src/server.rs` | `Session`: fid table, T-message dispatch, `Rerror` strings; deferred `Tread` for blocking streams (`buttons/event`, `mic/pcm`) |
| `ninep/src/vfs.rs` | **`Qid`, `QT_DIR`, `QT_FILE` only** — wire types for walk replies |
| `ninep/src/buffers.rs` | Per-session RX/TX/work buffers (`MSG_CAP` = 4096 on Plus2) |
| `devices/*` | Mutex/refcell state, `handle_ctl` / `try_read_*` — **no 9P imports** |
| `firmware/src/net/services.rs` | Builds `FsContext` from `devices::*`, spawns `ninep_tcp_server` / `ninep_ws_server` |

**Data flow:** client `Twalk` / `Topen` → `server` resolves fid → `Node` → `read_file` / `write_file` calls the matching `FsContext` fn → `devices` returns bytes or applies ctl text. Streaming devices use rings or queues in `devices`; the server polls and completes pending `Tread` when data appears (or blocks the session until then).

**Why we changed:** avoids `alloc` for `Box<dyn Handle>`, async traits in the tree, and per-type `impl Node` boilerplate; the full tree is known at compile time. Trade-off: adding a file means extending the `Node` enum, path constants, and usually one `FsContext` field — explicit but easy to grep.

**Peripheral pattern (as implemented):** e.g. LED — `devices/src/led.rs` holds state + `handle_ctl`; `firmware/src/led_task.rs` applies GPIO from that state; `services.rs` sets `on_led_ctl: led::handle_ctl`. IMU/buttons/display follow the same split (`firmware/src/dev/plus2.rs` tasks + `devices` readers). This replaces the trait-sketch examples below.

<details>
<summary>Original trait sketch (historical — not in codebase)</summary>

```rust
// Planned ninep/src/vfs.rs — not shipped
pub trait Handle {
    async fn read (&mut self, off: u64, buf: &mut [u8]) -> Result<usize, VfsError>;
    async fn write(&mut self, off: u64, buf: &[u8])    -> Result<usize, VfsError>;
}
pub trait Node: Send + Sync {
    async fn walk(&self, name: &str) -> Result<&'static dyn Node, VfsError>;
    async fn open(&self, mode: u8) -> Result<Box<dyn Handle + Send>, VfsError>;
}
```

LED/IMU would have used `impl Node` / `impl Handle` with `embassy_sync::Channel` in the handle's `read().await`. Shipped code uses `buttons::try_read_event`, `mic::try_read_pcm`, etc., called synchronously from `server::handle_read`.

</details>

#### Memory budget

- Framebuffer 135 × 240 × 2 = 64,800 bytes → allocate in **PSRAM** (`#[link_section = ".dram2_uninit"]` or `esp_alloc`'s PSRAM allocator). SRAM remains free for stacks.
- **`/tmp` arena (§5.1, shipped):** **2 MiB** PSRAM slab (StickS3) or **1 MiB** (Plus2), reserved at boot and excluded from `esp_alloc`; inode table (63 user slots + root, SRAM) in `devices/memfs.rs`. **`/sys/tmpfs`** reports arena and inode usage; **`/sys/heap`** does not include `/tmp` file bytes.
- Per-session 9P buffers: `msize` 4 KiB → 8 KiB RX+TX per session = 16 KiB. Limit to 4 concurrent sessions = 64 KiB.
- Per-fid state: ~128 bytes. Cap to 256 fids/session.
- Streaming channels sized 32 samples × 6 bytes ≈ 200 bytes each — negligible.

#### Backpressure & flush

When a streaming sensor's bounded channel fills (e.g. the client is slow), the producer task drops the oldest sample (overwrite-style ring) and increments a `dropped` counter exposed in `/dev/imu/ctl`'s read response. Bulk transfers (`/dev/spk/pcm`) instead use 9P's natural rate-limit: don't ack the `Twrite` until DMA has consumed half the buffer. This makes `cat file.s16 > /dev/spk/pcm` exactly as fast as the speaker plays.

### 5.1 `/tmp` — PSRAM-backed ramfs (shipped)

Expose a **normal in-memory file service** at **`/tmp`** (Plan 9 scratch-space convention). Mounted clients should be able to use familiar host tools without staging through the laptop's `/tmp`:

```sh
echo hello > /mnt/stick/tmp/greeting
mkdir /mnt/stick/tmp/capture
dd if=/mnt/stick/dev/mic/pcm of=/mnt/stick/tmp/capture/clip.s16 bs=32000 count=5
cat /mnt/stick/tmp/greeting
rm /mnt/stick/tmp/greeting
```

This closes a practical gap: mic/speaker workflows today often round-trip through the host filesystem; on-device scratch space keeps scripts entirely on the stick.

#### Why a dedicated service (not “just use the heap”)

PSRAM is also registered in the global `esp_alloc` pool (`/sys/heap` → `psram free=…`). A separate ramfs layer is still warranted:

| Concern | Global heap alone | Dedicated `/tmp` ramfs |
|---|---|---|
| **Accounting** | Mixed with framebuffer, fanfare, speaker ring | Fixed arena + inode caps at boot; **`/sys/tmpfs`** for arena/inode `free/used/total` |
| **Atomics** | Metadata can land in PSRAM → silent UB on ESP32 | **All inode metadata in SRAM**; only byte payloads in PSRAM |
| **9P semantics** | N/A | Real `Tcreate` / `Tremove`, dynamic `readdir`, offset-aware `Rread`/`Rwrite` |
| **Predictability** | Fragmentation over long uptime | One arena; delete/truncate returns space to the arena |

`/dev/display/fb` is the right model for a **single fixed blob**; `/tmp` is the right model for **many named, resizable files**.

#### Placement

`/tmp` lives at the **root**, alongside `/dev` and `/sys`, not under `/dev` — it is namespace scratch space, not a peripheral. Only the mount point is static in the tree; all children are created at runtime via `Tcreate`.

#### Architecture (fits §5 shipped layout)

Keep the existing split: static `Node` enum for hardware, dynamic inode table for `/tmp`.

```
ninep/server.rs     → Tcreate/Tremove/Twalk when fid resolves under MemRoot
ninep/fs.rs         → static anchor Node::MemRoot only (no per-file enum entries)
devices/memfs.rs    → inode table (SRAM), data arena (PSRAM), Mutex
firmware/main.rs    → reserve PSRAM slab after psram init, before heavy Box::leak alloc
```

**Second fid target** in the session (alongside static `Node`):

```rust
enum FidTarget {
    Static(Node),
    Mem(Ino),   // u16 inode index into memfs
}
```

`Twalk` on `Mem(dir)` resolves path components in `memfs`; `Twalk` on `Static` is unchanged. One static mount point, many paths underneath (within caps).

#### Storage model

**Two pools:**

1. **Inode table (SRAM)** — fixed array, compile-time caps per board profile:
   - `typ`: free / file / dir
   - `parent`, `name` (`heapless::String<32>` per component)
   - `size`, `data_off`, `data_len` into the arena
   - `qid_vers` — bump on unlink/truncate so stale fids fail cleanly

2. **Data arena (PSRAM)** — single contiguous slab carved out at boot, **not** handed to the general `esp_alloc` heap (so display framebuffer and fanfare cannot consume scratch space):

   | Board | Arena (firmware) | Rationale |
   |---|---|---|
   | StickS3 | **2 MiB** of ~8 MiB PSRAM | Leave headroom for heap (fanfare, speaker ring, WiFi); arena deferred on captive-portal boot until STA reboot |
   | Plus2 | **1 MiB** of ~2 MiB PSRAM | Same API, smaller heap slice after arena |

Allocation inside the arena: best-fit or bump+free-list by size class. **No `Atomic*` in arena blocks.** Directory entries are inode indices only (no pointers into PSRAM for metadata).

Boot sequence (after existing PSRAM init in `main.rs`):

```text
memfs::init(slab_ptr, slab_len) → register remaining PSRAM with esp_alloc::HEAP.add_region
```

The low slice is the arena; bytes above `arena_len` join the general PSRAM heap. StickS3 **provision boot** skips `memfs::init` and registers all PSRAM as heap until reboot with stored WiFi (`arena unavailable` in `/sys/tmpfs` until then).

#### 9P behavior (“normal” for Linux v9fs)

On static nodes, `Tcreate` still means “open an existing writable node.” Under **`/tmp`**, the server uses `FidTarget::Mem` and `devices/memfs`:

| Op | Behavior |
|---|---|
| **Twalk** | `tmp` → `tmp/foo` → `tmp/foo/bar` via inode parent + name lookup |
| **Tcreate** | Parent must be a `Mem` directory; `perm & DMDIR` → directory, else regular file; initial size 0 |
| **Topen** | `OTRUNC` truncates file, bumps `qid.vers` |
| **Tread / Twrite** | Honor **offset** (unlike ctl files today); reads past EOF return 0 |
| **Tremove** | Unlink; directory must be empty; reject `.` / `..` |
| **readdir** | Dynamic listing from inode children (not a static `CHILDREN` slice) |
| **Tstat** | `length` = file size; directories `length = 0` |
| **Twstat** | v1: `Rerror "not supported"`; v2: rename if needed |

**Not** streaming: no blocking reads, no zero-padding workarounds (contrast `/dev/buttons/event` on some v9fs clients). Binary payloads are never parsed as UTF-8.

**Concurrency:** one `Mutex<MemFs>` around inode table + arena (short critical sections). Multiple readers on one file are fine; v1 documents single-writer per file (typical `dd`/`cp` usage).

#### Example on-stick pipeline

```sh
echo 'rate 16000' > dev/mic/ctl
echo start > dev/mic/ctl
dd if=dev/mic/pcm of=tmp/clip.s16 bs=32000 count=5
echo stop > dev/mic/ctl
dd if=tmp/clip.s16 of=dev/spk/pcm bs=4096
```

#### Phased rollout

| Phase | Deliverable | Status |
|---|---|---|
| **1** | Arena + flat files under `/tmp` (`Tcreate` file, seek R/W, `Tremove`, `readdir`) | **Shipped** |
| **2** | Nested directories (`mkdir` = `Tcreate` with `DMDIR`); empty-dir `Tremove` | **Shipped** |
| **3** | `Twstat` rename (optional) | Not implemented |
| **—** | `/sys/tmpfs` arena + inode stats | **Shipped** |

#### Risks and mitigations

| Risk | Mitigation |
|---|---|
| PSRAM slow for many small files | Clients use reasonable `bs=`; inode cap limits tiny-file churn |
| Arena fragmentation | Size buckets, or grow-in-place truncate for same inode |
| Fid table stores inode index | `FidTarget::Mem(ino)`; `Tclunk` releases fid, not inode |
| Plus2 2 MiB PSRAM | Same code path, smaller compile-time arena and inode limits |

#### Non-goals (v1)

- Object-safe `dyn Node` VFS (§5 abandoned trait plan)
- Per-file `Node` enum entries under `/tmp`
- Inode structs stored in PSRAM
- Relying on `esp_alloc` for file bytes without a reserved slab
- Symlinks, hard links, mmap, or execute permission

---

### 6. Board profiles: StickS3 vs M5StickC Plus2

The 9P tree is **one schema**; nodes are **present, stubbed, or absent** per board. Build with Cargo features `board-sticks3` (default) or `board-plus2`. Runtime exposes `/sys/board`.

#### 6.1 Hardware comparison (authoritative deltas)

| | **StickS3 (K150)** | **Plus2 (K016-P2)** | Source |
|---|---|---|---|
| **SoC** | ESP32-S3-PICO N8R8, 8 MB flash, **8 MB OPI PSRAM** | ESP32-PICO-V3-02, 8 MB flash, **2 MB PSRAM** | M5Stack specs |
| **esp-hal target** | `esp32s3` | `esp32` | chip family |
| **PMIC** | M5PM1 @ I²C 0x6E (rails, charger, LED, wake timer) | **No PMIC** (AXP192 removed Dec 2023); **GPIO4 HOLD** keeps power; **GPIO38** battery ADC | Plus2 docs version history |
| **Display** | ST7789P3 135×240; SPI G39/40/45/41/21/38 | ST7789V2 135×240; SPI G15/13/14/12/5/27 | PinMap |
| **IMU** | BMI270 @ 0x68 | MPU6886 @ 0x68 (same address, **different driver**) | PinMap |
| **RTC** | Soft clock (SNTP) + M5PM1 wake timer only | **BM8563** @ I²C 0x51 — hardware wall clock | Plus2 PinMap |
| **Mic** | ES8311 + I²S → `/dev/mic/pcm` | SPM1423 **PDM** G0/G34 → `/dev/mic/pcm` (different codec path) or defer v1 | Plus2 specs |
| **Speaker** | ES8311 + AW8737 1 W → `/dev/spk/pcm` | **Passive buzzer** G2 → `/dev/buzzer/ctl` only | Plus2 specs |
| **IR** | TX G46 + **RX G42** (RMT) | **TX only G19** (shared with red LED); **no IR RX** | Plus2 PinMap |
| **LED** | M5PM1 `LED_EN` (green) | **Red LED G19** (active-high; shares IR); green LED is **sleep indicator only** (not in tree) | Plus2 specs + factory docs |
| **Buttons** | KEY1 G11, KEY2 G12, M5PM1 `PWR_BTN` | A G37, B G39, C G35 (power/wake) | PinMap |
| **USB** | Native **USB-OTG** CDC | **CH9102 UART** bridge only | Plus2 specs |
| **BLE** | BLE 5.0 | BLE 4.2 (ESP32); lower priority | specs |
| **Expansion** | Grove G9/G10 + Hat2 16-pin | Grove G32/G33 only | PinMap |
| **WiFi provisioning** | Same captive portal | Same captive portal | §4.8 |

#### 6.2 VFS adaptation rules

**Always present (both boards):** `/README`, `/ctl`, `/dev/display`, `/dev/imu`, `/dev/buttons`, `/dev/led` (implementation differs), `/dev/gpio`, `/dev/i2c`, `/dev/adc`, `/net`, `/sys`, `/tmp` (ramfs — §5.1), `/sys/tmpfs` (ramfs stats).

| Path | StickS3 | Plus2 |
|---|---|---|
| `/dev/power/*` | Full M5PM1 rail map | **Shrink:** `battery` from ADC G38 mV; `ctl` → `hold on\|off` (G4), `shutdown` → clear HOLD; no `rail/` subtree |
| `/dev/rtc/*` | Timer + SNTP `time` | **BM8563:** `time` from hardware; drop M5PM1 wake timer or map to BM8563 alarm regs |
| `/dev/mic/*` | ES8311 I²S | PDM SPM1423 (Stage 3+); stub `ctl` returning `unsupported` until driver lands |
| `/dev/spk/*` | I²S PCM | **Omit**; expose `/dev/buzzer/ctl` (`beep 1000 200` ms freq/duration) |
| `/dev/ir/tx` | RMT NEC + raw | RMT on G19; **no `ir/rx`** node |
| `/dev/ir/rx` | Present | **Omit** (ENOENT on walk) |
| `/dev/usb/*` | OTG state | **Omit** or stub "uart_bridge" |
| Transport default | WiFi TCP + WS; USB CDC | WiFi TCP + WS; **UART serial** primary wired |

**IMU driver swap:** compile-time `#[cfg(feature = "board-plus2")]` → `mpu6886` crate or minimal register peek; `board-sticks3` → `bmi2` + BMI270 blob upload.

**Memory budget (Plus2):** 2 MB PSRAM forces smaller `msize` (4096), fewer concurrent sessions (2), and **no large PSRAM framebuffer** — optional `/dev/display/fb` still 64 KB but allocate in internal RAM only if fit, else line-by-line `ctl region` writes only. `text` still targets the same `fb` (or a line buffer if `fb` is omitted). `/tmp` arena is ~1 MiB with fewer inodes than StickS3 (§5.1).

**Toolchain:** Plus2 builds with `espup install --targets esp32` and `xtensa-esp32-none-elf`; separate CI matrix row.

#### 6.3 Porting priority for Plus2

1. **Stage 1 parity:** blinky on red LED G19, captive portal, TCP/564, WS/8080, `/sys/*`, MPU6886 accel stream, display fb + `text` ctl (provisioning splash).
2. **Defer:** ES8311-class audio, IR RX, BLE 5, USB-OTG, M5PM1 power rails, Hat2 bus.
3. **Validate on real hardware:** HOLD pin (G4) must be set high in `main` within 2 s of boot or Plus2 powers off (documented in M5Stack "Operation Instructions").

#### 6.4 Coexistence with TinyGo experiments

The repo's `.local/exp1/` TinyGo firmware targets **Plus2-class ESP32** (`esp32-coreboard-v2`) with UART mux — useful reference for pin assignments (e.g. LED G19 active-high) but **not** the production stack. Production Plus2 uses the same `stick9p` Rust workspace with `board-plus2`.

---

## Recommendations

**Stage 1 — get a 9P shell working over TCP + provisioning:**
1. Bring up esp-hal + Embassy on StickS3 (`esp32s3`); init the M5PM1 PMIC over I²C 0x6E. ⚠ The single green LED is **autonomously driven by the M5PM1 firmware** and is not user-controllable from our side; `/dev/led/*` writes succeed at the 9P layer but produce no visible change. See ISSUES.md.
2. Implement **WiFi provisioning** (§4.8): soft-AP, captive DNS, HTTP form, NVS, display splash, reboot-to-STA. Confirm with a phone — no serial config.
3. Bring up WiFi STA, `embassy-net`, mDNS `stick9p.local`, TCP/564.
4. Write `ninep::wire` + `ninep::server`; test `mount -t 9p` and `9p ls`.
5. Wire `/sys/*` and `/dev/led/*` (schema is uniform across boards even when LED is hardware-internal).
6. Add **WebSocket** transport (`transport/ws.rs`, :8080/9p); smoke-test with browser or `websocat`.

**Stage 2 — sensors & display:**
5. Add `/dev/power` (full M5PM1 register-map exposure), `/dev/imu`, `/dev/buttons`, `/dev/ir`. Each is a separate `task` + `Node`.
6. Add `/dev/display/{fb,ctl,brightness}` with mipidsi over SPI; framebuffer in PSRAM (Plus2) or internal RAM (StickS3, 96 KB heap reservation); embedded 8×8 font + `text`/`scale` ctl commands.
   - **StickS3 ordering quirk:** the LCD's analog supply, mic supply, and speaker amp all share the **L3B** rail, gated by M5PM1 `PYG2` (FUNC1 bits[1:0] = `0b11` → `L3B_EN`, GPIO_MODE bit2 = output, GPIO_OUT bit2 = 1). Without that, GPIO38 driven high alone leaves the panel dark — `mipidsi::Builder::init` will still ACK over SPI (no MISO) so it looks fine in logs. **Boot sequencing (2026-05):** L3B is enabled only after WiFi DHCP (`boot_gate::network_ready`), not at cold boot — see `firmware/src/boot_gate.rs` and [ISSUES.md](ISSUES.md). The display task waits on `L3B_READY` before backlight/SPI.

**Stage 3 — audio:**
7. Implement an `ES8311` Rust driver (port from Espressif's BSP C code), I²S DMA via esp-hal's `i2s` module. **Status (StickS3):** minimal driver lives in `firmware/src/dev/sticks3.rs::Es8311`; the init sequence is for 16 kHz / 16-bit slave mode with MCLK from the I²S master at 256 × Fs (= 4.096 MHz). The AW8737 power amp is enabled by driving M5PM1 `PYG3` high (plain GPIO output, not the `OTHER`/`PYG3_SPK_Pulse` function — the schematic label is misleading).
8. Wire `/dev/mic/pcm` and `/dev/spk/pcm` with DMA driving Embassy channels. **Status (StickS3):** `/dev/spk/{ctl,pcm,info}` and `/dev/mic/{ctl,pcm}` are live @ **16 kHz** mono s16le. Boot fanfare runs only after `boot: ready` (codec + display + 9P); amp and DAC unmute immediately before I²S TX; RX/mic starts after fanfare (`firmware/src/boot_gate.rs`). The audio task uses `I2s::write_dma_circular_async` / `read_dma_circular_async` against **6144-byte** stereo rings (≈ 96 ms @ 16 kHz) from `dma_circular_buffers!` (word-aligned static — heap `Vec` buffers misalign circular DMA). **Pitfall:** ring size must be a **multiple of 12** so esp-hal's 3-way circular split stays on stereo-frame boundaries (otherwise metallic aliasing). **Plus2:** PDM mic path still broken (`queued=0`); see [ISSUES.md](ISSUES.md).

**Stage 4 — secondary transports & polish:**
9. Add USB CDC-ACM (StickS3) and UART serial (Plus2 / debug).
10. Add BLE transport (trouble-host); negotiate `msize=240`.
11. Add `/net`, `/dev/gpio`, `/dev/i2c/1`, `/dev/adc`. Optional WS bearer token from NVS. **Status (StickS3):** `/dev/i2c/1/{ctl,scan,data}` is live on the Grove port (SDA=G9, SCL=G10); transactions run synchronously inside the 9P session via a `critical_section`-guarded `I2c<Blocking>`. The scan loop holds the CS for one address at a time (~90 µs per NACK) so WiFi RX and audio DMA stay responsive across the ~10 ms probe. `/dev/gpio/<N>/{ctl,level}` exposes the StickS3 Hat2 header (G1..G8) as user-claimable digital I/O — each pin lives in an `esp_hal::gpio::Flex` so `in` ↔ `out` transitions don't tear down the driver. Plus2 has no spare GPIOs (G32/G33 are owned by `/dev/i2c/1`); the tree shows the dirs but `ctl` reads return `absent\n`. `/dev/adc` deferred.
12. **Plus2 bring-up:** `board-plus2` profile — MPU6886, BM8563, G19 LED, captive portal, HOLD pin, UART 9P.
13. **`/tmp` ramfs (§5.1):** **Done** — PSRAM arena + `FidTarget::Mem`, `Tcreate`/`Tremove`, nested dirs, seekable R/W, `/sys/tmpfs` stats. Enables on-device staging for mic/spk and general scratch files.

**Stage 5 — Plus2 feature parity (as needed):**
14. PDM mic, buzzer, IR TX-only; document omitted nodes in `/README`.

**Benchmarks / kill criteria:**
- If 9P latency on WiFi exceeds 20 ms RTT for small files, switch to UDP-based 9P or drop `msize`.
- If framebuffer push over TCP < 10 fps, move to a "diff-region" ctl (`'region x y w h'` followed by writes to `fb`) to send less data.
- If BLE throughput is < 30 KiB/s, drop GATT and use L2CAP CoC (already supported by trouble-host).

**Things to *not* do:**
- Don't pick `esp-idf-hal`/std. You'll fight FreeRTOS scheduling and lose the ability to compose tasks cleanly.
- Don't try to write a 9P client in firmware (this is purely a server).
- Don't require TLS on-device in v1 — terminate `wss://` on a host reverse proxy.
- Don't run **open** WiFi AP for provisioning (always WPA2 PSK printed on screen).
- Don't implement full Plan 9 `Tauth` until transport-layer tokens prove insufficient.

---

## Caveats

- **9P2000 vs 9P2000.L vs 9P2000.u**: Linux `v9fs` prefers `9p2000.L` (Linux extensions) but accepts plain `9p2000`. We implement vanilla `9p2000` because it's simpler and Plan9port/9pfuse/Wanix all speak it natively. If you need POSIX semantics from Linux (uid/gid mapping, `Tgetattr`), revisit and add `.L`. The `rs9p` crate, by contrast, is `.L`-only — another reason it's a poor fit here.
- **Battery percentage**: the M5PM1 exposes only raw mV, not state-of-charge — you'll need a Li-Po discharge-curve approximation if a "%" file is desired. We left it out by design.
- **No wall-clock RTC**: `/dev/rtc/time` is a soft clock backed by SNTP from `/net`. After power-off, time resets until WiFi reconnects. If a hardware RTC matters, you must add an external Unit on the Grove port (e.g. M5Stack RTC Unit).
- **L3B rail dependency**: writing `off` to `/dev/power/rail/l3b` immediately turns off the LCD backlight, mic, AND speaker amplifier. Don't be surprised. Likewise, `dcdc3v3 off` browns out the ESP32-S3 itself — the M5PM1 will deny that write unless the device is also on USB.
- **IR RX & speaker amp are mutually exclusive**: per M5Stack's docs, "When using the infrared receiver function, the speaker amplifier must be turned off." Our `/dev/ir/rx` open-handler asserts `'amp off'` on `/dev/spk/ctl` automatically and restores on clunk.
- **Firmware availability**: the M5PM1 datasheet cited above was version 1.9 (Jan 2026) at time of writing; future revisions may add registers or change defaults (e.g. `WDT_CNT` was changed to default-disabled in HW:5/SW:5 on 2025-11-04).
- **esp-hal 1.0** stabilises a deliberately small subset; many drivers we lean on (I²S, USB-OTG, RMT) are still marked `unstable` in late-2025 docs. Expect minor breakage and pin to a known-good commit.
- **The BMI270 boot blob**: the BMI270 requires uploading an exactly 8,192-byte configuration blob (`bmi270_config_file`) on every cold boot, per Bosch Sensortec's own API (github.com/boschsensortec/BMI270-Sensor-API) and confirmed in the qrasmont/bmi2 Rust crate README ("a configuration of > 8kB is uploaded to the sensor"). The `bmi2` crate handles this; make sure your flash partition has room and that the burst-write parameter is set generously (255 bytes works on the ESP32-S3 I²C).
- **CNX Software's January 2026 launch article** and the M5Stack product page are written as marketing material; the StickS3 schematic PDF (v0.6, dated 2025-11-11, file `K150_Stick_S3_PRJ_V0.6_20251111_2025_11_17_16_10_24.pdf`) is the authoritative source for the pin map and was the basis for all pin assignments quoted above.
- **WebSerial mount in Wanix** is conceptually clean but practical end-to-end browser mounting depends on the Wanix v0.3 file-service API surface, which is still labelled "preview" (tractordev/wanix v0.3-preview, September 2025); **WebSocket `/9p` is the preferred browser path** once Wanix supports custom WS URLs.
- **Plus2 has no USB-OTG:** browser access is WebSocket-over-WiFi or WebSerial-over-UART, not `trans=usbg`.
- **Plus2 PSRAM (2 MB):** full framebuffer + 9P sessions may require aggressive `msize` and session limits vs StickS3.
- **Captive portal during scan:** must scan WiFi networks **before** starting the AP; otherwise the SSID dropdown is empty on some phones.
- **M5StickC Plus2 green LED** is documented as non-programmable (sleep indicator); only the red LED (G19) appears under `/dev/led`.