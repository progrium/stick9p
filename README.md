# stick9p — M5Stack Stick 9P file server

Plan 9–style filesystem over **9P2000** on **M5StickC Plus2** (default) and **M5Stack StickS3**. Firmware is `no_std` Rust on **esp-hal 1.x** + **Embassy**: Wi‑Fi provisioning, TCP **564**, WebSocket **8080**, and a static device tree wired through `ninep::fs` + `devices/*`.

Full architecture and hardware targets: [DESIGN.md](DESIGN.md). Open bugs: [ISSUES.md](ISSUES.md).

## Status (Plus2)

| Area | State |
|------|--------|
| Wi‑Fi captive portal + NVS STA | Works |
| 9P TCP `:564` / WS `:8080` | Works |
| LED, display, IMU, buttons (levels), power, buzzer | Works |
| `/dev/buttons/event` | Works; `cat` batches ~16 events (kernel quirk) — [ISSUES.md](ISSUES.md) |
| `/dev/display/brightness` | On/off only — dimming deferred |
| `/dev/mic/pcm` | Tree + ctl present; **no capture** (`queued=0`) — [ISSUES.md](ISSUES.md) |
| StickS3 (`board-sticks3`) | Wi‑Fi + 9P + ST7789P3 display, BMI270 IMU, buttons G11/G12, M5PM1 VBAT, ES8311 + AW8737 **boot fanfare** (staggered bring-up — [ISSUES.md](ISSUES.md)), **`/dev/spk/{ctl,pcm,info}`** + **`/dev/mic/{ctl,pcm}`** @ 16 kHz, `/dev/i2c/1`, `/dev/gpio/1..8`, `/sys/heap`. `/dev/led/*` no‑op ([ISSUES.md](ISSUES.md)) |

Firmware version string: `stick9p-0.4.0-stage3-spk` (`cat /mnt/stick/sys/version`).

## Prerequisites

| Tool | Purpose |
|------|---------|
| [rustup](https://rustup.rs/) | Rust (avoid Homebrew `rust` for ESP) |
| [espup](https://github.com/esp-rs/espup) | Xtensa toolchain |
| [espflash](https://github.com/esp-rs/espflash) | Flash + serial monitor |
| flip-link, ldproxy | `cargo install flip-link ldproxy` |

## One-time setup

**Plus2 (ESP32)** — default target:

```bash
cargo install espup espflash flip-link ldproxy --locked
espup install --toolchain-version 1.95.0.0 --skip-version-parse --targets esp32
source ~/export-esp.sh
```

**StickS3 (ESP32-S3)** — optional:

```bash
espup install --toolchain-version 1.95.0.0 --skip-version-parse --targets esp32s3
source ~/export-esp.sh
```

`./scripts/setup.sh` installs the **esp32s3** toolchain and builds firmware; for Plus2-only work, prefer the `esp32` commands above.

## Build and flash

From the repo root with `~/export-esp.sh` sourced:

```bash
cargo build -p firmware
cargo run -p firmware    # flash + monitor (USB serial, CH9102 on Plus2)
```

StickS3 (Wi‑Fi + 9P + display + IMU + buttons + battery + boot fanfare):

```bash
cargo build -p firmware --no-default-features --features board-sticks3 --target xtensa-esp32s3-none-elf
cargo run -p firmware --no-default-features --features board-sticks3 --target xtensa-esp32s3-none-elf
```

The repo default `.cargo/config.toml` target is **Plus2** (`xtensa-esp32-none-elf`); StickS3 builds must pass `--target xtensa-esp32s3-none-elf` (or override `build.target` in a local config).

**StickS3 serial / flash notes:** Native USB-JTAG — use `--before usb-reset --after hard-reset` (already set in the S3 runner). If the monitor shows `boot:0x23 (DOWNLOAD)` and `waiting for download`, the chip is in the ROM bootloader, not running firmware: close other serial tools, **quick-press** the side reset button once, then re-flash. Run `espflash monitor` from Terminal.app/iTerm (Cursor’s terminal often fails with “Failed to initialize input reader”). On boot the LCD shows a `stick9p / booting…` banner, then a green **READY / ip …** banner once WiFi associates, plus a two-tone (880 Hz → 1175 Hz) fanfare from the speaker — visible *and* audible confirmation that the new flash came up.

## First boot and mount

1. **Provision** — Serial shows AP name/password, e.g. `Stick9p-a3f2` / `a1b2c3d4`. Join that Wi‑Fi on a phone and open **http://192.168.4.1/**, enter home SSID/password, submit. Device reboots to STA; serial prints `net: ip …` and 9P on TCP/564 and WS/8080.

2. **Mount** (Linux; replace `<ip>`):

```bash
sudo mkdir -p /mnt/stick
sudo mount -t 9p -o trans=tcp,port=564,version=9p2000,msize=4096 <ip> /mnt/stick
cat /mnt/stick/sys/board      # plus2
cat /mnt/stick/sys/version
ls /mnt/stick/dev
```

Use **`msize=4096`** (server caps negotiated size to fit Plus2 buffers). Reads larger than ~4 KiB per `dd` block are clamped; `bs=4096` is safe.

3. **WebSocket** (optional): same 9P session over `ws://<ip>:8080/9p` (binary frames).

**Plus2 pins (firmware):** status LED **GPIO19** (active-high). **GPIO4** held on boot (future factory reset). **HOLD** line documented in DESIGN for later.

## 9P tree on Plus2 (implemented)

```
/
├── README              # compiled from USAGE.md (full path docs)
├── ctl                 # msize, server ctl
├── sys/
│   ├── board           # plus2
│   ├── version
│   ├── uptime
│   └── reboot          # write to reboot
└── dev/
    ├── led/ctl, state
    ├── display/ctl, brightness, fb, info
    ├── imu/ctl, accel, gyro      # MPU6886
    ├── buttons/a, b, event, ctl  # event: batched under cat — ISSUES.md
    ├── power/ctl, battery, vbat_mv
    ├── buzzer/ctl
    └── mic/ctl, pcm              # pcm: not capturing — ISSUES.md

```

Not on Plus2: `/dev/spk`, IR, M5PM1 power rails, StickS3-only nodes (see DESIGN §6). On StickS3 the tree adds `dev/spk/{ctl,pcm,info}`, `dev/mic/{ctl,pcm}`, `dev/i2c/1`, `dev/gpio/1..8`, `sys/heap` — see [USAGE.md](USAGE.md).

### Examples

```bash
# LED (GPIO19)
echo 'on'  | sudo tee /mnt/stick/dev/led/ctl
echo 'blink 200 200' | sudo tee /mnt/stick/dev/led/ctl
cat /mnt/stick/dev/led/state

# Display (ST7789, RGB565 fb)
echo 'fill 000000' | sudo tee /mnt/stick/dev/display/ctl
printf 'text 8 8 ffffff Hello\n' | sudo tee /mnt/stick/dev/display/ctl
cat /mnt/stick/dev/display/info
# brightness 0–255: stored; panel is ~on/off on Plus2 — ISSUES.md

# IMU (text lines)
cat /mnt/stick/dev/imu/accel
cat /mnt/stick/dev/imu/gyro

# Buttons (levels work; event stream does not)
cat /mnt/stick/dev/buttons/a
cat /mnt/stick/dev/buttons/b

# Power (ADC battery sense)
cat /mnt/stick/dev/power/battery
cat /mnt/stick/dev/power/vbat_mv

# Buzzer (GPIO2) — avoid during mic experiments
echo 'beep 1000 100' | sudo tee /mnt/stick/dev/buzzer/ctl

# Speaker (StickS3 only) — ES8311 + AW8737, mono s16le @ 16 kHz
echo fanfare | sudo tee /mnt/stick/dev/spk/ctl              # replay boot beeps
ffmpeg -y -i in.wav -f s16le -ar 16000 -ac 1 - | \
    (echo start | sudo tee /mnt/stick/dev/spk/ctl; \
     sudo cp /dev/stdin /mnt/stick/dev/spk/pcm; \
     echo stop  | sudo tee /mnt/stick/dev/spk/ctl)
```

### Mic (experimental — capture not working)

PDM mic **GPIO0** (clock), **GPIO34** (data). API exists; DMA path does not fill the ring yet.

```bash
echo start > /mnt/stick/dev/mic/ctl
cat /mnt/stick/dev/mic/ctl          # running=1 queued=0 ← expected 0 today
# dd if=/mnt/stick/dev/mic/pcm of=clip.raw bs=4096 count=50   # hangs
echo stop > /mnt/stick/dev/mic/ctl
```

When capture works, `clip.raw` is mono **s16le @ 44100 Hz**:

```bash
ffmpeg -f s16le -ar 44100 -ac 1 -i clip.raw clip.wav
```

## Workspace

```
stick9p/
├── firmware/          # ESP binary (esp-hal, Embassy, board tasks)
├── ninep/             # 9P2000 wire codec, static fs (Node enum), Session server
├── devices/           # Peripheral state + ctl/read helpers (no 9P types)
├── tools/stick9p-bridge/   # Host stub (not implemented; excluded from workspace)
├── scripts/setup.sh
├── USAGE.md           # Source for 9P /README (edit here, rebuild firmware)
├── DESIGN.md          # Full design + board matrix
└── ISSUES.md          # Known bugs
```

After mount, **`cat README`** (or `less README`) serves `USAGE.md` embedded at build time.

**How the tree is implemented:** not trait-based `vfs::Node` / `Handle` (see DESIGN §5). Shipped design is `ninep/src/fs.rs` (`Node` enum + `FsContext` callbacks) and `ninep/src/server.rs` (fid table, blocking stream reads for `buttons/event` and `mic/pcm`). `ninep/src/vfs.rs` only holds `Qid` wire types.

## Development stages

| Stage | Plus2 today |
|-------|-------------|
| 1 | Wi‑Fi provision, TCP/WS 9P, `/sys/*`, `/dev/led/*` |
| 2 | Display, IMU, buttons, power, buzzer |
| 3 | `/dev/mic/*` tree + driver WIP (capture open) |
| 4+ | StickS3 (ES8311, M5PM1, USB, BLE), host bridge, WAN auth — DESIGN.md |

## License

MIT — see workspace `Cargo.toml`.
