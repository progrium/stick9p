# stick9p filesystem usage

You are reading this from `/README` on a mounted stick9p volume (M5StickC Plus2 by default).
Board-specific gaps and open bugs are noted inline; see the repo `ISSUES.md` on the host for detail.

## Conventions

- **Paths** below are relative to the mount root (e.g. `/mnt/stick`). Use `dev/...` not `/dev/...` after mount.
- **ctl files** accept UTF-8 text. One command per line unless noted (`display/ctl` `text` is special).
- **Read streaming:** `imu/accel`, `imu/gyro`, `buttons/event`, `mic/pcm` block until data is ready (9P deferred read).
- **Write:** use `echo`, `printf`, or `tee`; multi-line display ctl works best with `printf '...\n' > dev/display/ctl`.
- **9P msize:** negotiate at most **4096** bytes per message (`mount -o msize=4096` on Linux). Use `dd bs=4096` or smaller for large reads.
- **Permissions:** ctl and framebuffer files are writable; most data files are read-only.

## Tree overview (Plus2 — implemented)

```
/
├── README              ← this document
├── ctl                 server msize hint
├── sys/
│   ├── board           "plus2"
│   ├── version         firmware version string
│   ├── uptime          milliseconds since boot
│   └── reboot          write anything → reboot
└── dev/
    ├── led/
    │   ├── ctl         on | off | blink <ms_on> <ms_off>
    │   └── state       current mode (read)
    ├── display/
    │   ├── ctl         on | off | flush | fill | text | font | scale
    │   ├── brightness  0–255 (stored; panel ~on/off on Plus2)
    │   ├── fb          64800 bytes RGB565 LE framebuffer
    │   └── info        panel description (read)
    ├── imu/
    │   ├── ctl         rate <Hz>
    │   ├── accel       latest sample line (text)
    │   └── gyro        latest sample line (text)
    ├── buttons/
    │   ├── a           "0\n" or "1\n" level (1 = pressed)
    │   ├── b           same
    │   ├── event       edge stream (BROKEN — see below)
    │   └── ctl         flush
    ├── power/
    │   ├── ctl         hold on | hold off | shutdown
    │   ├── battery     text summary
    │   └── vbat_mv     integer mV
    ├── buzzer/
    │   └── ctl         beep <hz> <ms> | stop
    └── mic/
        ├── ctl         start | stop | flush | rate | bits | gain
        └── pcm         s16le mono PCM stream (CAPTURE BROKEN)
```

**Not on Plus2:** `dev/spk`, IR, M5PM1 rails, StickS3-only nodes. **StickS3** firmware profile exists but device tasks are not wired yet.

---

## `/ctl`

Server control (read for hints; limited write support).

| Read | Write |
|------|-------|
| `msize=4096\n` | `msize <N>` (accepted; server still caps at 4096) |

```bash
cat ctl
```

---

## `/sys/board`

Read-only board id.

```bash
cat sys/board          # plus2
```

---

## `/sys/version`

Firmware version string (includes stage tag).

```bash
cat sys/version        # e.g. stick9p-0.3.0-stage3-mic
```

---

## `/sys/uptime`

Milliseconds since boot, one integer per line.

```bash
cat sys/uptime
```

---

## `/sys/reboot`

Write any UTF-8 payload to trigger immediate software reset.

```bash
echo reboot > sys/reboot
```

---

## `/dev/led/ctl`

GPIO19 red LED (active-high on Plus2).

| Command | Meaning |
|---------|---------|
| `on` | LED on |
| `off` | LED off |
| `blink <ms_on> <ms_off>` | Toggle intervals in milliseconds |

```bash
echo on > dev/led/ctl
echo off > dev/led/ctl
echo 'blink 200 200' > dev/led/ctl
echo 'blink 50 950' > dev/led/ctl    # short flash
```

Errors return I/O error on write (bad syntax).

---

## `/dev/led/state`

Read-only snapshot: `on`, `off`, or `blink <hi> <lo>` plus newline.

```bash
cat dev/led/state
# blink 200 200
```

---

## `/dev/display/ctl`

ST7789 135×240 RGB565. Commands can be combined in one write (newline-separated). **`text`** payloads may contain `\n` for wrapped lines; the next line that looks like another verb ends the text string.

| Command | Meaning |
|---------|---------|
| `on` | Enable panel output path |
| `off` | Disable panel |
| `flush` | Mark framebuffer dirty (push to panel) |
| `fill RRGGBB` | Solid fill, 6 hex digits |
| `text X Y RRGGBB <string>` | Draw text at pixel (X,Y), color hex |
| `font builtin` | Builtin 8×8 font (only font) |
| `scale 1` / `scale 2` | Text scale |

Read returns ctl status line from driver (`font=builtin scale=1` style).

```bash
echo on > dev/display/ctl
echo off > dev/display/ctl
echo 'fill 0000ff' > dev/display/ctl
echo flush > dev/display/ctl

printf 'text 4 20 ff0000 Hello\n' > dev/display/ctl
printf 'text 4 40 00ff00 Line two\nfill 000010\n' > dev/display/ctl

cat dev/display/ctl
```

---

## `/dev/display/brightness`

Write ASCII decimal **0–255**; read returns current value. On Plus2 the backlight is effectively **on/off** (no smooth dimming) — value is stored but PWM curve is limited.

```bash
echo 255 > dev/display/brightness
echo 0 > dev/display/brightness
cat dev/display/brightness
```

---

## `/dev/display/fb`

Binary framebuffer: **64800** bytes = 135 × 240 × 2, **RGB565 little-endian**. Seekable read/write at byte offsets.

```bash
# Read entire fb (large)
dd if=dev/display/fb of=screen.raw bs=4096

# Write raw RGB565 (must be exact size for full replace)
dd if=screen.raw of=dev/display/fb bs=4096
echo flush > dev/display/ctl
```

Pixel at (x,y): offset `(y * 135 + x) * 2` bytes.

---

## `/dev/display/info`

Read-only: `st7789v2 135x240 rgb565 le`

```bash
cat dev/display/info
```

---

## `/dev/imu/ctl`

MPU6886 (Plus2). Sets poll rate for the firmware IMU task.

| Command | Meaning |
|---------|---------|
| `rate <Hz>` | Sample rate hint (firmware uses 25–200 Hz range; common: 25, 50, 100, 200) |

```bash
echo 'rate 50' > dev/imu/ctl
echo 'rate 100' > dev/imu/ctl
```

---

## `/dev/imu/accel`

Read returns **one line** of latest accelerometer sample (text): three integers ax ay az, then `\n`. Not a continuous stream in current firmware — re-read to poll.

```bash
cat dev/imu/accel
# 123 -456 789
```

Typical pattern:

```bash
while true; do cat dev/imu/accel; sleep 0.1; done
```

---

## `/dev/imu/gyro`

Same as accel for gyroscope: `gx gy gz\n`.

```bash
cat dev/imu/gyro
```

---

## `/dev/buttons/a` and `/dev/buttons/b`

Instantaneous level: **`1`** = pressed, **`0`** = released (active-low GPIO, inverted in software).

```bash
cat dev/buttons/a
cat dev/buttons/b
```

---

## `/dev/buttons/event`

**Status: broken on current firmware.** Intended: newline-delimited edges `a down\n`, `a up\n`, `b down\n`, `b up\n`. Blocking read should wait for transitions; today **`cat` blocks with no output**.

Workaround: poll levels in a loop:

```bash
while true; do
  echo -n "a="; cat dev/buttons/a
  echo -n "b="; cat dev/buttons/b
  sleep 0.05
done
```

When fixed:

```bash
cat dev/buttons/event
# a down
# a up
```

---

## `/dev/buttons/ctl`

| Command | Meaning |
|---------|---------|
| `flush` | Clear queued events |

```bash
echo flush > dev/buttons/ctl
echo flush > dev/buttons/event    # alternate flush target
```

---

## `/dev/power/ctl`

Plus2 battery / hold pin (GPIO4 hold line).

| Command | Meaning |
|---------|---------|
| `hold on` | Keep device powered (default) |
| `hold off` | Release hold (may power off on battery) |
| `shutdown` | Same as hold off |

```bash
echo 'hold on' > dev/power/ctl
echo shutdown > dev/power/ctl      # caution: may power off
```

---

## `/dev/power/battery`

One-line text snapshot:

```bash
cat dev/power/battery
# vbat_mv=3850 charging=0 source=BAT
```

---

## `/dev/power/vbat_mv`

Plain integer millivolts + newline.

```bash
cat dev/power/vbat_mv
# 3850
```

---

## `/dev/buzzer/ctl`

Piezo on GPIO2. Avoid long beeps during mic experiments (shared analog path per M5 docs).

| Command | Meaning |
|---------|---------|
| `beep <freq_hz> <duration_ms>` | Queue one tone |
| `stop` | No-op ack |

```bash
echo 'beep 1000 100' > dev/buzzer/ctl
echo 'beep 2000 50' > dev/buzzer/ctl
```

---

## `/dev/mic/ctl`

SPM1423 PDM mic (GPIO0 clock, GPIO34 data). **Capture path not working:** `queued=` stays 0 after `start`.

| Command | Meaning |
|---------|---------|
| `start` | Enable capture (flush ring, set running) |
| `stop` | Stop capture |
| `flush` | Clear PCM ring |
| `rate <Hz>` | Store rate (8000, 16000, 32000, 44100, 48000); hardware fixed **44100** on ESP32 today |
| `bits 16` | Ack only (16-bit only) |
| `gain <N>` | Ack only (not applied yet) |

```bash
echo start > dev/mic/ctl
cat dev/mic/ctl
# running=1 rate=44100 queued=0 fmt=s16le
echo stop > dev/mic/ctl
echo flush > dev/mic/ctl
```

When capture works, expect `queued=` to grow while speaking.

---

## `/dev/mic/pcm`

**Status: broken** — read blocks forever with empty ring. Intended: blocking stream of **mono s16le little-endian @ 44100 Hz**.

Planned usage:

```bash
echo start > dev/mic/ctl
dd if=dev/mic/pcm of=clip.raw bs=4096 count=100
echo stop > dev/mic/ctl
# host:
ffmpeg -f s16le -ar 44100 -ac 1 -i clip.raw clip.wav
```

---

## Quick recipes

### Blink LED

```bash
echo 'blink 300 300' > dev/led/ctl
```

### Full-screen color + message

```bash
echo on > dev/display/ctl
echo 'fill 001020' > dev/display/ctl
printf 'text 10 100 ffffff stick9p\n' > dev/display/ctl
echo flush > dev/display/ctl
```

### Monitor tilt (accel poll)

```bash
echo 'rate 50' > dev/imu/ctl
for i in $(seq 1 20); do cat dev/imu/accel; sleep 0.2; done
```

### Battery check

```bash
cat dev/power/battery
cat dev/power/vbat_mv
```

### Safe reboot from mount

```bash
echo 1 > sys/reboot
```

---

## 9P / client notes for automations

- **Attach/walk** standard 9P2000; root fid walks `dev`, `sys`, `README`.
- **Large reads:** use offset/count; `README` length is non-zero in stat (full doc size).
- **Framebuffer** length is 64800 in stat.
- **Do not** assume `buttons/event` or `mic/pcm` work without checking `cat dev/mic/ctl` / ISSUES.
- **Concurrent clients:** two sessions (TCP + WS) share device state; one global button event queue.
