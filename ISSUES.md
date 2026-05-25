# Known issues

Tracked bugs and gaps not yet fixed. Stage 3 adds Plus2 PDM mic (`/dev/mic`); speaker/`/dev/spk` deferred (weak buzzer only on Plus2).

---

## `/dev/buttons/event` — events delivered in batches under `cat`

**Status:** Working with v9fs quirk (events buffered ~16 deep before `cat` prints)
**Board:** M5StickC Plus2 (GPIO37 = A, GPIO39 = B, active-low); StickS3 (GPIO11/12)

### Symptoms

- `cat dev/buttons/a` / `b` reflect level correctly (`1` = pressed, `0` = released).
- `cat dev/buttons/event` produces correct lines (`a down`, `a up`, `b down`, `b up`) but **only flushes after ~16 button transitions** — `cat`'s userspace buffer must fill before the kernel returns data.
- `dd bs=8 count=1 if=dev/buttons/event` returns a single event immediately as expected.
- `echo flush > dev/buttons/ctl` / `> dev/buttons/event` clears the queue.

### Root cause: this kernel's `p9_client_read` is non-standard

We confirmed via serial logs that the firmware delivers each event as soon as it occurs (single `Rread` of 7 or 5 bytes per transition). The batching is entirely on the client side. We tried two spec-compliant ways to terminate `p9_client_read`'s fill loop after each event and both failed on this kernel:

1. **`Rread(0 bytes)` follow-up** — kernel did **not** treat as EOF/end-of-loop; it immediately reissued a `Tread` at the same offset, ignoring the zero-length reply.
2. **`Rerror` follow-up** — kernel **propagated the error to `cat`** even though bytes were already accumulated (`read()` returned `-EAGAIN`, no data delivered). Result: `cat: read error: Resource temporarily unavailable`.

The only thing that terminates the loop is filling the whole `Tread` count buffer with payload. Current firmware therefore **zero-pads each event to fill the full negotiated `max_count` (≈4072 bytes)**, which satisfies the kernel's fill loop in a single response. `cat`'s internal `read()` buffer is ~65536 bytes, so ~16 padded responses are accumulated before `cat` calls `write()` to stdout — hence the batching.

### Implementation notes

| Layer | Location |
|-------|----------|
| GPIO poll + edge detect | `firmware/src/dev/plus2.rs`, `firmware/src/dev/sticks3.rs` — `buttons_task`, 25 ms period |
| Queue + `try_read_event` | `devices/src/buttons.rs` |
| 9P tree | `ninep/src/fs.rs` — `event`, `ctl` (`flush`) |
| Blocking read (`WaitStream` + zero-padded response) | `ninep/src/server.rs` — `handle_read` / pend-drain for `DevBtnEvent` |
| `Tclunk` cancels stale `WaitStream` (was leaking) | `ninep/src/server.rs::handle_clunk` |

### Side fixes made while investigating

- **`Tflush` (type `108`) was decoded as type `102`** — every `^C` on `cat` produced `unknown typ=108` and `Rerror` reply, leaving v9fs's request table corrupt. Fixed: `TFLUSH = 108`, `RFLUSH = 109`, proper handler pairs `Rflush` and clears matching `pending_stream`.
- **`read_until_deadline` ignored the timeout** — replaced with `embassy_futures::select` so the server actually polls the event queue while blocked on a read.
- **`PendingStreamRead.fid`** added so `handle_clunk` can cancel a `WaitStream` belonging to a closed fid; stale pend-drains used to fire against new sessions.

### Workarounds for single-event delivery

The batching only affects clients that read with large buffers (`cat`, most stdio tools). Tools that issue small reads see one event at a time:

```sh
# one event per dd invocation
dd bs=12 count=1 if=/mnt/stick/dev/buttons/event 2>/dev/null

# streaming with shell loop
while dd bs=12 count=1 if=/mnt/stick/dev/buttons/event 2>/dev/null; do :; done
```

For programs you control, `read(fd, buf, 12)` will return one event per call.

### Possible future fixes

1. **Different v9fs mount option / protocol** — try `9p2000.L` (`-o version=9p2000.L`) which uses a different read path that may honor short reads. Would require Tlread/Rlread support in the server.
2. **`cache=fscache` or similar** — already tried `cache=none`; no effect on the fill-loop behavior.
3. **Per-fid record framing** — `length()` already returns `u32::MAX` for streaming files; nothing else in the read path obviously gates the loop.
4. **Custom client** — a small `bevent` reader binary on the device side that does small reads and prints lines (analogous to `acme`'s event reader on Plan 9).

---

## `/dev/display/brightness` — no usable dimming on Plus2

**Status:** Open (deferred until StickS3 hardware available)  
**Board:** M5StickC Plus2 only tested so far

### Symptoms

- `echo <N> > dev/display/brightness` (0–255) accepts writes and `cat dev/display/brightness` reflects the value.
- **Perceived backlight is effectively on/off** — no smooth dimming across the range; only a narrow band near full brightness appears to change anything.
- May be hardware-limited on this panel/backlight driver (GPIO27 + LEDC), not only a software curve issue.

### Implementation notes

| Layer | Location |
|-------|----------|
| LEDC (Timer1 / Ch1, GPIO27, 40 kHz, 10-bit duty) | `firmware/src/dev/plus2.rs` — `apply_backlight`, `brightness_to_duty_pct` |
| Stored level 0–255 | `devices/src/display.rs` — `handle_brightness` |
| 9P node | `dev/display/brightness` |

Several duty mappings were tried (linear, gamma, remap 1–255 → raw duty ~860–1023); behavior stayed ~binary.

### Plan

- **Do not spend more time on Plus2 PWM** for now.
- Re-test brightness when **M5Stick S3** hardware is available — S3 may use **M5PM1** for backlight (`DESIGN.md`: `dev/display/backlight` / L3B rail) rather than the Plus2 GPIO27 LEDC path; dimming might work there or need a different driver altogether.
- If S3 also fails, document as a product limitation; if S3 works, keep Plus2-specific mapping or expose dim/full only on Plus2 `info`.

---

## `/dev/mic/pcm` — no capture on Plus2 (PDM)

**Status:** Open (deferred)  
**Board:** M5StickC Plus2 (SPM1423 PDM — GPIO0 CLK/WS, GPIO34 DATA)

### Symptoms

- `echo start > dev/mic/ctl` succeeds; `cat dev/mic/ctl` shows `running=1 rate=44100 …`.
- **`queued=` stays 0** while running (no PCM ever reaches the ring buffer).
- `dd if=dev/mic/pcm …` blocks indefinitely; Ctrl+C leaves `running=1`, file empty.
- Earlier builds: stack overflow on 9P connect (fixed — static `SessionStorage`); slice panic on `bs=8192` (fixed — cap `msize` to 4096).

Example:

```text
echo start > /mnt/stick/dev/mic/ctl
cat /mnt/stick/dev/mic/ctl
# running=1 rate=44100 queued=0 fmt=s16le
dd if=/mnt/stick/dev/mic/pcm of=clip.raw bs=4096 count=50
# hangs; queued= still 0
```

### Expected behavior (DESIGN.md / README)

- After `start`, I2S/PDM DMA fills the ring; `queued` increases; blocking reads on `pcm` return s16le mono @ 44100 Hz.

### Implementation notes

| Layer | Location |
|-------|----------|
| PDM + I2S DMA task | `firmware/src/dev/mic.rs` — WS on GPIO0 (TX), DIN on GPIO34 (RX), `pdm_conf` register poke, async circular DMA |
| Ring + ctl | `devices/src/mic.rs` |
| 9P blocking read | `ninep/src/server.rs` — `WaitStream` / `MicPcm` while `running` and queue empty |

`/dev/spk` omitted on Plus2 (buzzer only). StickS3 ES8311 mic path deferred.

### Things to verify when revisiting

1. **DMA actually produces bytes** — log `got` from `rx_xfer.pop` in `mic_task` (may be 0 forever today).
2. **PDM + esp-hal** — ESP32 has no first-class PDM API in esp-hal 1.1; compare M5 Arduino example / ESP-IDF `i2s_pdm` pin and clock setup.
3. **Clock path** — M5 uses GPIO0 as I2S WS; tried `with_mclk`/`CLK_OUT1` (wrong) then TX `with_ws` + silence DMA; still `queued=0`.
4. **Mic vs buzzer** — M5 docs: mic and speaker amp should not run together; avoid boot chime during capture tests.
5. **Sample rate** — hardware fixed at 44100 Hz on ESP32; `ctl rate` is stored intent only.

### Workarounds

- None on device today. Stage 3 mic remains **tree + ctl API only** until PDM capture works.

---

## `/dev/led` — no software control on StickS3

**Status:** Open (deferred)  
**Board:** M5Stack StickS3 only

### Symptoms

- `echo on > /dev/led/ctl`, `echo blink … > /dev/led/ctl`, and `echo off > /dev/led/ctl` all succeed at the 9P layer; `cat /dev/led/state` reflects the requested state.
- The physical green LED on the device **does not respond** to those writes.
- Observed behaviour: M5PM1 firmware drives the LED itself — it blinks briefly during boot/charging cycles and then sits solid on while the device is running normally.

### Investigation summary

Confirmed by reading back the M5PM1 register file after our init sequence on HW rev 5 / SW rev 0x4f (matching DESIGN.md notes):

| Register | Final value | Meaning |
|----------|-------------|---------|
| `DEVICE_ID` (0x00) | `0x50` | Correct chip |
| `GPIO_MODE` (0x10) | `0x01` | GPIO0 = output |
| `GPIO_OUT` (0x11) | `0x01` | GPIO0 high |
| `GPIO_DRV` (0x13) | `0x1e` | LED_EN_DRV bit 5 = 0 (push-pull) |
| `GPIO_FUNC0` (0x16) | `0x03` | GPIO0 = OTHER (LED_EN / NeoPixel) |
| `PWR_CFG` (0x06) | `0x17` | LED_CTRL bit 4 = 1 (rail on) |
| `NEO_CFG` (0x50) | `0x01` | LED count = 1 |

A boot-time probe (`firmware/src/dev/sticks3.rs::probe_led_paths`, since removed) tried three control paths back-to-back — NeoPixel write+refresh, plain GPIO_OUT toggle on GPIO0, and `PWR_CFG` bit 4 toggle. None of them visibly overrode the M5PM1's built-in status pattern; the LED behaved like an autonomous power indicator.

### Hypothesis

`DESIGN.md` line 45 describes the LED as *"firmware-controlled flash patterns (500 ms in download mode, etc.) — driven by PWR_CFG bit 4"*. We now believe **the M5PM1 internal firmware owns the LED** for status indication, and the user-facing `LED_CTRL` / NeoPixel registers are intended for **other M5Stack products** (e.g. StampS3Bat's RGB LED) that share the same PMIC. The official M5PM1 NeoPixel example notes that `setLedEnLevel()` is *"mainly for the Stamp-S3Bat product"*.

### Things to verify when revisiting

1. **Check schematic v0.6 (2025-11-11)** for the actual LED wiring — is the cathode tied to a separate M5PM1 pin we haven't tried (e.g. PYG1), or routed through the AW8737 amplifier rail?
2. **Read `IRQ_STATUS3` (0x42)** to see if M5PM1 button/status events keep retriggering the LED pattern; if so, masking via `IRQ_MASK3` (0x45) might quiet the chip.
3. **Try M5PM1 sleep/wake cycle** — `setLedEnLevel` may only take effect after a `shutdown`/`sleep` round-trip.
4. **Compare with a fresh M5Unified-based Arduino sketch** running on the same StickS3; if M5Unified can blink the LED, instrument the I²C bus to capture the exact register sequence.

### Workarounds

- None today. The 9P `/dev/led/*` surface remains in place so the **schema stays uniform across boards**, but writes are no-ops on StickS3. `/dev/led/state` still reflects the requested state for clients that care.

---
