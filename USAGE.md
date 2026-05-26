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
├── tmp/                PSRAM ramfs — files/dirs via mkdir, cp, rm (runtime)
├── sys/
│   ├── board           "plus2"
│   ├── version         firmware version string
│   ├── uptime          milliseconds since boot
│   ├── mac             base efuse MAC (aa:bb:cc:dd:ee:ff)
│   ├── chip            model rev cores cpu_mhz
│   ├── heap            free / used / total bytes (esp_alloc pools)
│   ├── tmpfs           `/tmp` ramfs arena + inode usage
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
    │   ├── event       edge stream (use `dd bs=12 count=1`; `cat` batches — see below)
    │   └── ctl         flush
    ├── power/
    │   ├── ctl         hold on | hold off | shutdown
    │   ├── battery     text summary
    │   └── vbat_mv     integer mV
    ├── buzzer/
    │   └── ctl         beep <hz> <ms> | stop
    ├── mic/
    │   ├── ctl         start | stop | flush | rate | bits | gain
    │   └── pcm         s16le mono PCM stream (CAPTURE BROKEN)
    ├── i2c/
    │   └── 1/          external I²C bus (Grove HY2.0 PORT.A)
    │       ├── ctl     freq <hz>; also accepts r/w/rw transaction lines
    │       ├── scan    read: probes 0x08..0x77, returns ack'd addrs
    │       └── data    write: r/w/rw transaction; read: last result bytes
    └── gpio/           user-claimable digital pins
        └── <N>/        one dir per pin (StickS3: 1..8 on Hat2 header)
            ├── ctl     mode: in | in-pup | in-pdn | out | out-od
            └── level   read: '0'/'1'; write: '0'/'1' (output only)
```

**Not on Plus2:** `dev/spk`, IR, M5PM1 rails, StickS3-only nodes. **StickS3** runs Wi‑Fi + 9P with **staggered boot** (`firmware/src/boot_gate.rs`): PMIC + IMU → WiFi/DHCP → L3B + display + codec (amp off) → `boot: ready` → fanfare → mic RX. ST7789P3 (L3B + GPIO38 BL), BMI270 @ 100 Hz, buttons G11/G12, M5PM1 VBAT, ES8311 + AW8737 speaker (`/dev/spk/*`), MEMS mic (`/dev/mic/*` @ 16 kHz). `/dev/led/*` is no‑op ([ISSUES.md](ISSUES.md)). Serial boot checklist: [ISSUES.md](ISSUES.md) (§ StickS3 boot sequencing).

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
cat sys/version        # e.g. stick9p-0.4.0-stage3-spk
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

## `/sys/mac`

Base efuse MAC address (used as Wi-Fi STA address and as input to the provisioning SSID/password derivation).

```bash
cat sys/mac          # e.g. 7c:9e:bd:11:22:33
```

---

## `/sys/chip`

Single-line SoC description: `model rev cores cpu_mhz`.

```bash
cat sys/chip
# model=esp32 rev=3.0 cores=2 cpu_mhz=240
```

| Field | Source |
|-------|--------|
| `model` | `esp32` (Plus2) or `esp32s3` (StickS3) |
| `rev` | `efuse::chip_revision()` — `major.minor` |
| `cores` | static (2 on both boards) |
| `cpu_mhz` | live `esp_hal::clock::cpu_clock()` |

---

## `/sys/heap`

Per-pool heap stats from `esp-alloc`, one line per memory kind:

- **`sram`** — sum of every internal-SRAM region registered at boot. Used for atomics, DMA buffers, locks, and anything that must stay accessible while the cache is suspended.
- **`psram`** — external PSRAM (only present if PSRAM init succeeded for the board).

```bash
cat sys/heap
# sram free=11616 used=53920 total=65536
# psram free=4144768 used=49152 total=4193920
```

| Field | Meaning |
|-------|---------|
| `total` | Pool size in bytes (sum of constituent regions) |
| `used` | Currently allocated bytes |
| `free` | Currently free bytes (`total = free + used`) |

**Caveats on PSRAM (Plus2 + StickS3):**

- Slower than SRAM (SPI/OPI-mediated). Cache helps but cold reads are ~10× slower.
- **`Atomic*` types do not work in PSRAM on ESP32/S2/S3** — esp-alloc warns this is silent UB. The allocator places allocations across all regions by capability, so anything that ends up containing atomics must be kept in SRAM. Practically: don't `Box::new(SomeStructWithAtomic)` and hope.
- DMA-capable peripherals usually require SRAM source/destination buffers.

Useful for spotting leaks: `while true; do cat sys/heap; sleep 5; done`. If a pool's `used` keeps climbing rather than oscillating, something is leaking in that pool.

---

## `/sys/tmpfs`

Stats for the **`/tmp` ramfs** (PSRAM arena + SRAM inode table). This is separate from `/sys/heap`: file bytes under `/tmp` do not show up in `psram used=…` there.

```bash
cat sys/tmpfs
# arena free=2097152 used=0 total=2097152
# inodes free=63 used=0 total=63
```

| Line | Meaning |
|------|---------|
| `arena` | PSRAM data slab reserved at boot (`free + used = total`) |
| `inodes` | Dynamic entries under `/tmp` (root inode not counted; max 63 on current firmware) |

`arena unavailable` means PSRAM `/tmp` was not initialized (e.g. StickS3 captive-portal boot before STA reconnect — `/tmp` appears after reboot with stored WiFi).

Watch `/tmp` fill: `arena used=` grows when you create/write files; `inodes used=` grows with each file or directory name.

---

## `/tmp`

On-device scratch space (Plan 9 convention). The mount shows a `tmp` directory at the root; everything under it is created at runtime.

| Op | How |
|----|-----|
| Create file | `echo … > tmp/foo` or `cp` / `dd` |
| Create dir | `mkdir tmp/capture` (nested paths OK) |
| List | `ls tmp` / `ls tmp/capture` |
| Remove file | `rm tmp/foo` |
| Remove empty dir | `rmdir tmp/capture` |
| Monitor space | `cat sys/tmpfs` (not `sys/heap`) |

**Limits (current firmware):** up to **63** files or directories (names ≤ 32 bytes); **1 MiB** data arena (Plus2) or **2 MiB** (StickS3). Binary-safe — use for PCM clips, JSON, etc.

```bash
mkdir tmp/work
echo test > tmp/work/note.txt
cat tmp/work/note.txt
cat sys/tmpfs
```

StickS3 **captive-portal boot** defers the arena until reboot with stored WiFi — `cat sys/tmpfs` shows `arena unavailable` and `Tcreate` under `tmp` fails until then.

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

Newline-delimited edge stream: `a down`, `a up`, `b down`, `b up`. Blocking read until a transition occurs.

**Quirk under `cat`:** this kernel's v9fs `p9_client_read` won't deliver short reads to userspace until its ~64 KB buffer fills. Events do reach the firmware immediately (verified via serial), but `cat` batches them ~16 at a time. Tools that issue small reads see one event at a time:

```bash
dd bs=12 count=1 if=dev/buttons/event 2>/dev/null
# a down

while dd bs=12 count=1 if=dev/buttons/event 2>/dev/null; do :; done
# a down
# a up
# b down
# b up
```

For programs you write, `read(fd, buf, 12)` returns one event per call. See `ISSUES.md` for the underlying v9fs investigation.

`cat` still works if you don't mind batching:

```bash
cat dev/buttons/event
# (events appear ~16 at a time)
```

Polling levels remains a simple alternative:

```bash
while true; do
  echo -n "a="; cat dev/buttons/a
  echo -n "b="; cat dev/buttons/b
  sleep 0.05
done
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

**StickS3 has no piezo** — the equivalent tone path is `/dev/spk/ctl fanfare` (replays the boot two‑beep) or feeding s16le PCM through `/dev/spk/pcm`. Writes to `/dev/buzzer/ctl` on StickS3 still succeed at the 9P layer for schema parity but produce no sound.

---

## `/dev/spk/ctl` *(StickS3 only)*

ES8311 codec + AW8737 1 W amp on I²S0 (MCLK=G18, BCLK=G17, LRCK=G15, DOUT=G14). Boot fanfare runs once after `boot: ready` (see [ISSUES.md](ISSUES.md)). The audio task runs a **6144-byte** circular TX ring (≈ 96 ms @ 16 kHz) via `dma_circular_buffers!` and `push_with` (boot fanfare or `/dev/spk/pcm`). BCLK/LRCK stay running while the task is alive.

| Command | Meaning |
|---------|---------|
| `start` | Begin draining `/dev/spk/pcm` into the codec |
| `stop` | Stop draining; ring keeps any queued samples |
| `flush` | Empty the PCM ring immediately |
| `fanfare` | Re-play the boot two‑beep (880 Hz → 1175 Hz, 360 ms) |
| `rate <Hz>` | Store rate (8000/16000/22050/32000/44100/48000); hardware fixed at **16000** today |
| `bits 16` | Ack only (16-bit only) |
| `gain <N>` | Software multiplier in Q8 (0..512, **256 = unity**) applied before mono→stereo expansion |

```bash
cat dev/spk/ctl
# running=0 rate=16000 gain=256 queued=0 cap=32768 under=0 fmt=s16le ch=1
echo fanfare > dev/spk/ctl     # play boot beeps any time
echo 'gain 320' > dev/spk/ctl  # +2 dB
echo start > dev/spk/ctl
# … feed samples via /dev/spk/pcm …
echo stop > dev/spk/ctl
```

`under=` counts producer underruns (audio task ran while `running=1` but the ring was empty). Non-zero means your client isn't keeping up with the 32 KiB/s drain rate.

On Plus2, `cat /dev/spk/ctl` reads as **0 bytes** and writes return `no spk on this board (use /dev/buzzer/ctl)`.

---

## `/dev/spk/pcm` *(StickS3 only)*

**Write-only** stream of **mono s16le @ 16 kHz** samples (32 KiB/s). The firmware expands each mono sample to stereo (L=R) before the DMA stage so any client can feed mono audio directly.

The ring is 32 KiB (≈ 1.0 s of audio). When it's full, Twrite returns a short count — 9P clients retry the remainder, giving natural backpressure. There's no need for a separate `flow control` ctl.

```bash
# 'say hello' through macOS → mono 16-bit PCM → speaker
say -v Samantha "stick 9p says hi" -o /tmp/hi.aiff
ffmpeg -y -i /tmp/hi.aiff -f s16le -ar 16000 -ac 1 /tmp/hi.s16

echo start > dev/spk/ctl
dd if=/tmp/hi.s16 of=dev/spk/pcm bs=4096
echo stop > dev/spk/ctl
```

```bash
# 440 Hz test tone (1 s):
python3 -c '
import math,struct,sys
for i in range(16000):
    s=int(0.4*32767*math.sin(2*math.pi*440*i/16000))
    sys.stdout.buffer.write(struct.pack("<h", s))
' > /tmp/a440.s16
echo start > dev/spk/ctl
cat /tmp/a440.s16 > dev/spk/pcm
echo stop > dev/spk/ctl
```

Reads from `/dev/spk/pcm` return 0 — it's a pipe, not a file.

---

## `/dev/spk/info` *(StickS3 only)*

Read-only one-liner describing the codec format. Useful for auto-detecting parameters from a client.

```bash
cat dev/spk/info
# fmt=s16le ch=1 rate=16000
```

---

## `/dev/i2c/1/ctl`, `/dev/i2c/1/scan`, `/dev/i2c/1/data`

External I²C bus on the Grove HY2.0 PORT.A connector (StickS3 SDA=G9, SCL=G10; Plus2 SDA=G32, SCL=G33).
Transactions execute synchronously inside the 9P session — no separate `start`/`stop`,
no async polling. Addresses are 7-bit; values accept either decimal (`16`) or hex (`0x10`).

### `ctl` — bus config + transactions

| Write | Effect |
|-------|--------|
| `freq <Hz>` | Reconfigure bus clock (range 10000…1000000, default 100000). Takes effect on the next transaction. |
| `r <addr> <count>` | Read `count` bytes from `addr`. Result lands in `data`. |
| `w <addr> <byte>…` | Write bytes to `addr`. |
| `rw <addr> <write_byte>… <read_count>` | Write-then-restart-read in one transaction (the typical "read register" pattern). |

Read of `ctl` returns one line: `freq=<hz> last=<idle|ok|err:msg>\n`.

### `scan` — bus discovery

Read of `scan` probes every 7-bit address from `0x08` to `0x77` and returns the
addresses that ACK'd, one hex per line. Each `Tread` at offset 0 re-runs the
probe, so unplugging or hot-swapping a unit between reads gives fresh results.

### `data` — transaction shortcut + result

Writes accept the same `r`/`w`/`rw` lines as `ctl` (without `freq`). Reads
return the raw response bytes from the most recent read or rw — useful for
piping into other tools (e.g. `xxd`, `od`, or a host script).

Errors: `nack` (no device at that address), `arbitration lost`, `timeout`,
`fifo overflow`. Check `cat dev/i2c/1/ctl` after a failed write to see which.

```bash
cat dev/i2c/1/scan
# 0x18
# 0x68
# 0x6e            ← (these are on the *internal* bus on StickS3; on the
                  #    external Grove port you'll see whatever you plug in)

# Read the WHO_AM_I register (0x75) on an MPU6050 at 0x68
echo 'rw 0x68 0x75 1' > dev/i2c/1/ctl
xxd dev/i2c/1/data   # 00000000: 68
cat dev/i2c/1/ctl    # freq=100000 last=ok

# Drive a Grove relay/LED expander (e.g. PCA9554 at 0x20) — set output register
echo 'w 0x20 0x03 0x00' > dev/i2c/1/data   # config: all pins output
echo 'w 0x20 0x01 0xff' > dev/i2c/1/data   # output:  all pins high

# Bump the bus to 400 kHz fast-mode
echo 'freq 400000' > dev/i2c/1/ctl
```

Max 64 bytes per direction in a single transaction (write payload, read
count, or write half of `rw`). Split larger transfers across multiple
commands; the bus retains no state between them.

---

## `/dev/gpio/<N>/ctl` and `/dev/gpio/<N>/level` *(StickS3 only)*

Per-pin digital I/O on the Hat2-Bus header. StickS3 exposes G1..G8;
Plus2 has no spare claimable pins in the v0.6 board map (G32/G33 are
permanently bound to `/dev/i2c/1`). Reading `/dev/gpio/<N>/ctl` on a
board that doesn't wire the pin returns `absent\n`.

### `ctl` — mode

| Write | Result |
|-------|--------|
| `in`, `in-z`, `input` | Floating input (no pull). Default at boot. |
| `in-pup`, `in-up`, `pullup` | Input with internal pull-up. |
| `in-pdn`, `in-down`, `pulldown` | Input with internal pull-down. |
| `out`, `out-pp`, `output` | Push-pull output. Defaults to low. |
| `out-od`, `open-drain` | Open-drain output. Idles high (Hi-Z); reads also work. |

Read returns one line: `mode=<m> [out=<0|1>] in=<0|1>\n`. `in=` always
reflects the most recent sample of the physical pin, even in output
mode (so you can verify your drive made it onto the line).

### `level` — read / write

- Read returns `0\n` or `1\n` based on the current pin level (input
  buffer sample, or output drive readback).
- Write `0` or `1` to drive the pin **only when configured as output**.
  Writes to inputs return an error.

```bash
# Drive G7 as a push-pull output, blink it at 2 Hz
echo out > dev/gpio/7/ctl
while true; do
    echo 1 > dev/gpio/7/level; sleep 0.25
    echo 0 > dev/gpio/7/level; sleep 0.25
done

# Read a button hanging off G3 (pulled-up internally)
echo in-pup > dev/gpio/3/ctl
cat dev/gpio/3/level     # → "1\n" floating / button released
                         # → "0\n" button pressed to GND
```

---

## `/dev/mic/ctl`

| Board | Hardware | Status |
|-------|----------|--------|
| **Plus2** | SPM1423 PDM (GPIO0 CLK, GPIO34 DATA) | **Broken** — `queued=0` after `start` ([ISSUES.md](ISSUES.md)) |
| **StickS3** | ES8311 ADC, 16 kHz mono s16le | **Working** — full-duplex I²S after boot fanfare; `rate` fixed at 16000 |

| Command | Meaning |
|---------|---------|
| `start` | Enable capture (flush ring, set running) |
| `stop` | Stop capture |
| `flush` | Clear PCM ring |
| `rate <Hz>` | Store rate; hardware **44100** (Plus2 PDM) or **16000** (StickS3 ES8311) |
| `bits 16` | Ack only (16-bit only) |
| `gain <N>` | Ack only (not applied yet) |

**Plus2** (broken today):

```bash
echo start > dev/mic/ctl
cat dev/mic/ctl
# running=1 rate=44100 queued=0 fmt=s16le
```

**StickS3** (after boot fanfare / `mic: rx loop entered` in serial):

```bash
echo start > dev/mic/ctl
cat dev/mic/ctl
# running=1 rate=16000 queued=… fmt=s16le
dd if=dev/mic/pcm of=clip.raw bs=4096 count=50
echo stop > dev/mic/ctl
# host:
ffmpeg -f s16le -ar 16000 -ac 1 -i clip.raw clip.wav
```

---

## `/dev/mic/pcm`

Blocking stream of **mono s16le little-endian**. Sample rate follows board: **44100 Hz** (Plus2, not capturing) or **16000 Hz** (StickS3).

**Plus2:** broken — read blocks with empty ring ([ISSUES.md](ISSUES.md)).

**StickS3:** working — `echo start > dev/mic/ctl` then `dd if=dev/mic/pcm …` as above.

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

### Speak through StickS3

```bash
echo fanfare > dev/spk/ctl        # boot two-beep on demand
# or play a WAV from the host:
ffmpeg -y -i sample.wav -f s16le -ar 16000 -ac 1 - | \
    (echo start > dev/spk/ctl; cat > dev/spk/pcm; echo stop > dev/spk/ctl)
```

### Probe a Grove I²C unit

```bash
cat dev/i2c/1/scan
# 0x44                    # example: SHT3x temp/humidity at 0x44

echo 'rw 0x44 0x24 0x00 6' > dev/i2c/1/ctl   # one-shot high-rep measure
sleep 0.02
xxd dev/i2c/1/data
```

### Hat2 GPIO blinker

```bash
echo out > dev/gpio/2/ctl
for i in $(seq 1 10); do
    echo 1 > dev/gpio/2/level; sleep 0.1
    echo 0 > dev/gpio/2/level; sleep 0.1
done
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
- **`buttons/event`** delivers per-event under small reads (`dd bs=12 count=1`) but batches under `cat` — kernel v9fs quirk, see ISSUES.
- **Plus2:** do not assume `mic/pcm` works (`queued=0`). **StickS3:** mic is live @ 16 kHz after boot fanfare.
- **Concurrent clients:** two sessions (TCP + WS) share device state; one global button event queue.
